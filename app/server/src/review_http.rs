//! HTTP policy for generated offline review packages.

pub(crate) fn text_type(ext: &str) -> Option<&'static str> {
    match ext {
        "txt" => Some("text/plain; charset=utf-8"),
        "html" => Some("text/html; charset=utf-8"),
        "json" => Some("application/json; charset=utf-8"),
        _ => None,
    }
}

/// Hash-pin the one inline script in a generated review document. The regular
/// SPA CSP refuses inline JavaScript; this policy is tighter still: no network,
/// no forms, and only the exact generated script may execute.
pub(crate) fn document_csp(ext: &str, bytes: &[u8]) -> Option<axum::http::HeaderValue> {
    if ext != "html" {
        return None;
    }
    use base64::Engine;
    use sha2::{Digest, Sha256};

    let script_hash = std::str::from_utf8(bytes).ok().and_then(|document| {
        let start = document.find("<script>")? + "<script>".len();
        let end = document[start..].find("</script>")? + start;
        Some(
            base64::engine::general_purpose::STANDARD
                .encode(Sha256::digest(&document.as_bytes()[start..end])),
        )
    });
    let script_src = script_hash
        .map(|hash| format!("'sha256-{hash}'"))
        .unwrap_or_else(|| "'none'".into());
    let policy = format!(
        "default-src 'none'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; \
         form-action 'none'; script-src {script_src}; style-src 'unsafe-inline'; \
         media-src 'self' data: blob:; img-src 'none'; font-src 'none'; connect-src 'none'"
    );
    Some(
        axum::http::HeaderValue::from_str(&policy).expect("generated review CSP is a valid header"),
    )
}

pub(crate) fn export_response(
    ext: &str,
    content_type: &'static str,
    bytes: Vec<u8>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let csp = document_csp(ext, &bytes);
    let mut response = (
        [
            (axum::http::header::CONTENT_TYPE, content_type),
            (axum::http::header::ACCEPT_RANGES, "bytes"),
        ],
        bytes,
    )
        .into_response();
    if let Some(csp) = csp {
        response
            .headers_mut()
            .insert(axum::http::header::CONTENT_SECURITY_POLICY, csp);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_document_csp_hash_pins_inline_script() {
        let first = b"<html><script>const reviewed=true;</script></html>";
        let csp = document_csp("html", first).unwrap();
        let csp = csp.to_str().unwrap();
        assert!(csp.contains("script-src 'sha256-"));
        assert!(csp.contains("connect-src 'none'"));
        assert!(!csp.contains("script-src 'unsafe-inline'"));
        let second = b"<html><script>const reviewed=false;</script></html>";
        let different = document_csp("html", second).unwrap();
        assert_ne!(csp, different.to_str().unwrap());
    }

    #[test]
    fn non_html_exports_do_not_get_a_document_policy() {
        assert!(document_csp("json", b"{}").is_none());
        assert_eq!(text_type("json"), Some("application/json; charset=utf-8"));
    }
}
