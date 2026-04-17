mod config;
mod resolve;

use std::{sync::Arc, time::Duration};

use axum::{
    body::Body,
    extract::{Host, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use bytes::Bytes;
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

    let cache = Cache::builder()
        .max_capacity(config.cache_max_mb * 1024 * 1024)
        .time_to_live(Duration::from_secs(config.cache_ttl_secs))
        .weigher(|_k, v: &Cached| v.body.len() as u32)
        .build();

    let cache_control = config
        .cache_control
        .as_deref()
        .and_then(|v| HeaderValue::from_str(v).ok());

    let app = App {
        client: Client::builder()
            .timeout(Duration::from_secs(config.s3_timeout_secs))
            .build()
            .unwrap(),
        cache_control,
        config: Arc::new(config),
        cache,
    };

    let router = Router::new()
        .route("/healthz", get(|| async { (StatusCode::OK, "ok\n") }))
        .fallback(handle)
        .with_state(app);

    info!("sh3rine listening on {listen}");
    let listener = tokio::net::TcpListener::bind(&listen).await.unwrap();
    axum::serve(listener, router).await.unwrap();
}

async fn handle(Host(host): Host, State(app): State<App>, req: Request) -> Response {
    let path = req.uri().path().to_string();

    let Some(bucket) = app.config.resolve_bucket(&host) else {
        warn!(host = %host, "no bucket configured");
        return (StatusCode::NOT_FOUND, "not found\n").into_response();
    };

    let Some(candidates) = resolve::candidates(&path) else {
        return (StatusCode::BAD_REQUEST, "invalid path\n").into_response();
    };

    debug!(host = %host, bucket = %bucket, path = %path, "request");

    for key in candidates {
        if let Some(resp) = fetch(&app, &bucket, &key, StatusCode::OK).await {
            return resp;
        }
    }

    // Bucket-level 404 page
    if let Some(resp) = fetch(&app, &bucket, "404.html", StatusCode::NOT_FOUND).await {
        return resp;
    }

    (StatusCode::NOT_FOUND, "404 not found\n").into_response()
}

async fn fetch(app: &App, bucket: &str, key: &str, status: StatusCode) -> Option<Response> {
    let cache_key = format!("{bucket}/{key}");
    let max_bytes = app.config.cache_max_bytes;

    if let Some(hit) = app.cache.get(&cache_key).await {
        debug!("cache HIT {cache_key}");
        return Some(buffered_response(app, status, hit.content_type, hit.body, true));
    }

    let url = format!("{}/{}/{}", app.config.endpoint, bucket, key);
    let resp = app.client.get(&url).send().await.ok()?;

    if !resp.status().is_success() {
        return None;
    }

    // Stream large files — avoid buffering the full body before the first byte goes out.
    // Content-Length is reliable for RGW; extension-only content-type is fine for large files.
    if resp.content_length().map_or(false, |len| len > max_bytes) {
        let ct = resolve::content_type(key, &[]);
        debug!("streaming {cache_key}");
        return Some(streaming_response(app, status, &ct, resp));
    }

    let body = resp.bytes().await.ok()?;
    let ct_str = resolve::content_type(key, &body);
    let ct = HeaderValue::from_str(&ct_str).unwrap_or(HeaderValue::from_static("application/octet-stream"));

    if body.len() as u64 <= max_bytes {
        app.cache
            .insert(cache_key, Cached { content_type: ct.clone(), body: body.clone() })
            .await;
    }

    Some(buffered_response(app, status, ct, body, false))
}

fn buffered_response(app: &App, status: StatusCode, content_type: HeaderValue, body: Bytes, cached: bool) -> Response {
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

fn streaming_response(app: &App, status: StatusCode, content_type: &str, resp: reqwest::Response) -> Response {
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
