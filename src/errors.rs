use axum::{
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};

/// Typed upstream failure — used both for error pages and for metrics labels.
pub enum UpstreamError {
    Timeout,
    Connect,
    ServerError(u16),
    ReadBody,
}

impl UpstreamError {
    /// Prometheus-safe label value.
    pub fn metric_label(&self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Connect => "connect_error",
            Self::ServerError(_) => "server_error",
            Self::ReadBody => "read_error",
        }
    }

    fn description(&self) -> String {
        match self {
            Self::Timeout => "The request to upstream storage timed out.".into(),
            Self::Connect => "Could not connect to upstream storage.".into(),
            Self::ServerError(s) => format!("Upstream storage returned HTTP {s}."),
            Self::ReadBody => "Failed to read the response body from upstream storage.".into(),
        }
    }
}

const CSS: &str = "\
*{box-sizing:border-box;margin:0;padding:0}\
body{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;\
background:#070000;color:#c8aaaa;min-height:100vh;display:flex;\
align-items:center;justify-content:center}\
.wrap{text-align:center;padding:2rem;max-width:560px;width:100%}\
.gate{font-size:7rem;line-height:1;margin-bottom:1.5rem;\
filter:drop-shadow(0 0 24px #7a0000) drop-shadow(0 0 6px #3d0000)}\
.code{font-size:5.5rem;font-weight:900;color:#c01010;line-height:1;\
letter-spacing:-0.05em;text-shadow:0 0 40px #6e0000,0 2px 0 #1a0000}\
.bar{width:3.5rem;height:2px;background:#5a0000;margin:1.25rem auto;border-radius:2px}\
.title{font-size:1.375rem;font-weight:600;color:#e8cccc;margin-bottom:.75rem}\
.desc{color:#6b4040;font-size:.9375rem;line-height:1.7}\
.detail{margin-top:1.25rem;padding:.75rem 1rem;background:#110000;\
border:1px solid #3d0000;border-radius:.25rem;text-align:left}\
.detail+.detail{margin-top:.5rem}\
.detail-label{font-size:.6875rem;text-transform:uppercase;letter-spacing:.06em;\
color:#5a2a2a;margin-bottom:.3rem;font-family:inherit}\
.detail-value{font-family:'SF Mono','Fira Code',monospace;font-size:.8125rem;\
color:#cc3333;word-break:break-all;white-space:pre-wrap}\
a.home{display:inline-block;margin-top:2rem;padding:.5625rem 1.5rem;\
background:#0d0000;color:#bb2222;text-decoration:none;border-radius:.25rem;\
font-size:.875rem;font-weight:500;border:1px solid #5a0000;letter-spacing:.02em}\
a.home:hover{background:#1a0000;border-color:#991111;color:#e03333}";

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render an error page. `details` is a list of (label, value) pairs;
/// values are escaped and rendered in a monospace block that preserves newlines.
fn page(code: u16, title: &str, desc: &str, details: &[(&str, &str)]) -> String {
    let detail_blocks: String = details
        .iter()
        .map(|(label, val)| {
            format!(
                "<div class=\"detail\">\
                 <div class=\"detail-label\">{label}</div>\
                 <div class=\"detail-value\">{}</div>\
                 </div>",
                escape(val)
            )
        })
        .collect();

    format!(
        "<!doctype html><html lang=\"en\"><head>\
         <meta charset=\"UTF-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{code} {title}</title>\
         <style>{CSS}</style></head><body>\
         <div class=\"wrap\">\
           <div class=\"gate\">&#x26E9;</div>\
           <div class=\"code\">{code}</div>\
           <div class=\"bar\"></div>\
           <div class=\"title\">{title}</div>\
           <p class=\"desc\">{desc}</p>\
           {detail_blocks}\
           <a class=\"home\" href=\"/\">&#8592; Go home</a>\
         </div></body></html>",
    )
}

fn html_resp(status: StatusCode, body: String) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
        body,
    )
        .into_response()
}

/// 404 — shows the request path and the S3 keys that were actually tried.
pub fn not_found(path: &str, tried: &[String]) -> Response {
    let tried_str = tried.join("\n");
    html_resp(
        StatusCode::NOT_FOUND,
        page(
            404,
            "Page Not Found",
            "The page you&#8217;re looking for doesn&#8217;t exist in this bucket.",
            &[
                ("Requested path", path),
                ("Keys tried", &tried_str),
            ],
        ),
    )
}

/// 404 — unknown hostname, no bucket mapping found.
pub fn no_host(host: &str) -> Response {
    html_resp(
        StatusCode::NOT_FOUND,
        page(
            404,
            "No Site Configured",
            "No site is configured for this hostname. Check the server configuration.",
            &[("Hostname", host)],
        ),
    )
}

/// 400 — path traversal rejected.
pub fn bad_request(path: &str) -> Response {
    html_resp(
        StatusCode::BAD_REQUEST,
        page(
            400,
            "Bad Request",
            "The requested path is not valid. Path traversal sequences (&#8220;..&#8221;) are not permitted.",
            &[("Rejected path", path)],
        ),
    )
}

/// 500 — upstream fetch failed. Shows what went wrong and which key was being fetched.
pub fn server_error(bucket: &str, key: &str, err: &UpstreamError) -> Response {
    let s3_key = format!("{bucket}/{key}");
    html_resp(
        StatusCode::INTERNAL_SERVER_ERROR,
        page(
            500,
            "Server Error",
            "An error occurred while fetching content from upstream storage.",
            &[
                ("Reason", &err.description()),
                ("Key", &s3_key),
            ],
        ),
    )
}
