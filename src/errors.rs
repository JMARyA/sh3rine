use axum::{
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};

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
.detail-label{font-size:.6875rem;text-transform:uppercase;letter-spacing:.06em;\
color:#5a2a2a;margin-bottom:.3rem;font-family:inherit}\
.detail-value{font-family:'SF Mono','Fira Code',monospace;font-size:.8125rem;\
color:#cc3333;word-break:break-all}\
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

fn page(code: u16, title: &str, desc: &str, detail_label: Option<&str>, detail: Option<&str>) -> String {
    let detail_block = match (detail_label, detail) {
        (Some(label), Some(val)) => format!(
            "<div class=\"detail\"><div class=\"detail-label\">{label}</div>\
             <div class=\"detail-value\">{}</div></div>",
            escape(val)
        ),
        _ => String::new(),
    };

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
           {detail_block}\
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

pub fn not_found(path: &str) -> Response {
    html_resp(
        StatusCode::NOT_FOUND,
        page(
            404,
            "Page Not Found",
            "The page you&#8217;re looking for doesn&#8217;t exist or has been moved.",
            Some("Requested path"),
            Some(path),
        ),
    )
}

pub fn no_host(host: &str) -> Response {
    html_resp(
        StatusCode::NOT_FOUND,
        page(
            404,
            "No Site Configured",
            "No site is configured for this hostname. Check the server configuration.",
            Some("Hostname"),
            Some(host),
        ),
    )
}

pub fn bad_request(path: &str) -> Response {
    html_resp(
        StatusCode::BAD_REQUEST,
        page(
            400,
            "Bad Request",
            "The requested path is not valid. Path traversal sequences (&#8220;..&#8221;) are not permitted.",
            Some("Rejected path"),
            Some(path),
        ),
    )
}

pub fn server_error() -> Response {
    html_resp(
        StatusCode::INTERNAL_SERVER_ERROR,
        page(
            500,
            "Server Error",
            "An error occurred while fetching the resource from upstream storage. \
             Please try again later or contact the site administrator.",
            None,
            None,
        ),
    )
}
