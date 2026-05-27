mod config;
mod errors;
mod resolve;

use std::{sync::Arc, time::{Duration, Instant}};

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use bytes::Bytes;
use metrics_exporter_prometheus::PrometheusBuilder;
use moka::future::Cache;
use reqwest::Client;
use tracing::{debug, info, warn};

use config::Config;

#[derive(Clone)]
struct Cached {
    content_type: HeaderValue,
    body: Bytes,
}

#[derive(Clone)]
struct App {
    config: Arc<Config>,
    cache_control: Option<HeaderValue>,
    // Client and Cache are already internally Arc-backed; no outer Arc needed.
    client: Client,
    cache: Cache<String, Cached>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = Config::from_env().expect("bad config");
    let listen = config.listen.clone();
    let metrics_listen = config.metrics_listen.clone();

    // Install the Prometheus recorder globally; get a handle to render output.
    let prometheus_handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install Prometheus recorder");

    let cache = Cache::builder()
        .max_capacity(config.cache_max_mb * 1024 * 1024)
        .time_to_live(Duration::from_secs(config.cache_ttl_secs))
        .weigher(|_k, v: &Cached| v.body.len() as u32)
        .build();

    let cache_control = config
        .cache_control
        .as_deref()
        .and_then(|v| HeaderValue::from_str(v).ok());

    // Expose the configured cap so the dashboard can show fill %.
    metrics::gauge!("sh3rine_cache_max_bytes")
        .set((config.cache_max_mb * 1024 * 1024) as f64);

    let app = App {
        client: Client::builder()
            .timeout(Duration::from_secs(config.s3_timeout_secs))
            .build()
            .unwrap(),
        cache_control,
        config: Arc::new(config),
        cache,
    };

    // Background task: push cache gauges into the metrics registry every 15 s.
    // moka's entry_count / weighted_size are O(1) reads.
    {
        let cache = app.cache.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(15));
            loop {
                tick.tick().await;
                metrics::gauge!("sh3rine_cache_entries")
                    .set(cache.entry_count() as f64);
                metrics::gauge!("sh3rine_cache_size_bytes")
                    .set(cache.weighted_size() as f64);
            }
        });
    }

    // Optional metrics server — intentionally on a different port so it's never
    // accidentally reachable via the public-facing listener.
    if let Some(addr) = metrics_listen {
        let handle = prometheus_handle.clone();
        let addr_log = addr.clone();
        tokio::spawn(async move {
            let router = Router::new().route(
                "/metrics",
                get(move || {
                    let h = handle.clone();
                    async move { h.render() }
                }),
            );
            let listener = tokio::net::TcpListener::bind(&addr)
                .await
                .expect("failed to bind METRICS_LISTEN");
            info!("sh3rine metrics on {addr_log}");
            axum::serve(listener, router).await.unwrap();
        });
    }

    let router = Router::new()
        .route("/healthz", get(|| async { (StatusCode::OK, "ok\n") }))
        .fallback(handle)
        .with_state(app);

    info!("sh3rine listening on {listen}");
    let listener = tokio::net::TcpListener::bind(&listen).await.unwrap();
    axum::serve(listener, router).await.unwrap();
}

async fn handle(State(app): State<App>, req: Request) -> Response {
    let start = Instant::now();
    let path = req.uri().path().to_string();

    let host = req
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let Some(bucket) = app.config.resolve_bucket(&host) else {
        warn!(host = %host, "no bucket configured");
        return errors::no_host(&host);
    };

    let Some(candidates) = resolve::candidates(&path) else {
        return errors::bad_request(&path);
    };

    debug!(host = %host, bucket = %bucket, path = %path, "resolving");

    for key in candidates {
        match fetch(&app, &bucket, &key, StatusCode::OK).await {
            FetchResult::Hit(resp, cache_status) => {
                record_request(&bucket, resp.status().as_u16(), cache_status, start);
                info!(
                    host = %host, bucket = %bucket, path = %path,
                    status = resp.status().as_u16(),
                    cache = cache_status,
                    duration_ms = start.elapsed().as_millis(),
                );
                return resp;
            }
            FetchResult::Err => {
                // warn already emitted inside fetch(); emit a request-level line too.
                record_request(&bucket, 500, "ERR", start);
                info!(
                    host = %host, bucket = %bucket, path = %path,
                    status = 500u16, cache = "ERR",
                    duration_ms = start.elapsed().as_millis(),
                );
                return errors::server_error();
            }
            FetchResult::Miss => {}
        }
    }

    // Bucket-level 404 page — also tracked in upstream metrics by fetch().
    let (resp, cache_status) =
        if let FetchResult::Hit(r, cs) = fetch(&app, &bucket, "404.html", StatusCode::NOT_FOUND).await {
            (r, cs)
        } else {
            (errors::not_found(&path), "MISS")
        };

    record_request(&bucket, 404, cache_status, start);
    info!(
        host = %host, bucket = %bucket, path = %path,
        status = 404u16, cache = cache_status,
        duration_ms = start.elapsed().as_millis(),
    );
    resp
}

/// Increment request counter + record latency histogram.
fn record_request(bucket: &str, status: u16, cache: &'static str, start: Instant) {
    let elapsed = start.elapsed().as_secs_f64();
    metrics::counter!("sh3rine_requests_total",
        "bucket" => bucket.to_string(),
        "status" => status.to_string(),
        "cache"  => cache,
    ).increment(1);
    metrics::histogram!("sh3rine_request_duration_seconds",
        "bucket" => bucket.to_string(),
        "cache"  => cache,
    ).record(elapsed);
}

/// Increment upstream counter + record upstream latency.
/// Only called for actual S3 fetches (cache hits bypass this).
fn record_upstream(bucket: &str, result: &'static str, elapsed: f64) {
    metrics::counter!("sh3rine_upstream_requests_total",
        "bucket" => bucket.to_string(),
        "result" => result,
    ).increment(1);
    metrics::histogram!("sh3rine_upstream_duration_seconds",
        "bucket" => bucket.to_string(),
    ).record(elapsed);
}

/// Carries the cache status label alongside the response so `handle` can log
/// and record it without re-reading the response header.
enum FetchResult {
    Hit(Response, &'static str), // (response, cache: "HIT" | "MISS" | "STREAM")
    Miss,
    Err,
}

async fn fetch(app: &App, bucket: &str, key: &str, status: StatusCode) -> FetchResult {
    let cache_key = format!("{bucket}/{key}");
    let max_bytes = app.config.cache_max_bytes;

    // ── Cache hit ──────────────────────────────────────────────────────────────
    if let Some(hit) = app.cache.get(&cache_key).await {
        debug!(key = %cache_key, "cache HIT");
        return FetchResult::Hit(
            buffered_response(app, status, hit.content_type, hit.body, true),
            "HIT",
        );
    }

    // ── S3 fetch ───────────────────────────────────────────────────────────────
    let url = format!("{}/{}/{}", app.config.endpoint, bucket, key);
    let t0 = Instant::now();

    let resp = match app.client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            let elapsed = t0.elapsed().as_secs_f64();
            let kind = if e.is_timeout() { "timeout" } else { "connect_error" };
            warn!(bucket = %bucket, key = %key, kind, "upstream fetch failed");
            record_upstream(bucket, kind, elapsed);
            return FetchResult::Err;
        }
    };

    let s3_status = resp.status();

    if s3_status.is_server_error() {
        warn!(bucket = %bucket, key = %key, status = s3_status.as_u16(), "upstream server error");
        record_upstream(bucket, "server_error", t0.elapsed().as_secs_f64());
        return FetchResult::Err;
    }

    if !s3_status.is_success() {
        debug!(bucket = %bucket, key = %key, status = s3_status.as_u16(), "upstream miss");
        record_upstream(bucket, "miss", t0.elapsed().as_secs_f64());
        return FetchResult::Miss;
    }

    // ── Stream large files ─────────────────────────────────────────────────────
    // Content-Length is reliable for RGW; extension-only content-type is fine for large files.
    if resp.content_length().map_or(false, |len| len > max_bytes) {
        record_upstream(bucket, "hit", t0.elapsed().as_secs_f64());
        let ct = resolve::content_type(key, &[]);
        debug!(key = %cache_key, "streaming");
        return FetchResult::Hit(streaming_response(app, status, &ct, resp), "STREAM");
    }

    // ── Buffer + cache ─────────────────────────────────────────────────────────
    let body = match resp.bytes().await {
        Ok(b) => b,
        Err(_) => {
            warn!(bucket = %bucket, key = %key, "failed to read upstream body");
            record_upstream(bucket, "read_error", t0.elapsed().as_secs_f64());
            return FetchResult::Err;
        }
    };

    record_upstream(bucket, "hit", t0.elapsed().as_secs_f64());

    let ct_str = resolve::content_type(key, &body);
    let ct = HeaderValue::from_str(&ct_str)
        .unwrap_or(HeaderValue::from_static("application/octet-stream"));

    if body.len() as u64 <= max_bytes {
        app.cache
            .insert(cache_key, Cached { content_type: ct.clone(), body: body.clone() })
            .await;
    }

    FetchResult::Hit(buffered_response(app, status, ct, body, false), "MISS")
}

fn buffered_response(
    app: &App,
    status: StatusCode,
    content_type: HeaderValue,
    body: Bytes,
    cached: bool,
) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(
        "x-sh3rine-cache",
        HeaderValue::from_static(if cached { "HIT" } else { "MISS" }),
    );
    if let Some(cc) = &app.cache_control {
        headers.insert(header::CACHE_CONTROL, cc.clone());
    }
    (status, headers, Body::from(body)).into_response()
}

fn streaming_response(
    app: &App,
    status: StatusCode,
    content_type: &str,
    resp: reqwest::Response,
) -> Response {
    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(content_type) {
        headers.insert(header::CONTENT_TYPE, v);
    }
    if let Some(len) = resp.content_length() {
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    }
    if let Some(cc) = &app.cache_control {
        headers.insert(header::CACHE_CONTROL, cc.clone());
    }
    headers.insert("x-sh3rine-cache", HeaderValue::from_static("STREAM"));
    (status, headers, Body::from_stream(resp.bytes_stream())).into_response()
}
