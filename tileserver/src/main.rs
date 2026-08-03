//! A single file tile server reading from pre-computed SQLite DBs.
//
// Possible follow-ups:
// - Stream get_all_diffs instead of buffering: create a tokio::sync::mpsc channel, blocking_send each chunk from the blocking task, and return Body::from_stream(ReceiverStream::new(rx)).
// - Serve the tile bodies as Bytes read directly from stmt.query_row(|r| r.get_ref(0)) to avoid one copy.

use std::{
    collections::HashMap,
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

            // Skip the part with DateHours == 0 (except on the first iteration).
            let mut skip = 0usize;
            if !is_first && diff_data.len() > 8 {
                let date_hours = u32::from_le_bytes(diff_data[0..4].try_into().unwrap());
                if date_hours == 0 {
                    let length = u32::from_le_bytes(diff_data[4..8].try_into().unwrap()) as usize;
                    // clamp: Go would panic on an out-of-range slice
                    skip = (8 + length).min(diff_data.len());
                }
            }

            out.extend_from_slice(&diff_data[skip..]);
            is_first = false;
        }

        out
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
    index_html: Bytes,
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
            index_html: Bytes::from(index_html),
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

/// Loads index.html.tmpl and replaces `//$$VERSION_OPTIONS$$` with the options.
fn build_index(data_path: &Path, dates: &[u32]) -> Result<(String, String)> {
    let last = *dates
        .last()
        .ok_or_else(|| anyhow!("no versions found in the databases"))?;
    let latest_version = format!("{:.3}", last as f64);

    let tmpl_path = data_path.join("index.html.tmpl");
    let content = fs::read_to_string(&tmpl_path)
        .with_context(|| format!("failed to read {}", tmpl_path.display()))?;

    let options: Vec<String> = dates
        .iter()
        .map(|&epoch_hour| {
            format!(
                "{{version: '{epoch_hour}', date: '{}'}}",
                epoch_hour_to_date(epoch_hour)
            )
        })
        .collect();

    let content = content.replace("//$$VERSION_OPTIONS$$", &options.join(","));
    Ok((content, latest_version))
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

/// GET /diff/all/{z}/{x}/{y}.zst?from=&to=   (only z = 11 is supported)
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
    if z != 11 {
        return text_error(
            StatusCode::BAD_REQUEST,
            "diff tiles only supported for z=11",
        );
    }

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
        Ok(body) => (StatusCode::OK, out_headers, body).into_response(),
        Err(e) => {
            error!("Blocking task failed: {e}");
            text_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        }
    }
}

async fn serve_index(State(ts): State<Arc<TileServer>>) -> Response {
    Html(ts.index_html.clone()).into_response()
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

    // NOTE: axum 0.8 path syntax is `{param}` (it was `:param` in axum 0.7).
    // A path parameter always matches a whole segment, so `{y}` captures
    // "0.zst" and the handler strips the extension.
    let app = Router::new()
        .route("/tiles/{version}/{z}/{x}/{y}", get(serve_tile))
        .route("/diff/all/{z}/{x}/{y}", get(serve_all_diff))
        .route("/", get(serve_index))
        .route("/preview.png", get(serve_preview))
        .route("/favicon.ico", get(serve_favicon))
        .route("/assets/{filename}", get(serve_asset))
        .layer(middleware::from_fn(logging_middleware))
        .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(15))) // ~ Read/WriteTimeout
        .with_state(tile_server);

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