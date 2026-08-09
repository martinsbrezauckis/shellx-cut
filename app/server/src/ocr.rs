//! ocr.rs — server-side OCR + PII detection for REDACTION AUTO-DETECT
//! (`edit.redact{ocr_auto}`).
//!
//! Role: read text + boxes from one video frame (via the perception venv's
//! RapidOCR one-shot runner) and decide which boxes are SENSITIVE (passwords /
//! API keys / emails / cards / PII), so the agent can auto-redact them without
//! the human eyeballing every frame. The OCR + matching run ONCE in the dispatch
//! handler (like edit.track); the resulting region is committed into a normal
//! `edit.redact` op, so replay never re-runs OCR (deterministic + offline).
//!
//! Security posture (FAIL-SAFE — never under-redact):
//!  - The PII matchers use `regex` (the trusted tool — a hand-rolled email/key
//!    matcher bug would leak a secret), with a conservative generic-secret rule
//!    (requires letters AND digits, length ≥ 20) so plain words don't trip it.
//!  - The receipt reports the matched CATEGORY + box only — NEVER the matched
//!    text (echoing a detected secret into the op log / receipt would itself be
//!    a leak).
//!  - Redaction granularity is the OCR text BOX (a whole line), and v1 redacts
//!    the UNION of all matched boxes as one region — over-covering is the safe
//!    direction. Tight per-box multi-region masking is handled separately.
//!
//! Dependencies: cut_core (CutError), regex, serde_json. Caller: dispatch.rs
//! `edit_redact` (the ocr_auto branch).

use cut_core::{error_codes, CutError};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// The resolved OCR runtime: the perception python + the one-shot script.
/// `OCR_RUNNER_PY` / `OCR_RUNNER_SCRIPT` override (dev points them at the repo
/// venv, which has rapidocr; the appdata venv needs it installed on consent).
#[derive(Debug, Clone)]
pub struct Runtime {
    pub python: PathBuf,
    pub script: PathBuf,
}

/// The one-shot OCR script (ships beside `instruments.py` in the sidecar payload).
pub fn runner_script() -> PathBuf {
    let (_py, instruments) = cut_perception::sidecar_paths();
    instruments
        .parent()
        .map(|d| d.join("ocr_runner.py"))
        .unwrap_or_else(|| PathBuf::from("ocr_runner.py"))
}

/// `Some` when the perception python + the OCR script exist. `None` → `ocr_auto`
/// returns a setup hint. The runner itself surfaces a crisp error if rapidocr is
/// not installed in the venv.
pub fn runtime() -> Option<Runtime> {
    let python = std::env::var_os("OCR_RUNNER_PY")
        .map(PathBuf::from)
        .unwrap_or_else(|| cut_perception::sidecar_paths().0);
    let script = std::env::var_os("OCR_RUNNER_SCRIPT")
        .map(PathBuf::from)
        .unwrap_or_else(runner_script);
    (python.exists() && script.exists()).then_some(Runtime { python, script })
}

/// One OCR text box: the text + centre/size as FRACTIONS of the frame + confidence.
#[derive(Debug, Clone, Deserialize)]
pub struct OcrBox {
    pub text: String,
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    pub conf: f64,
}

/// The runner's JSON output. `width`/`height` are part of the runner contract
/// (the boxes are already normalized to fractions, so they're informational).
#[derive(Debug, Clone, Deserialize)]
pub struct OcrResult {
    #[allow(dead_code)]
    pub width: u32,
    #[allow(dead_code)]
    pub height: u32,
    pub boxes: Vec<OcrBox>,
}

/// OCR the frame at `at_ms` of `video` via the one-shot runner (same transport as
/// the matte / track runners). Parses the single JSON line.
pub fn run_ocr(rt: &Runtime, video: &Path, at_ms: u64) -> Result<OcrResult, CutError> {
    let mut command = std::process::Command::new(&rt.python);
    command
        .arg(&rt.script)
        .arg(video)
        .arg("--at-ms")
        .arg(at_ms.to_string());
    let out = crate::dispatch::run_bounded_foreground_command(&mut command, "OCR runner").map_err(
        |e| {
            CutError::new(
                error_codes::IO,
                format!("ocr runner spawn failed: {e}"),
                "the local OCR runtime could not be started",
            )
            .with_suggested_action(
                "install the perception sidecar + rapidocr-onnxruntime in its venv",
            )
        },
    )?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(CutError::new(
            error_codes::IO,
            format!("ocr runner failed: {}", err.trim()),
            "the local OCR runtime errored (is rapidocr-onnxruntime installed in the perception venv?)",
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or("")
        .trim();
    serde_json::from_str(line).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("ocr runner output not JSON ({e}); got: {line}"),
            "the runner must print one JSON line on stdout",
        )
    })
}

/// The redaction region chosen from the matched PII boxes: the UNION rect (centre +
/// size, frame fractions) plus the matched CATEGORIES (NOT the text — never echo a
/// secret) and how many boxes matched.
#[derive(Debug, Clone, PartialEq)]
pub struct RedactRegion {
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    pub categories: Vec<String>,
    pub matched: usize,
    /// Lowest OCR confidence among the matched boxes — a fail-safe signal (a
    /// low-confidence detection is still redacted, but the agent can see it).
    pub min_conf: f64,
}

/// Select the PII boxes (optionally restricted to `categories`) and return their
/// UNION rect, expanded by `margin` (fraction of frame, a fail-safe over-cover) and
/// clamped to the frame. `None` when nothing sensitive is found. The matched text is
/// deliberately NOT returned (leak-safety).
pub fn pii_region(
    res: &OcrResult,
    categories: Option<&[String]>,
    margin: f64,
) -> Option<RedactRegion> {
    let mut x0 = f64::MAX;
    let mut y0 = f64::MAX;
    let mut x1 = f64::MIN;
    let mut y1 = f64::MIN;
    let mut cats: Vec<String> = Vec::new();
    let mut matched = 0usize;
    let mut min_conf = 1.0_f64;
    for b in &res.boxes {
        let Some(cat) = pii::category(&b.text) else {
            continue;
        };
        if let Some(want) = categories {
            if !want.iter().any(|c| c == cat) {
                continue;
            }
        }
        matched += 1;
        min_conf = min_conf.min(b.conf);
        if !cats.iter().any(|c| c == cat) {
            cats.push(cat.to_string());
        }
        x0 = x0.min(b.cx - b.w / 2.0);
        y0 = y0.min(b.cy - b.h / 2.0);
        x1 = x1.max(b.cx + b.w / 2.0);
        y1 = y1.max(b.cy + b.h / 2.0);
    }
    if matched == 0 {
        return None;
    }
    // Fail-safe margin, then clamp to the frame.
    x0 = (x0 - margin).max(0.0);
    y0 = (y0 - margin).max(0.0);
    x1 = (x1 + margin).min(1.0);
    y1 = (y1 + margin).min(1.0);
    Some(RedactRegion {
        cx: (x0 + x1) / 2.0,
        cy: (y0 + y1) / 2.0,
        w: x1 - x0,
        h: y1 - y0,
        categories: cats,
        matched,
        min_conf,
    })
}

/// PII pattern matching — the security core. Each `category()` arm names ONE
/// canonical secret class; the first match wins (most-specific first). Conservative
/// by construction so plain prose never trips it, but FAIL-SAFE on real secrets.
pub mod pii {
    use regex::Regex;
    use std::sync::OnceLock;

    macro_rules! re {
        ($cell:ident, $pat:expr) => {{
            static $cell: OnceLock<Regex> = OnceLock::new();
            $cell.get_or_init(|| Regex::new($pat).unwrap())
        }};
    }

    /// The PII category of a text line, or `None` if it looks benign. Order =
    /// specificity (a string that is both an email and a "secret" reports email).
    pub fn category(text: &str) -> Option<&'static str> {
        let t = text;
        // Email.
        if re!(EMAIL, r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").is_match(t) {
            return Some("email");
        }
        // AWS access-key id.
        if re!(AWS, r"\b(?:AKIA|ASIA|AROA|AIDA)[A-Z0-9]{16}\b").is_match(t) {
            return Some("aws_key");
        }
        // JWT (base64url header.payload.signature, header starts `eyJ`).
        if re!(
            JWT,
            r"\beyJ[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+"
        )
        .is_match(t)
        {
            return Some("jwt");
        }
        // Provider API-key / token prefixes (OpenAI sk-/pk-, GitHub ghp_/gho_…,
        // Slack xox?-, Google AIza…).
        if re!(
            APIKEY,
            r"(?:\b(?:sk|pk|rk)-[A-Za-z0-9]{8,})|(?:\bgh[pousr]_[A-Za-z0-9]{16,})|(?:\bxox[baprs]-[A-Za-z0-9\-]{10,})|(?:\bAIza[A-Za-z0-9_\-]{20,})"
        )
        .is_match(t)
        {
            return Some("api_key");
        }
        // US SSN.
        if re!(SSN, r"\b\d{3}-\d{2}-\d{4}\b").is_match(t) {
            return Some("ssn");
        }
        // IPv4.
        if re!(
            IPV4,
            r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)\b"
        )
        .is_match(t)
        {
            return Some("ip");
        }
        // Credit-card-like digit run (13–19 digits, optionally space/dash grouped) +
        // a Luhn check (kills phone numbers / ids that aren't cards).
        if let Some(m) = re!(CARD, r"\b\d[\d \-]{11,21}\d\b").find(t) {
            let digits: String = m.as_str().chars().filter(|c| c.is_ascii_digit()).collect();
            if (13..=19).contains(&digits.len()) && luhn(&digits) {
                return Some("credit_card");
            }
        }
        // A secret-bearing KEYWORD on the line (the value lives in this same box,
        // so we redact the whole line — fail-safe).
        let low = t.to_ascii_lowercase();
        if re!(
            KEYWORD,
            r"(?:password|passwd|secret|api[_ ]?key|private[_ ]?key|access[_ ]?token|bearer |credential)"
        )
        .is_match(&low)
        {
            return Some("keyword");
        }
        // Generic high-entropy token: ≥20 chars of key alphabet, with BOTH a letter
        // and a digit (so prose like "Helloworldthisisfine" — no digit — is spared).
        if let Some(m) = re!(GENERIC, r"\b[A-Za-z0-9_\-]{20,}\b").find(t) {
            let s = m.as_str();
            let has_alpha = s.chars().any(|c| c.is_ascii_alphabetic());
            let has_digit = s.chars().any(|c| c.is_ascii_digit());
            if has_alpha && has_digit {
                return Some("secret");
            }
        }
        None
    }

    /// Luhn checksum (mod-10) — credit-card validity.
    fn luhn(digits: &str) -> bool {
        let mut sum = 0u32;
        let mut alt = false;
        for c in digits.chars().rev() {
            let mut d = c.to_digit(10).unwrap_or(0);
            if alt {
                d *= 2;
                if d > 9 {
                    d -= 9;
                }
            }
            sum += d;
            alt = !alt;
        }
        sum.is_multiple_of(10)
    }

    #[cfg(test)]
    mod tests {
        use super::category;

        #[test]
        fn matches_real_secrets() {
            assert_eq!(category("Email:john@acme.com"), Some("email"));
            assert_eq!(category("contact me at a.b+x@sub.domain.co"), Some("email"));
            let provider_key = format!("APIkey:{}{}", "sk", "-abc123XYZ456def");
            assert_eq!(category(&provider_key), Some("api_key"));
            let github_token = format!("token {}{}", "ghp", "_ABCdef0123456789ABCdef");
            assert_eq!(category(&github_token), Some("api_key"));
            assert_eq!(category("AKIAIOSFODNN7EXAMPLE here"), Some("aws_key"));
            assert_eq!(category("Password: hunter2!"), Some("keyword"));
            assert_eq!(category("My SSN is 123-45-6789"), Some("ssn"));
            assert_eq!(category("host 203.0.113.42"), Some("ip"));
            // A Luhn-valid Visa test number.
            assert_eq!(category("card 4111 1111 1111 1111"), Some("credit_card"));
        }

        #[test]
        fn spares_benign_text() {
            assert_eq!(category("Helloworldthisisfine"), None);
            assert_eq!(category("The quick brown fox jumps"), None);
            assert_eq!(category("Chapter 12 introduction"), None);
            // A long digit run that FAILS Luhn is not flagged as a card.
            assert_eq!(category("order 1234567890123456 ok"), None);
            // A number that isn't a valid IPv4 octet set.
            assert_eq!(category("version 999.999.999.999"), None);
        }
    }
}
