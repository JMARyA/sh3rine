/// Returns S3 key candidates to try in order for a given request path.
/// Returns `None` if the path contains traversal sequences.
pub fn candidates(path: &str) -> Option<Vec<String>> {
    let p = path.trim_start_matches('/');

    // Reject path traversal — reqwest normalizes URLs so `../` would escape the bucket prefix.
    if p.split('/').any(|seg| seg == ".." || seg == ".") {
        return None;
    }

    Some(if p.is_empty() || p.ends_with('/') {
        vec![format!("{}index.html", p)]
    } else if has_extension(p) {
        vec![p.to_string()]
    } else {
        vec![
            format!("{}.html", p),
            format!("{}/index.html", p),
            p.to_string(),
        ]
    })
}

fn has_extension(path: &str) -> bool {
    path.rsplit('/').next().map_or(false, |seg| seg.contains('.'))
}

/// Validates that a bucket name contains only safe S3 characters.
/// Prevents path traversal via regex-extracted bucket names.
pub fn is_valid_bucket_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Determine Content-Type from the S3 key name, falling back to byte sniffing.
pub fn content_type(key: &str, body: &[u8]) -> String {
    let from_ext = mime_guess::from_path(key).first();
    if let Some(mime) = from_ext {
        if mime.essence_str() != "application/octet-stream" {
            return mime.to_string();
        }
    }
    // Sniff bytes for common formats mime_guess can't detect by extension alone
    // (e.g. extensionless AVIF, WebP, etc.)
    infer::get(body)
        .map(|t| t.mime_type().to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string())
}
