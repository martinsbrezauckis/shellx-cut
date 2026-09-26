//! HTTP policy for isolated offline review packages.

pub(crate) fn text_type(ext: &str) -> Option<&'static str> {
    match ext {
        "txt" => Some("text/plain; charset=utf-8"),
        "html" => Some("text/html; charset=utf-8"),
        "json" => Some("application/json; charset=utf-8"),
        _ => None,
    }
}

/// Project exports are untrusted even when Cut normally generates the filename.
/// A CSP sandbox gives HTML an opaque origin. Its inline review script can still
/// run, play the sibling exported video, and download feedback, but it cannot
/// inherit the editor's origin or call Cut's local API. This policy never hashes
/// untrusted script bytes.
pub(crate) fn isolate_html_export(response: &mut axum::response::Response) {
    use axum::http::{header, HeaderValue};
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox allow-scripts allow-downloads; default-src 'none'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; media-src http://127.0.0.1:* http://localhost:* data: blob:; connect-src 'none'; img-src 'none'; font-src 'none'"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_export_runs_in_an_opaque_origin() {
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        isolate_html_export(&mut response);
        let policy = response.headers()["content-security-policy"]
            .to_str()
            .unwrap();
        assert!(policy.starts_with("sandbox allow-scripts allow-downloads;"));
        assert!(!policy.contains("allow-same-origin"));
        assert!(policy.contains("connect-src 'none'"));
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    }

    #[test]
    fn non_html_exports_do_not_get_a_document_policy() {
        assert_eq!(text_type("json"), Some("application/json; charset=utf-8"));
    }
}
