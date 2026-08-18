//! A single file tile server reading from pre-computed SQLite DBs.
//
// Possible follow-ups:
// - Stream get_all_diffs instead of buffering: create a tokio::sync::mpsc channel, blocking_send each chunk from the blocking task, and return Body::from_stream(ReceiverStream::new(rx)).
// - Serve the tile bodies as Bytes read directly from stmt.query_row(|r| r.get_ref(0)) to avoid one copy.

use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use axum::{
    body::Bytes,
    extract::{Path as AxumPath, Query, Request, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use chrono::{DateTime, NaiveDateTime, TimeDelta, TimeZone, Utc};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, OpenFlags, OptionalExtension};
use serde::Deserialize;
use tower_http::timeout::TimeoutLayer;
use tracing::{error, info, warn};
use scheduled_thread_pool::ScheduledThreadPool;

mod i18n;
use i18n::{html_escape, Lang};

// ---------------------------------------------------------------------------
// Database manager
// ---------------------------------------------------------------------------

const TILE_QUERY: &str = "SELECT data FROM tiles WHERE z = ?1 AND x = ?2 AND y = ?3";

type SqlitePool = r2d2::Pool<SqliteConnectionManager>;

#[derive(Debug, thiserror::Error)]
enum TileError {
    #[error("requested version {0} not found")]
    VersionNotFound(u32),
    #[error("tile not found")]
    TileNotFound,
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("pool error: {0}")]
    Pool(#[from] r2d2::Error),
}

#[derive(Default)]
struct DatabaseManager {
    /// version (week number) -> pool of read-only connections.
    pools: HashMap<u32, SqlitePool>,
}

impl DatabaseManager {
    fn new() -> Self {
        Self::default()
    }

    /// Scans for database files and initializes connections.
    fn initialize_week_databases(&mut self, folder: &Path) -> Result<()> {
        let entries = fs::read_dir(folder)
            .with_context(|| format!("failed to read directory {}", folder.display()))?;

        let r2d2_threads = Arc::new(ScheduledThreadPool::new(2));

        let mut db_count = 0usize;
        for entry in entries {
            let entry = entry.context("failed to read directory entry")?;
            if entry.file_type()?.is_dir() {
                continue;
            }

            let filename = entry.file_name().to_string_lossy().into_owned();

            // Expected shape: w<version>_<anything>.db
            let Some(stem) = filename
                .strip_suffix(".db")
                .and_then(|s| s.strip_prefix('w'))
            else {
                continue;
            };

            let mut parts = stem.split('_');
            let version_str = parts.next().unwrap_or_default();
            if parts.next().is_none() {
                // invalid filename (no "_" separator)
                continue;
            }
            let Ok(version) = version_str.parse::<u32>() else {
                warn!("Invalid week database filename: {filename}");
                continue;
            };

            let path = folder.join(&filename);
            info!("Initializing database: {} (version {version})", path.display());

            let manager = SqliteConnectionManager::file(&path)
                .with_flags(
                    OpenFlags::SQLITE_OPEN_READ_ONLY
                        | OpenFlags::SQLITE_OPEN_URI
                        | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )
                .with_init(|c| c.execute_batch("PRAGMA query_only = ON; PRAGMA temp_store = MEMORY;"));

            let pool = r2d2::Pool::builder()
                .max_size(3)
                .min_idle(Some(0))
                .idle_timeout(Some(Duration::from_secs(60)))
                .max_lifetime(Some(Duration::from_secs(24 * 60 * 60)))
                .connection_timeout(Duration::from_secs(5))
                .test_on_check_out(false) // read-only files, no need for "SELECT 1"
                .thread_pool(r2d2_threads.clone())
                .build(manager)
                .with_context(|| format!("failed to open database {}", path.display()))?;

            self.pools.insert(version, pool);
            db_count += 1;
        }

        if db_count == 0 {
            return Err(anyhow!(
                "no week database files found (looking for w*.db files)"
            ));
        }

        info!("Initialized {db_count} database(s)");
        Ok(())
    }

    fn initialize_databases(&mut self, folder_path: &Path) -> Result<()> {
        self.initialize_week_databases(&folder_path.join("weeks"))
    }

    fn get_tile(&self, z: i64, x: i64, y: i64, version: u32) -> Result<Vec<u8>, TileError> {
        let pool = self
            .pools
            .get(&version)
            .ok_or(TileError::VersionNotFound(version))?;

        let conn = pool.get()?;
        let mut stmt = conn.prepare_cached(TILE_QUERY)?;
        let data: Option<Vec<u8>> = stmt
            .query_row(params![z, x, y], |row| row.get(0))
            .optional()?;

        data.ok_or(TileError::TileNotFound)
    }

    fn get_date_list(&self) -> Vec<u32> {
        let mut dates: Vec<u32> = Vec::new();

        for (version, pool) in &self.pools {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(e) => {
                    error!("Error getting connection for database {version}: {e}");
                    continue;
                }
            };
            let mut stmt = match conn.prepare("SELECT date FROM versions") {
                Ok(s) => s,
                Err(e) => {
                    error!("Error querying version for database {version}: {e}");
                    continue;
                }
            };
            let rows = match stmt.query_map([], |row| row.get::<_, u32>(0)) {
                Ok(r) => r,
                Err(e) => {
                    error!("Error querying version for database {version}: {e}");
                    continue;
                }
            };
            for row in rows {
                match row {
                    Ok(date) => dates.push(date),
                    Err(e) => error!("Error scanning version for database {version}: {e}"),
                }
            }
        }

        dates.sort_unstable();
        dates
    }

    /// Retrieves all diffs for a given tile across all versions in [from, to].
    /// (The Go version streamed into the `http.ResponseWriter`; here we build the
    /// body in memory, which is simpler and fast enough for tile-sized blobs.)
    fn get_all_diffs(&self, z: i64, x: i64, y: i64, from: u32, to: u32) -> Vec<u8> {
        let from_week = from / (24 * 7);
        let to_week = to / (24 * 7);

        // version is already in week unit
        let mut versions: Vec<u32> = self
            .pools
            .keys()
            .copied()
            .filter(|v| *v >= from_week && *v <= to_week)
            .collect();
        versions.sort_unstable();

        let mut out: Vec<u8> = Vec::new();
        let mut is_first = true;

        for version in versions {
            let pool = &self.pools[&version];

            let diff_data: Vec<u8> = match pool
                .get()
                .map_err(TileError::from)
                .and_then(|conn| {
                    let mut stmt = conn.prepare_cached(TILE_QUERY)?;
                    stmt.query_row(params![z, x, y], |row| row.get::<_, Vec<u8>>(0))
                        .optional()?
                        .ok_or(TileError::TileNotFound)
                }) {
                Ok(d) => d,
                Err(e) => {
                    error!("Error querying diff for version {version}: {e}");
                    continue;
                }
            };

            // The `dh == 0` full base of the first-week blob is kept as-is so the
            // concatenated stream always starts from a clean full frame. For any
            // other week, the base is only redundant when the week was seeded from
            // the immediately-previous week's tail. When a week was snapshotted via
            // makebase (fresh state) or an earlier week is missing, skipping it
            // severs the diff chain and the APNG accumulates onto a stale canvas.
            // Keep it, renamed to the week boundary so it lands in the date-ordered
            // stream as a unique full-frame reset.
            if is_first {
                out.extend_from_slice(&diff_data);
            } else {
                let mut body = diff_data;
                let boundary = version * (24 * 7);
                self.reseat_week_base(&mut body, boundary);
                out.extend_from_slice(&body);
            }
            is_first = false;
        }

        out
    }

    /// If `data` starts with a `dh == 0` full base, rewrite its date to the week
    /// `boundary` instead of dropping it (see `get_all_diffs`). Any later entry in
    /// the same blob already at `boundary` encodes the same state as the base and
    /// is removed, so the rewritten base remains the single boundary frame.
    fn reseat_week_base(&self, data: &mut Vec<u8>, boundary: u32) {
        if boundary == 0 || data.len() < 8 {
            return;
        }
        let first_date = u32::from_le_bytes(data[0..4].try_into().unwrap());
        if first_date != 0 {
            return;
        }

        let mut offset = 0usize;
        let mut first_entry = true;
        let mut out = Vec::with_capacity(data.len());
        while offset + 8 <= data.len() {
            let date = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
            let length = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap()) as usize;
            let entry_end = offset + 8 + length;
            if entry_end > data.len() {
                break; // tolerate trailing garbage, mirroring from_bytes
            }
            if first_entry {
                first_entry = false;
                out.extend_from_slice(&boundary.to_le_bytes());
                out.extend_from_slice(&data[offset + 4..entry_end]);
            } else if date == boundary {
                // Redundant diff colliding with the renamed base: drop it.
            } else {
                out.extend_from_slice(&data[offset..entry_end]);
            }
            offset = entry_end;
        }

        *data = out;
    }
}

// Note: there is no explicit `Close()`. Dropping the `DatabaseManager` drops the
// pools, which closes every SQLite connection (and its cached statements).

// ---------------------------------------------------------------------------
// Date helpers
// ---------------------------------------------------------------------------

fn wplace_epoch() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap()
}

#[allow(dead_code)]
fn date_to_epoch_hour(date: &str) -> Result<u32> {
    // chrono needs minutes to build a NaiveDateTime, so append ":00".
    let naive = NaiveDateTime::parse_from_str(&format!("{date}:00"), "%Y-%m-%dT%H:%M")
        .with_context(|| format!("invalid date: {date}"))?;
    let t = Utc.from_utc_datetime(&naive);
    Ok((t - wplace_epoch()).num_hours() as u32)
}

fn epoch_hour_to_date(epoch_hour: u32) -> String {
    (wplace_epoch() + TimeDelta::hours(epoch_hour as i64))
        .format("%Y-%m-%dT%H")
        .to_string()
}

/// Convert a version float to a date string (e.g. 1.001 -> 2025-01-08T01):
/// the integral part is weeks since the epoch, the fraction is hours*1e-3.
#[allow(dead_code)]
fn date_from_version(version: f32) -> String {
    let major = version.trunc() as i64;
    let minor = ((version - major as f32) * 1000.0) as i64;
    (wplace_epoch() + TimeDelta::days(major * 7) + TimeDelta::hours(minor))
        .format("%Y-%m-%dT%H")
        .to_string()
}

// ---------------------------------------------------------------------------
// Tile server state
// ---------------------------------------------------------------------------

struct Asset {
    #[allow(dead_code)]
    name: String,
    data: Bytes,
    mime: &'static str,
}

struct TileServer {
    #[allow(dead_code)]
    data_path: PathBuf,
    db: DatabaseManager,
    index_html: BTreeMap<Lang, Bytes>,
    #[allow(dead_code)]
    latest_version: String,
    preview_image: Bytes,
    favicon: Bytes,
    assets: HashMap<String, Asset>,
}

impl TileServer {
    fn new(data_path: PathBuf) -> Result<Self> {
        let mut db = DatabaseManager::new();
        db.initialize_databases(&data_path)?;

        let dates = db.get_date_list();
        let (index_html, latest_version) = build_index(&data_path, &dates)?;
        let index_html: BTreeMap<Lang, Bytes> = index_html
            .into_iter()
            .map(|(k, v)| (k, Bytes::from(v)))
            .collect();

        let preview_image = match make_latest_image() {
            Ok(d) => d,
            Err(e) => {
                warn!("Warning: failed to create preview image: {e}");
                Bytes::new()
            }
        };

        let favicon = match fs::read(data_path.join("favicon.ico")) {
            Ok(d) => Bytes::from(d),
            Err(e) => {
                warn!("Warning: failed to load favicon: {e}");
                Bytes::new()
            }
        };

        let assets = load_assets(&data_path)?;

        Ok(Self {
            data_path,
            db,
            index_html,
            latest_version,
            preview_image,
            favicon,
            assets,
        })
    }
}

fn make_latest_image() -> Result<Bytes> {
    Err(anyhow!("TODO"))
}

/// Base URL used for canonical links, og:url and hreflang alternates.
const SITE_BASE: &str = "https://wplace.eralyon.net";

/// Renders the full index page for one language from the template.
fn render_index(
    tmpl: &str,
    lang: Lang,
    dict: &BTreeMap<String, String>,
    version_options: &str,
) -> Result<String> {
    let mut content = tmpl.to_string();

    content = content.replace("//$$VERSION_OPTIONS$$", version_options);
    content = content.replace("{{LANG_PATH}}", lang.path());

    let mut hrefs = String::new();
    for l in Lang::ALL {
        hrefs.push_str(&format!(
            "    <link rel=\"alternate\" hreflang=\"{}\" href=\"{SITE_BASE}/{}/\">\n",
            l.code(),
            l.path()
        ));
    }
    hrefs.push_str(&format!(
        "    <link rel=\"alternate\" hreflang=\"x-default\" href=\"{SITE_BASE}/{}/\">\n",
        Lang::En.path()
    ));
    content = content.replace("{{HREFLANG_LINKS}}", &hrefs);

    let mut switcher = String::from("<nav class=\"lang-switcher\">\n");
    for l in Lang::ALL {
        let cls = if l == lang { " class=\"lang-active\"" } else { "" };
        switcher.push_str(&format!(
            "  <a href=\"/{}/\"{cls}>{}</a>\n",
            l.path(),
            l.label()
        ));
    }
    switcher.push_str("</nav>");
    content = content.replace("{{LANG_SWITCHER}}", &switcher);

    while let Some(start) = content.find("{{t:") {
        let after = &content[start + 4..];
        let end = after
            .find("}}")
            .ok_or_else(|| anyhow!("unterminated i18n placeholder at byte {start}"))?;
        let key = &after[..end];
        let value = dict
            .get(key)
            .ok_or_else(|| anyhow!("unknown i18n key '{{{{t:{key}}}}}'"))?;
        content.replace_range(start..start + 4 + end + 2, &html_escape(value));
    }

    let dict_json = serde_json::to_string(dict).context("serialize i18n dict")?;
    content = content.replace("//$$I18N_DICT$$", &format!("window.I18N = {dict_json};"));

    for marker in [
        "{{t:",
        "{{LANG_PATH}}",
        "{{HREFLANG_LINKS}}",
        "{{LANG_SWITCHER}}",
        "//$$I18N_DICT$$",
        "//$$VERSION_OPTIONS$$",
    ] {
        if content.contains(marker) {
            return Err(anyhow!("leftover template marker {marker} in rendered page"));
        }
    }

    Ok(content)
}

/// Loads index.html.tmpl and renders one copy per language.
fn build_index(data_path: &Path, dates: &[u32]) -> Result<(BTreeMap<Lang, String>, String)> {
    let last = *dates
        .last()
        .ok_or_else(|| anyhow!("no versions found in the databases"))?;
    let latest_version = format!("{:.3}", last as f64);

    let tmpl_path = data_path.join("index.html.tmpl");
    let tmpl = fs::read_to_string(&tmpl_path)
        .with_context(|| format!("failed to read {}", tmpl_path.display()))?;

    let versions: Vec<String> = dates
        .iter()
        .map(|&epoch_hour| {
            format!(
                "{{version: '{epoch_hour}', date: '{}'}}",
                epoch_hour_to_date(epoch_hour)
            )
        })
        .collect();
    let version_options = versions.join(",");

    let translations = i18n::load_translations(data_path)?;
    let mut pages = BTreeMap::new();
    for lang in Lang::ALL {
        let page = render_index(&tmpl, lang, &translations[&lang], &version_options)?;
        pages.insert(lang, page);
    }
    Ok((pages, latest_version))
}

/// Loads static assets from the assets directory.
fn load_assets(data_path: &Path) -> Result<HashMap<String, Asset>> {
    let folder = data_path.join("assets");
    let entries = fs::read_dir(&folder)
        .with_context(|| format!("failed to read assets directory {}", folder.display()))?;

    let mut assets = HashMap::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().into_owned();
        let data = fs::read(entry.path())
            .with_context(|| format!("failed to read asset {filename}"))?;
        info!("Loaded asset: {filename} ({} bytes)", data.len());
        assets.insert(
            filename.clone(),
            Asset {
                mime: mime_type(&filename),
                name: filename,
                data: Bytes::from(data),
            },
        );
    }

    Ok(assets)
}

fn mime_type(filename: &str) -> &'static str {
    let ext = Path::new(filename)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "js" => "application/javascript",
        "css" => "text/css",
        "html" => "text/html",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

// ---------------------------------------------------------------------------
// Coordinate helpers
// ---------------------------------------------------------------------------

fn tile_key(z: i64, x: i64, y: i64) -> String {
    format!("{z}/{x}/{y}")
}

fn parse_tile_coords(z_str: &str, x_str: &str, y_str: &str) -> Result<(i64, i64, i64), &'static str> {
    let z: i64 = z_str.parse().map_err(|_| "invalid z coordinate")?;
    let x: i64 = x_str.parse().map_err(|_| "invalid x coordinate")?;
    let y: i64 = y_str.parse().map_err(|_| "invalid y coordinate")?;

    // Validate coordinates (basic sanity check)
    if !(0..=11).contains(&z) || x < 0 || y < 0 || x >= (1 << z) || y >= (1 << z) {
        return Err("tile coordinate out of bound");
    }
    Ok((z, x, y))
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

/// Mirrors Go's `http.Error`: text/plain body with a trailing newline.
fn text_error(status: StatusCode, msg: &str) -> Response {
    (
        status,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )],
        format!("{msg}\n"),
    )
        .into_response()
}

fn etag_header(etag: &str) -> (HeaderName, HeaderValue) {
    (
        header::ETAG,
        HeaderValue::from_str(etag).unwrap_or(HeaderValue::from_static("\"\"")),
    )
}

fn if_none_match(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(|v| v == etag)
        .unwrap_or(false)
}

/// GET /tiles/{version}/{z}/{x}/{y}.zst
async fn serve_tile(
    State(ts): State<Arc<TileServer>>,
    AxumPath((version_str, z_str, x_str, y_str)): AxumPath<(String, String, String, String)>,
    headers: HeaderMap,
) -> Response {
    // matchit params always cover a full segment, so strip the extension here.
    let Some(y_str) = y_str.strip_suffix(".zst") else {
        return text_error(StatusCode::NOT_FOUND, "404 page not found");
    };

    let (z, x, y) = match parse_tile_coords(&z_str, &x_str, y_str) {
        Ok(v) => v,
        Err(e) => return text_error(StatusCode::BAD_REQUEST, e),
    };

    let version = match version_str.parse::<f32>() {
        Ok(v) if v.is_finite() && v >= 0.0 => v as u32,
        _ => return text_error(StatusCode::BAD_REQUEST, "invalid version"),
    };

    let etag = format!("\"{}-{}\"", version, tile_key(z, x, y));
    let out_headers = [
        (
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        ),
        (
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=86400"), // Cache for 1 day
        ),
        etag_header(&etag),
    ];

    if if_none_match(&headers, &etag) {
        return (StatusCode::NOT_MODIFIED, out_headers).into_response();
    }

    let state = ts.clone();
    let result = tokio::task::spawn_blocking(move || state.db.get_tile(z, x, y, version)).await;

    match result {
        Ok(Ok(data)) => (StatusCode::OK, out_headers, data).into_response(),
        Ok(Err(TileError::TileNotFound)) => text_error(StatusCode::NOT_FOUND, "tile not found"),
        Ok(Err(TileError::VersionNotFound(v))) => {
            text_error(StatusCode::NOT_FOUND, &format!("version {v} not found"))
        }
        Ok(Err(e)) => {
            error!("Database query error: {e}");
            text_error(StatusCode::INTERNAL_SERVER_ERROR, "Database error")
        }
        Err(e) => {
            error!("Blocking task failed: {e}");
            text_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct DiffQuery {
    from: Option<String>,
    to: Option<String>,
}

/// GET /diff/all/{z}/{x}/{y}.zst?from=&to=   (any stored z (0..9, 11) is supported; z=10 has no stored tiles)
async fn serve_all_diff(
    State(ts): State<Arc<TileServer>>,
    AxumPath((z_str, x_str, y_str)): AxumPath<(String, String, String)>,
    Query(q): Query<DiffQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(y_str) = y_str.strip_suffix(".zst") else {
        return text_error(StatusCode::NOT_FOUND, "404 page not found");
    };

    let (z, x, y) = match parse_tile_coords(&z_str, &x_str, y_str) {
        Ok(v) => v,
        Err(e) => return text_error(StatusCode::BAD_REQUEST, e),
    };

    let from_str = q.from.as_deref().unwrap_or("").trim().to_owned();
    let to_str = q.to.as_deref().unwrap_or("").trim().to_owned();

    let from: u32 = if from_str.is_empty() {
        0
    } else {
        match from_str.parse() {
            Ok(v) => v,
            Err(_) => return text_error(StatusCode::BAD_REQUEST, "invalid from date"),
        }
    };
    let to: u32 = if to_str.is_empty() {
        u32::MAX
    } else {
        match to_str.parse() {
            Ok(v) => v,
            Err(_) => return text_error(StatusCode::BAD_REQUEST, "invalid to date"),
        }
    };

    info!("Serving diff for tile {z} {x} {y} from {from} to {to}");

    let etag = format!("\"alldiff-{}-from{}-to{}\"", tile_key(z, x, y), from, to);
    let out_headers = [
        (
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        ),
        (
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=3600"), // Cache for 1 hour
        ),
        etag_header(&etag),
    ];

    if if_none_match(&headers, &etag) {
        return (StatusCode::NOT_MODIFIED, out_headers).into_response();
    }

    let state = ts.clone();
    match tokio::task::spawn_blocking(move || state.db.get_all_diffs(z, x, y, from, to)).await {
        // An empty diff means either the tile does not exist at this z (z=10 is
        // intentionally not stored, and /diff/all is open to any z) or no frame in
        // the requested range changed this tile; both are "nothing to render".
        Ok(body) if body.is_empty() => text_error(StatusCode::NOT_FOUND, "diff not found"),
        Ok(body) => (StatusCode::OK, out_headers, body).into_response(),
        Err(e) => {
            error!("Blocking task failed: {e}");
            text_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        }
    }
}

async fn serve_index_en(State(ts): State<Arc<TileServer>>) -> Response {
    serve_index_lang(ts, Lang::En)
}

async fn serve_index_ja(State(ts): State<Arc<TileServer>>) -> Response {
    serve_index_lang(ts, Lang::Ja)
}

async fn serve_index_es(State(ts): State<Arc<TileServer>>) -> Response {
    serve_index_lang(ts, Lang::Es)
}

fn serve_index_lang(ts: Arc<TileServer>, lang: Lang) -> Response {
    match ts.index_html.get(&lang) {
        Some(html) => Html(html.clone()).into_response(),
        None => text_error(StatusCode::NOT_FOUND, "404 page not found"),
    }
}

async fn serve_index_redirect(headers: HeaderMap) -> Response {
    let accept = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();
    let lang = Lang::from_accept_language(&accept);
    let mut resp = redirect_found(&format!("/{}/", lang.path()));
    resp.headers_mut()
        .insert(header::VARY, HeaderValue::from_static("accept-language"));
    resp
}

async fn serve_lang_redirect(AxumPath(lang): AxumPath<String>) -> Response {
    match Lang::from_path(&lang) {
        Some(l) => redirect_found(&format!("/{}/", l.path())),
        None => text_error(StatusCode::NOT_FOUND, "404 page not found"),
    }
}

/// 302 Found: temporary redirect with an empty body.
fn redirect_found(path: &str) -> Response {
    (
        StatusCode::FOUND,
        [(header::LOCATION, HeaderValue::from_str(path).unwrap())],
        (),
    )
        .into_response()
}

async fn serve_preview(State(ts): State<Arc<TileServer>>) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, HeaderValue::from_static("image/png"))],
        ts.preview_image.clone(),
    )
        .into_response()
}

async fn serve_favicon(State(ts): State<Arc<TileServer>>) -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("image/x-icon"),
        )],
        ts.favicon.clone(),
    )
        .into_response()
}

async fn serve_asset(
    State(ts): State<Arc<TileServer>>,
    AxumPath(filename): AxumPath<String>,
) -> Response {
    match ts.assets.get(&filename) {
        Some(asset) => (
            StatusCode::OK,
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static(asset.mime),
            )],
            asset.data.clone(),
        )
            .into_response(),
        None => text_error(StatusCode::NOT_FOUND, "404 page not found"),
    }
}

/// Logs HTTP requests (equivalent of the Go `loggingMiddleware`).
async fn logging_middleware(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let start = Instant::now();

    let response = next.run(req).await;

    info!(
        "{} {} {} {:?}",
        method,
        path,
        response.status().as_u16(),
        start.elapsed()
    );
    response
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn build_router(tile_server: Arc<TileServer>) -> Router {
    Router::new()
        .route("/tiles/{version}/{z}/{x}/{y}", get(serve_tile))
        .route("/diff/all/{z}/{x}/{y}", get(serve_all_diff))
        .route("/", get(serve_index_redirect))
        .route("/en/", get(serve_index_en))
        .route("/ja/", get(serve_index_ja))
        .route("/es/", get(serve_index_es))
        .route("/{lang}", get(serve_lang_redirect))
        .route("/preview.png", get(serve_preview))
        .route("/favicon.ico", get(serve_favicon))
        .route("/assets/{filename}", get(serve_asset))
        .layer(middleware::from_fn(logging_middleware))
        .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(15))) // ~ Read/WriteTimeout
        .with_state(tile_server)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let port = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let data_path = env::var("DATA_PATH").unwrap_or_else(|_| ".".to_string());

    let tile_server = match TileServer::new(PathBuf::from(data_path)) {
        Ok(ts) => Arc::new(ts),
        Err(e) => {
            error!("Failed to create tile server: {e:#}");
            std::process::exit(1);
        }
    };

    let app = build_router(tile_server);

    let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("Starting tile server on {addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    info!("Shutting down");
    // Dropping the Arc<TileServer> closes every pooled SQLite connection,
    // which is the equivalent of Go's `defer tileServer.Close()`.
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    #[test]
    fn real_translation_files_have_identical_key_sets() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let maps = i18n::load_translations(dir).expect("i18n files parse and share keys");
        assert_eq!(maps.len(), 3);
        for lang in Lang::ALL {
            assert!(
                (40..=100).contains(&maps[&lang].len()),
                "{} has {} keys",
                lang.code(),
                maps[&lang].len()
            );
        }
    }

    #[test]
    fn real_template_renders_all_languages_without_leftovers() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let tmpl = std::fs::read_to_string(repo.join("frontend").join("index.html")).unwrap();
        let dicts = i18n::load_translations(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let options = "{version: '0', date: '2025-01-01T00'}";
        for lang in Lang::ALL {
            let page = render_index(&tmpl, lang, &dicts[&lang], options).unwrap();
            for marker in [
                "{{t:",
                "{{LANG_PATH}}",
                "{{HREFLANG_LINKS}}",
                "{{LANG_SWITCHER}}",
                "//$$I18N_DICT$$",
                "//$$VERSION_OPTIONS$$",
            ] {
                assert!(!page.contains(marker), "{:?} still contains {marker}", lang);
            }
            assert!(
                page.contains(&format!("<html lang=\"{}\">", lang.code())),
                "{:?}",
                lang
            );
            assert!(page.contains("window.I18N = {"), "{:?}\n{page}", lang);
        }
    }

    /// One TileHistory frame entry: [u32 LE date][u32 LE block_size][block_size payload bytes].
    fn entry(date: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&date.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn create_week_db(dir: &std::path::Path, week: u32, z: i64, x: i64, y: i64, data: Vec<u8>) {
        std::fs::create_dir_all(dir).unwrap();
        let conn = rusqlite::Connection::open(dir.join(format!("w{week}_0.db"))).unwrap();
        conn.execute_batch(
            "CREATE TABLE tiles (z INTEGER NOT NULL, x INTEGER NOT NULL, y INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (z, x, y));
             CREATE TABLE versions (date INTEGER PRIMARY KEY, original_file TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tiles (z, x, y, data) VALUES (?1, ?2, ?3, ?4)",
            params![z, x, y, data],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO versions (date, original_file) VALUES (0, 'test')",
            [],
        )
        .unwrap();
    }

    fn rich_week1_blob() -> Vec<u8> {
        // full-image header at dh==0 in this week, renamed to the week boundary (168) by concat logic
        let mut b = entry(0, &[0xCC, 0xDD]);
        b.extend(entry(7, &[0xEE])); // the actual change
        b
    }

    #[test]
    fn get_all_diffs_concatenates_lower_zoom_levels() {
        let base = entry(0, &[0xAA, 0xBB]);
        let w1 = rich_week1_blob();
        let expect = {
            let mut e = base.clone();
            e.extend(entry(168, &[0xCC, 0xDD])); // week-1 dh==0 base kept, renamed to boundary
            e.extend(entry(7, &[0xEE]));
            e
        };

        for z in [9i64, 0i64] {
            let tmp = tempfile::tempdir().unwrap();
            create_week_db(tmp.path(), 0, z, 0, 0, base.clone());
            create_week_db(tmp.path(), 1, z, 0, 0, w1.clone());

            let mut mgr = DatabaseManager::new();
            mgr.initialize_week_databases(tmp.path()).unwrap();

            assert_eq!(
                mgr.get_all_diffs(z, 0, 0, 0, u32::MAX),
                expect,
                "z={z} concatenation mismatch"
            );
        }
    }

    #[test]
    fn get_all_diffs_keeps_base_of_noncontiguous_week_at_boundary() {
        // Weeks 0 and 2 are present; week 1 is missing (a gap). The `dh==0` base
        // of week 2 is a fresh snapshot, not implied by the previous week's
        // tail, so it must be KEPT in the stream -- renamed to the week
        // boundary (2*168 = 336) -- rather than skipped.
        let mut w0 = entry(0, &[0xAA, 0xBB]);
        w0.extend(entry(5, &[0xCC]));
        let mut w2 = entry(0, &[0x11, 0x22]); // dh==0 base
        w2.extend(entry(343, &[0x33]));

        let expect = {
            let mut e = w0.clone();
            e.extend(entry(336, &[0x11, 0x22])); // renamed base kept as a full frame
            e.extend(entry(343, &[0x33]));
            e
        };

        let tmp = tempfile::tempdir().unwrap();
        create_week_db(tmp.path(), 0, 9, 0, 0, w0.clone());
        create_week_db(tmp.path(), 2, 9, 0, 0, w2.clone());

        let mut mgr = DatabaseManager::new();
        mgr.initialize_week_databases(tmp.path()).unwrap();

        assert_eq!(mgr.get_all_diffs(9, 0, 0, 0, u32::MAX), expect);
    }

    #[test]
    fn get_all_diffs_drops_diff_colliding_with_renamed_base() {
        // Unusual week: dh==0 base AND a diff stored at exactly the week boundary
        // (2*168 = 336). The renamed base must win; the colliding diff encodes the
        // same state, so it is dropped.
        let mut w0 = entry(0, &[0xAA, 0xBB]);
        w0.extend(entry(5, &[0xCC]));
        let mut w2 = entry(0, &[0x11, 0x22]); // dh==0 base
        w2.extend(entry(336, &[0x99]));       // colliding diff at boundary
        w2.extend(entry(343, &[0x33]));

        let expect = {
            let mut e = w0.clone();
            e.extend(entry(336, &[0x11, 0x22])); // renamed base kept, full frame
            e.extend(entry(343, &[0x33]));       // colliding diff dropped
            e
        };

        let tmp = tempfile::tempdir().unwrap();
        create_week_db(tmp.path(), 0, 9, 0, 0, w0.clone());
        create_week_db(tmp.path(), 2, 9, 0, 0, w2.clone());

        let mut mgr = DatabaseManager::new();
        mgr.initialize_week_databases(tmp.path()).unwrap();

        assert_eq!(mgr.get_all_diffs(9, 0, 0, 0, u32::MAX), expect);
    }

    #[tokio::test]
    async fn diff_endpoint_serves_lower_zoom_levels() {
        let base = entry(0, &[0xAA, 0xBB]);
        let w1 = rich_week1_blob();

        let tmp = tempfile::tempdir().unwrap();
        create_week_db(&tmp.path().join("weeks"), 0, 9, 0, 0, base.clone());
        create_week_db(&tmp.path().join("weeks"), 1, 9, 0, 0, w1.clone());
        std::fs::write(
            tmp.path().join("index.html.tmpl"),
            "<html><!-- //$$VERSION_OPTIONS$$ --></html>",
        )
        .unwrap();
        std::fs::create_dir(tmp.path().join("assets")).unwrap();
        write_i18n(tmp.path());

        let ts = Arc::new(TileServer::new(tmp.path().to_path_buf()).unwrap());
        let app = build_router(ts);

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/diff/all/9/0/0.zst?from=0&to=4294967295")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let mut expected = base.clone();
        expected.extend(entry(168, &[0xCC, 0xDD])); // week-1 dh==0 base kept, renamed to boundary
        expected.extend(entry(7, &[0xEE]));
        assert_eq!(&body[..], &expected[..]);
    }

    #[tokio::test]
    async fn diff_endpoint_unstored_z_is_404() {
        let tmp = tempfile::tempdir().unwrap();
        create_week_db(&tmp.path().join("weeks"), 0, 9, 0, 0, entry(0, &[0xAA]));
        std::fs::write(
            tmp.path().join("index.html.tmpl"),
            "<html><!-- //$$VERSION_OPTIONS$$ --></html>",
        )
        .unwrap();
        std::fs::create_dir(tmp.path().join("assets")).unwrap();
        write_i18n(tmp.path());

        let ts = Arc::new(TileServer::new(tmp.path().to_path_buf()).unwrap());
        let app = build_router(ts);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/diff/all/10/0/0.zst?from=0&to=4294967295")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn accept_language_picks_first_supported_tag() {
        assert_eq!(Lang::from_accept_language("en-US,en;q=0.9"), Lang::En);
        assert_eq!(Lang::from_accept_language("ja-JP,ja;q=0.9,en;q=0.8"), Lang::Ja);
        assert_eq!(Lang::from_accept_language("es-ES,es;q=0.9"), Lang::Es);
        // unsupported first tag is skipped; the first supported one wins
        assert_eq!(Lang::from_accept_language("fr-FR,ja;q=0.8,en;q=0.7"), Lang::Ja);
        // none supported -> default English; empty / wildcard also default
        assert_eq!(Lang::from_accept_language("fr-FR"), Lang::En);
        assert_eq!(Lang::from_accept_language(""), Lang::En);
        assert_eq!(Lang::from_accept_language("*"), Lang::En);
    }

    #[test]
    fn lang_code_path_label_and_path_lookup() {
        assert_eq!(Lang::En.code(), "en");
        assert_eq!(Lang::En.path(), "en");
        assert_eq!(Lang::En.label(), "English");
        assert_eq!(Lang::Ja.code(), "ja");
        assert_eq!(Lang::Ja.label(), "日本語");
        assert_eq!(Lang::Es.code(), "es");
        assert_eq!(Lang::Es.label(), "Español");
        assert_eq!(Lang::from_path("es"), Some(Lang::Es));
        assert_eq!(Lang::from_path("ja"), Some(Lang::Ja));
        assert_eq!(Lang::from_path("fr"), None);
        assert_eq!(Lang::from_path(""), None);
        assert_eq!(Lang::ALL.len(), 3);
    }

    fn write_i18n_files(dir: &std::path::Path, en: &str, ja: &str, es: &str) {
        std::fs::create_dir(dir.join("i18n")).unwrap();
        std::fs::write(dir.join("i18n").join("en.json"), en).unwrap();
        std::fs::write(dir.join("i18n").join("ja.json"), ja).unwrap();
        std::fs::write(dir.join("i18n").join("es.json"), es).unwrap();
    }

    fn write_i18n(dir: &std::path::Path) {
        std::fs::create_dir(dir.join("i18n")).unwrap();
        for code in ["en", "ja", "es"] {
            std::fs::write(dir.join("i18n").join(format!("{code}.json")), "{}").unwrap();
        }
    }

    #[test]
    fn load_translations_parses_all_languages() {
        let tmp = tempfile::tempdir().unwrap();
        write_i18n_files(tmp.path(), r#"{"a":"1"}"#, r#"{"a":"あ"}"#, r#"{"a":"1"}"#);
        let maps = i18n::load_translations(tmp.path()).unwrap();
        assert_eq!(maps[&Lang::Ja]["a"], "あ");
        assert_eq!(maps[&Lang::En]["a"], "1");
        assert_eq!(maps.len(), 3);
    }

    #[test]
    fn load_translations_rejects_key_missing_in_one_language() {
        let tmp = tempfile::tempdir().unwrap();
        write_i18n_files(tmp.path(), r#"{"a":"1","b":"2"}"#, r#"{"a":"あ"}"#, r#"{"a":"1"}"#);
        let err = i18n::load_translations(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("b"), "got: {err}");
        assert!(err.to_string().contains("ja"), "got: {err}");
    }

    #[test]
    fn load_translations_rejects_extra_key_in_one_language() {
        let tmp = tempfile::tempdir().unwrap();
        write_i18n_files(tmp.path(), r#"{"a":"1"}"#, r#"{"a":"あ","extra":"x"}"#, r#"{"a":"1"}"#);
        assert!(i18n::load_translations(tmp.path()).is_err());
    }

    #[test]
    fn load_translations_missing_file_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("i18n")).unwrap();
        std::fs::write(tmp.path().join("i18n").join("en.json"), r#"{"a":"1"}"#).unwrap();
        std::fs::write(tmp.path().join("i18n").join("ja.json"), r#"{"a":"あ"}"#).unwrap();
        // es.json absent
        assert!(i18n::load_translations(tmp.path()).is_err());
    }

    #[test]
    fn html_escape_escapes_special_chars() {
        assert_eq!(i18n::html_escape("a<b>&\"c'"), "a&lt;b&gt;&amp;&quot;c&#39;");
        assert_eq!(i18n::html_escape("plain"), "plain");
    }

    #[test]
    fn render_index_replaces_all_placeholders() {
        let tmp = tempfile::tempdir().unwrap();
        write_i18n_files(
            tmp.path(),
            r#"{"hello":"Hello {0}","who":"world"}"#,
            r#"{"hello":"こんにちは {0}","who":"世界"}"#,
            r#"{"hello":"Hola {0}","who":"mundo"}"#,
        );
        let dicts = i18n::load_translations(tmp.path()).unwrap();
        let tmpl = concat!(
            "<html lang=\"{{LANG_PATH}}\">",
            "<title>{{t:hello}}</title>",
            "{{HREFLANG_LINKS}}{{LANG_SWITCHER}}",
            "<!-- //$$VERSION_OPTIONS$$ -->",
            "<!-- //$$I18N_DICT$$ -->",
        );
        let page = render_index(tmpl, Lang::Ja, &dicts[&Lang::Ja], "OPTS").unwrap();
        assert!(page.contains("<html lang=\"ja\">"), "{page}");
        assert!(page.contains("こんにちは {0}"), "{page}");
        assert!(page.contains("hreflang=\"en\""), "{page}");
        assert!(page.contains("hreflang=\"ja\""), "{page}");
        assert!(page.contains("hreflang=\"es\""), "{page}");
        assert!(page.contains("hreflang=\"x-default\""), "{page}");
        assert!(page.contains("href=\"/ja/\" class=\"lang-active\">日本語</a>"), "{page}");
        assert!(page.contains("href=\"/en/\">English</a>"), "{page}");
        assert!(page.contains("window.I18N = {"), "{page}");
        assert!(page.contains("\"hello\":\"こんにちは {0}\""), "{page}");
        assert!(page.contains("<!-- OPTS -->"), "{page}");
        for marker in [
            "{{t:",
            "{{LANG_PATH}}",
            "{{HREFLANG_LINKS}}",
            "{{LANG_SWITCHER}}",
            "//$$I18N_DICT$$",
            "//$$VERSION_OPTIONS$$",
        ] {
            assert!(!page.contains(marker), "marker {marker} left over: {page}");
        }
    }

    #[test]
    fn render_index_errors_on_unknown_key() {
        let tmp = tempfile::tempdir().unwrap();
        write_i18n_files(
            tmp.path(),
            r#"{"a":"x"}"#,
            r#"{"a":"x"}"#,
            r#"{"a":"x"}"#,
        );
        let dicts = i18n::load_translations(tmp.path()).unwrap();
        let err = render_index("{{t:missing}}", Lang::En, &dicts[&Lang::En], "").unwrap_err();
        assert!(err.to_string().contains("missing"), "got: {err}");
    }

    #[test]
    fn render_index_errors_on_unterminated_placeholder() {
        let err = render_index("...{{t:oops", Lang::En, &BTreeMap::new(), "").unwrap_err();
        assert!(err.to_string().contains("unterminated"), "got: {err}");
    }

    #[test]
    fn render_index_html_escapes_injected_values() {
        let tmp = tempfile::tempdir().unwrap();
        write_i18n_files(tmp.path(), r#"{"v":"a\"<b>&c"}"#, r#"{"v":"x"}"#, r#"{"v":"x"}"#);
        let dicts = i18n::load_translations(tmp.path()).unwrap();
        let page = render_index("<p>{{t:v}}</p>", Lang::En, &dicts[&Lang::En], "").unwrap();
        assert!(page.contains("a&quot;&lt;b&gt;&amp;c"), "{page}");
    }

    #[test]
    fn build_index_returns_all_three_languages() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("weeks")).unwrap();
        create_week_db(&tmp.path().join("weeks"), 0, 9, 0, 0, entry(0, &[0xAA]));
        std::fs::write(
            tmp.path().join("index.html.tmpl"),
            "<html lang=\"{{LANG_PATH}}\"><!-- //$$VERSION_OPTIONS$$ --></html>",
        )
        .unwrap();
        std::fs::create_dir(tmp.path().join("assets")).unwrap();
        write_i18n(tmp.path());
        let mut mgr = DatabaseManager::new();
        mgr.initialize_week_databases(&tmp.path().join("weeks")).unwrap();
        let dates = mgr.get_date_list();
        let (pages, latest) = build_index(tmp.path(), &dates).unwrap();
        assert_eq!(pages.len(), 3);
        assert!(pages[&Lang::Ja].contains("<html lang=\"ja\">"));
        assert!(!latest.is_empty());
    }

    fn router_fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        create_week_db(&tmp.path().join("weeks"), 0, 9, 0, 0, entry(0, &[0xAA]));
        std::fs::write(
            tmp.path().join("index.html.tmpl"),
            "<html lang=\"en\" data-path=\"{{LANG_PATH}}\">{{HREFLANG_LINKS}}{{LANG_SWITCHER}}<!-- //$$VERSION_OPTIONS$$ --></html>",
        )
        .unwrap();
        std::fs::create_dir(tmp.path().join("assets")).unwrap();
        write_i18n(tmp.path());
        tmp
    }

    #[tokio::test]
    async fn index_root_redirects_by_accept_language() {
        let tmp = router_fixture();
        let ts = Arc::new(TileServer::new(tmp.path().to_path_buf()).unwrap());
        let app = build_router(ts);
        for (header, expected) in [
            ("en-US,en;q=0.9", "/en/"),
            ("ja-JP,ja;q=0.9,en;q=0.8", "/ja/"),
            ("es-ES,es;q=0.9", "/es/"),
            ("fr-FR", "/en/"),
        ] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/")
                        .header("accept-language", header)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::FOUND);
            assert_eq!(resp.headers().get("location").unwrap(), expected);
        }
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FOUND);
        assert_eq!(resp.headers().get("location").unwrap(), "/en/");
    }

    #[tokio::test]
    async fn lang_path_serves_localized_page() {
        let tmp = router_fixture();
        let ts = Arc::new(TileServer::new(tmp.path().to_path_buf()).unwrap());
        let app = build_router(ts);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/ja/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = String::from_utf8(to_bytes(resp.into_body(), 64 * 1024).await.unwrap().to_vec())
            .unwrap();
        assert!(body.contains("data-path=\"ja\""), "{body}");
        assert!(body.contains("hreflang=\"x-default\""), "{body}");
        assert!(
            body.contains("href=\"/ja/\" class=\"lang-active\">日本語</a>"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn lang_without_trailing_slash_redirects() {
        let tmp = router_fixture();
        let ts = Arc::new(TileServer::new(tmp.path().to_path_buf()).unwrap());
        let app = build_router(ts);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/en")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FOUND);
        assert_eq!(resp.headers().get("location").unwrap(), "/en/");
    }

    #[tokio::test]
    async fn unknown_lang_path_is_404() {
        let tmp = router_fixture();
        let ts = Arc::new(TileServer::new(tmp.path().to_path_buf()).unwrap());
        let app = build_router(ts);
        for uri in ["/fr/", "/fr"] {
            let resp = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        }
    }
}
