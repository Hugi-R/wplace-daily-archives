//! OpenTelemetry metrics for the tile server (metrics-only, OTLP HTTP, opt-in).

use std::sync::OnceLock;

use opentelemetry::{
    KeyValue, global,
    metrics::{Counter, Histogram},
};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, metrics::SdkMeterProvider};

/// Histogram buckets (seconds) for `http.server.request.duration`.
const DURATION_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

struct Instruments {
    counter: Counter<u64>,
    histogram: Histogram<f64>,
}

fn instruments() -> &'static Instruments {
    static INSTRUMENTS: OnceLock<Instruments> = OnceLock::new();
    INSTRUMENTS.get_or_init(|| {
        let meter = global::meter("wpda-tileserver");
        Instruments {
            counter: meter
                .u64_counter("http.server.request.count")
                .with_description("Number of HTTP requests received.")
                .build(),
            histogram: meter
                .f64_histogram("http.server.request.duration")
                .with_unit("s")
                .with_description("Duration of HTTP server requests.")
                .with_boundaries(DURATION_BUCKETS.to_vec())
                .build(),
        }
    })
}

/// Build the OTEL resource: SDK defaults (incl. `OTEL_RESOURCE_ATTRIBUTES`
/// and `OTEL_SERVICE_NAME` via env detection) plus `service.name`, plus
/// `deployment.environment.name` when `DEPLOYMENT_ENVIRONMENT` is set.
/// The dedicated env var wins if both sources define the environment.
fn build_resource() -> Resource {
    let builder = Resource::builder().with_service_name("wpda-tileserver");
    match std::env::var("DEPLOYMENT_ENVIRONMENT")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        Some(env) => builder
            .with_attribute(KeyValue::new("deployment.environment.name", env))
            .build(),
        None => builder.build(),
    }
}

/// Initialize the global meter provider.
///
/// Opt-in: returns `None` (leaving the global no-op provider in place) unless
/// `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` or `OTEL_EXPORTER_OTLP_ENDPOINT` is
/// set. The returned provider must be kept alive for the process lifetime and
/// shut down on exit so buffered metrics flush.
pub fn init_meter_provider() -> Option<SdkMeterProvider> {
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
                .ok()
                .filter(|v| !v.trim().is_empty())
        });
    let endpoint = endpoint?;
    // Accept both a bare collector origin ("http://collector:4318", the
    // standard `OTEL_EXPORTER_OTLP_ENDPOINT` form) and a full metrics URL.
    let endpoint = if endpoint.contains("/v1/metrics") {
        endpoint
    } else {
        format!("{}/v1/metrics", endpoint.trim_end_matches('/'))
    };

    let exporter = match opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_endpoint(&endpoint)
        .build()
    {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("OTEL exporter init failed ({e}); metrics disabled");
            return None;
        }
    };
    let provider = SdkMeterProvider::builder()
        .with_resource(build_resource())
        .with_periodic_exporter(exporter)
        .build();
    global::set_meter_provider(provider.clone());
    tracing::info!("OTEL metrics exporting to {endpoint}");
    Some(provider)
}

/// Map the request method to a low-cardinality label; unknown methods become
/// `_OTHER` per the OTel spec so clients cannot inflate cardinality.
fn method_label(method: &str) -> &'static str {
    match method {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        "CONNECT" => "CONNECT",
        "PATCH" => "PATCH",
        "TRACE" => "TRACE",
        _ => "_OTHER",
    }
}

/// Per-route extra attributes:
///
/// - `/tiles/...` -> `tile.version` (week number only; `x`/`y`/`z` deliberately
///   omitted — millions of series otherwise).
/// - `/`, `/{lang}/`, `/{lang}` -> `user.accept_languages` (full ordered list
///   of primary subtags parsed from the `Accept-Language` header, e.g.
///   `"fr,ja"` — the *requested* languages, not the resolved page) plus
///   `page.lang` (the language in the URL path) on the `/{lang}` routes.
pub fn extra_attributes(path: &str, route: &str, accept_language: Option<&str>) -> Vec<KeyValue> {
    let mut extra = Vec::new();
    if route == "/tiles/{version}/{z}/{x}/{y}" {
        if let Some(version) = tile_version_from_path(path) {
            extra.push(KeyValue::new("tile.version", version as i64));
        }
        return extra;
    }
    if route == "/" || route == "/{lang}/" || route == "/{lang}" {
        if let Some(header) = accept_language {
            extra.push(KeyValue::new(
                "user.accept_languages",
                parse_accept_languages(header),
            ));
        }
        if route != "/" {
            let segment = path.trim_matches('/');
            if !segment.is_empty() {
                let mut lang = segment.to_string();
                if lang.len() > 16 {
                    lang.truncate(16);
                }
                extra.push(KeyValue::new("page.lang", lang));
            }
        }
    }
    extra
}

fn record_on(
    counter: &Counter<u64>,
    histogram: &Histogram<f64>,
    route: &str,
    method: &str,
    status: u16,
    duration_secs: f64,
    extra: &[KeyValue],
) {
    let mut attrs = Vec::with_capacity(3 + extra.len());
    attrs.push(KeyValue::new("http.route", route.to_string()));
    attrs.push(KeyValue::new("http.request.method", method_label(method)));
    attrs.push(KeyValue::new("http.response.status_code", status as i64));
    attrs.extend(extra.iter().cloned());
    counter.add(1, &attrs);
    histogram.record(duration_secs, &attrs);
}

/// Record one finished request: `+1` on the request counter and one
/// duration sample on the response-time histogram.
pub fn record_request(
    route: &str,
    method: &str,
    status: u16,
    duration_secs: f64,
    extra: &[KeyValue],
) {
    let inst = instruments();
    record_on(
        &inst.counter,
        &inst.histogram,
        route,
        method,
        status,
        duration_secs,
        extra,
    );
}

/// Parse an `Accept-Language` header into a full ordered list of primary
/// language subtags, e.g. "ja-JP,ja;q=0.9,en;q=0.8" -> "ja,en".
pub fn parse_accept_languages(header: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for tag in header.split(',') {
        let subtag = tag.split(';').next().unwrap_or_default().trim();
        if subtag.is_empty() || subtag == "*" {
            continue;
        }
        let primary = subtag
            .split(['-', '_'])
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if primary.is_empty() || primary == "*" {
            continue;
        }
        if !out.contains(&primary) {
            out.push(primary);
        }
    }
    if out.is_empty() {
        return "unknown".to_string();
    }
    let mut joined = out.join(",");
    if joined.len() > 64 {
        joined.truncate(64);
    }
    joined
}

/// Normalize a request path to a low-cardinality route template.
pub fn normalize_route(path: &str) -> &'static str {
    // Strip any query string just in case a raw URI slips through
    // (`req.uri().path()` normally excludes it already).
    let path = path.split('?').next().unwrap_or(path);
    match path {
        "/" => "/",
        "/preview.png" => "/preview.png",
        "/favicon.ico" => "/favicon.ico",
        "/robots.txt" => "/robots.txt",
        "/sitemap.xml" => "/sitemap.xml",
        "/tiles" | "/tiles/" | "/diff" | "/diff/" | "/assets" | "/assets/" => "other",
        p if p.starts_with("/assets/") => "/assets/{filename}",
        p if p.starts_with("/tiles/") => "/tiles/{version}/{z}/{x}/{y}",
        p if p.starts_with("/diff/") => "/diff/all/{z}/{x}/{y}",
        p => {
            // Single-segment paths are the `/{lang}` redirect (no trailing
            // slash) or a `/{lang}/` page (trailing slash). Anything else
            // (multi-segment unknown) is "other".
            let stripped = p.strip_prefix('/').unwrap_or(p);
            if stripped.contains('/') {
                let mut parts = stripped.split('/');
                let first = parts.next().unwrap_or_default();
                let second = parts.next().unwrap_or_default();
                // Exactly one segment + trailing slash, e.g. "/en/".
                if parts.next().is_none() && second.is_empty() && is_lang_segment(first) {
                    return "/{lang}/";
                }
                return "other";
            }
            if is_lang_segment(stripped) {
                return "/{lang}";
            }
            "other"
        }
    }
}

/// A URL path segment that looks like a language code: `en`, `pt-BR`.
fn is_lang_segment(s: &str) -> bool {
    if s.is_empty() || s.len() > 8 || s.contains('.') {
        return false;
    }
    // Known codes always count (covers `pt-BR`).
    if matches!(s, "en" | "ja" | "es" | "pt-BR" | "ko" | "ru" | "tr") {
        return true;
    }
    // Otherwise accept short alpha tags (`fr`, `de`) and `xx-YY` regions
    // so unsupported-language probes still group under `/{lang}`.
    // Deliberately excludes longer words like `tiles`, `diff`, `assets`.
    let mut split = s.split('-');
    let primary = split.next().unwrap_or_default();
    if !(2..=3).contains(&primary.len()) || !primary.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    match split.next() {
        None => true,
        Some(region) => {
            region.len() == 2
                && region.chars().all(|c| c.is_ascii_alphabetic())
                && split.next().is_none()
        }
    }
}

/// Extract the tile `version` (week number) from a `/tiles/...` path.
/// Returns `None` when the path is not a tile path or the version is invalid.
pub fn tile_version_from_path(path: &str) -> Option<u32> {
    let rest = path.strip_prefix("/tiles/")?;
    let version_str = rest.split('/').next().unwrap_or_default();
    match version_str.parse::<f32>() {
        Ok(v) if v.is_finite() && v >= 0.0 => Some(v as u32),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_languages_keeps_full_ordered_list() {
        assert_eq!(parse_accept_languages("ja-JP,ja;q=0.9,en;q=0.8"), "ja,en");
        assert_eq!(parse_accept_languages("en-US,en;q=0.9"), "en");
        assert_eq!(
            parse_accept_languages("fr-FR,ja;q=0.8,en;q=0.7"),
            "fr,ja,en"
        );
        assert_eq!(parse_accept_languages("pt-BR,pt;q=0.9"), "pt");
        assert_eq!(parse_accept_languages("es-ES,es;q=0.9"), "es");
    }

    #[test]
    fn accept_languages_empty_or_wildcard_is_unknown() {
        assert_eq!(parse_accept_languages(""), "unknown");
        assert_eq!(parse_accept_languages("*"), "unknown");
        assert_eq!(parse_accept_languages("  "), "unknown");
    }

    #[test]
    fn accept_languages_dedups_and_lowercases() {
        assert_eq!(parse_accept_languages("EN-us, en;q=0.8"), "en");
        assert_eq!(parse_accept_languages("fr-FR, fr;q=0.9"), "fr");
    }

    #[test]
    fn normalize_route_groups_endpoints() {
        assert_eq!(normalize_route("/"), "/");
        assert_eq!(normalize_route("/en/"), "/{lang}/");
        assert_eq!(normalize_route("/ja/"), "/{lang}/");
        assert_eq!(normalize_route("/pt-BR/"), "/{lang}/");
        assert_eq!(normalize_route("/en"), "/{lang}");
        assert_eq!(
            normalize_route("/tiles/0/9/0/0.zst"),
            "/tiles/{version}/{z}/{x}/{y}"
        );
        assert_eq!(
            normalize_route("/diff/all/9/0/0.zst"),
            "/diff/all/{z}/{x}/{y}"
        );
        assert_eq!(normalize_route("/preview.png"), "/preview.png");
        assert_eq!(normalize_route("/favicon.ico"), "/favicon.ico");
        assert_eq!(normalize_route("/robots.txt"), "/robots.txt");
        assert_eq!(normalize_route("/sitemap.xml"), "/sitemap.xml");
        assert_eq!(normalize_route("/assets/app.js"), "/assets/{filename}");
    }

    #[test]
    fn tile_version_extracted_from_path() {
        assert_eq!(tile_version_from_path("/tiles/0/9/0/0.zst"), Some(0));
        assert_eq!(tile_version_from_path("/tiles/12/9/0/0.zst"), Some(12));
        assert_eq!(tile_version_from_path("/diff/all/9/0/0.zst"), None);
        assert_eq!(tile_version_from_path("/"), None);
        assert_eq!(tile_version_from_path("/tiles/abc/9/0/0.zst"), None);
    }

    #[test]
    fn extra_attributes_for_tiles_has_version_only() {
        let extra = extra_attributes("/tiles/12/9/0/0.zst", "/tiles/{version}/{z}/{x}/{y}", None);
        assert!(
            extra
                .iter()
                .any(|kv| kv.key.as_str() == "tile.version"
                    && kv.value == opentelemetry::Value::I64(12)),
            "got: {extra:?}"
        );
        assert!(
            !extra
                .iter()
                .any(|kv| kv.key.as_str() == "user.accept_languages"),
            "tiles must not carry accept_languages, got: {extra:?}"
        );
    }

    #[test]
    fn extra_attributes_for_root_has_accept_languages() {
        let extra = extra_attributes("/", "/", Some("fr-FR,ja;q=0.8,en;q=0.7"));
        assert!(
            extra
                .iter()
                .any(|kv| kv.key.as_str() == "user.accept_languages"
                    && kv.value == opentelemetry::Value::String("fr,ja,en".into())),
            "got: {extra:?}"
        );
        assert!(
            !extra.iter().any(|kv| kv.key.as_str() == "tile.version"),
            "got: {extra:?}"
        );
    }

    #[test]
    fn extra_attributes_for_lang_page_has_page_lang_and_accept_languages() {
        let extra = extra_attributes("/ja/", "/{lang}/", Some("en-US,en;q=0.9"));
        assert!(
            extra.iter().any(|kv| kv.key.as_str() == "page.lang"
                && kv.value == opentelemetry::Value::String("ja".into())),
            "got: {extra:?}"
        );
        assert!(
            extra
                .iter()
                .any(|kv| kv.key.as_str() == "user.accept_languages"
                    && kv.value == opentelemetry::Value::String("en".into())),
            "got: {extra:?}"
        );
    }

    #[test]
    fn extra_attributes_for_other_routes_is_empty() {
        assert!(extra_attributes("/diff/all/9/0/0.zst", "/diff/all/{z}/{x}/{y}", None).is_empty());
        assert!(extra_attributes("/preview.png", "/preview.png", None).is_empty());
    }

    #[test]
    fn build_resource_includes_deployment_environment() {
        use opentelemetry::{Key, Value};

        // SAFETY: this test is the only one touching DEPLOYMENT_ENVIRONMENT.
        unsafe { std::env::remove_var("DEPLOYMENT_ENVIRONMENT") };
        assert_eq!(
            build_resource().get(&Key::new("deployment.environment.name")),
            None
        );

        unsafe { std::env::set_var("DEPLOYMENT_ENVIRONMENT", "staging") };
        let resource = build_resource();
        assert_eq!(
            resource.get(&Key::new("deployment.environment.name")),
            Some(Value::String("staging".into()))
        );
        assert!(resource.get(&Key::new("service.name")).is_some());

        unsafe { std::env::remove_var("DEPLOYMENT_ENVIRONMENT") };
        assert_eq!(
            build_resource().get(&Key::new("deployment.environment.name")),
            None
        );
    }

    #[test]
    fn record_on_exports_count_and_duration_with_attributes() {
        use opentelemetry::metrics::MeterProvider as _;
        use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};

        let exporter = opentelemetry_sdk::metrics::InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let provider = SdkMeterProvider::builder().with_reader(reader).build();
        let meter = provider.meter("test");
        let counter = meter.u64_counter("http.server.request.count").build();
        let histogram = meter.f64_histogram("http.server.request.duration").build();

        record_on(
            &counter,
            &histogram,
            "/tiles/{version}/{z}/{x}/{y}",
            "GET",
            200,
            0.05,
            &[KeyValue::new("tile.version", 12_i64)],
        );
        provider.force_flush().unwrap();

        let exported = exporter.get_finished_metrics().unwrap();
        let mut saw_count = false;
        let mut saw_duration = false;
        for rm in &exported {
            for sm in rm.scope_metrics() {
                for m in sm.metrics() {
                    use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
                    match m.data() {
                        AggregatedMetrics::U64(MetricData::Sum(sum))
                            if m.name() == "http.server.request.count" =>
                        {
                            for dp in sum.data_points() {
                                assert_eq!(dp.value(), 1);
                                let attrs: Vec<_> = dp.attributes().collect();
                                assert!(attrs.iter().any(|kv| kv.key.as_str() == "http.route"));
                                assert!(attrs.iter().any(|kv| kv.key.as_str() == "tile.version"));
                                saw_count = true;
                            }
                        }
                        AggregatedMetrics::F64(MetricData::Histogram(hist))
                            if m.name() == "http.server.request.duration" =>
                        {
                            for dp in hist.data_points() {
                                assert_eq!(dp.count(), 1);
                                saw_duration = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        assert!(saw_count, "counter datapoint not found");
        assert!(saw_duration, "histogram datapoint not found");
    }
}
