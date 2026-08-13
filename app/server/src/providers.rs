//! providers.rs — pluggable asset providers.
//!
//! Role: search + fetch external/local media as PERMISSION-SCOPED providers,
//! importing results as normal project assets with the LICENSE + ATTRIBUTION
//! surfaced (and recorded). This is the concrete first cut of the tool/plugin
//! API: a provider is a scoped capability the agent/host calls through
//! the verb registry (`assets.providers` / `assets.search` / `assets.fetch`),
//! never a hosted catalog we run.
//!
//! First instances:
//!   - `local_folder` — filename substring search under a search dir; NO network,
//!      NO key; the hit id IS the absolute file path. NOTE: fetch reads that path
//!      directly and is NOT re-confined to the search dir — this is the SAME
//!      local-file-read capability `media.import {path}` already grants (the editor
//!      imports any local file the user points it at); it is NOT a network-reachable
//!      surface, so confining it here would be theater while media.import stays open.
//!   - `openverse`    — api.openverse.org, the Creative-Commons MEDIA AGGREGATOR
//!      (sources: Freesound, Jamendo, Wikimedia, …); KEYLESS (anonymous tier is
//!      rate-limited); license + attribution are first-class in its responses.
//!      We request `license_type=commercial` so results are commercially usable.
//!
//! Safety: `assets.fetch` re-RESOLVES the hit through the provider by id and
//! downloads ONLY the URL the provider returns — there is no CALLER-supplied URL
//! (so no caller-driven SSRF). The provider-returned URL (a third-party CC CDN)
//! is still treated defensively: `download_to` REJECTS internal/private hosts and
//! DISABLES redirects (so a hostile CC entry can't 302 the desktop app at an
//! internal address), and the download is size-capped. The import goes through
//! core's `record_import` + the import chain (the ONLY valid import path), so
//! receipts/replay stay intact.
//!
//! Primary callers: dispatch.rs (`assets_*` verb handlers) wrap these blocking
//! functions in `spawn_blocking`. Deps: ureq 3 (same client as matte), serde_json.

use cut_core::{error_codes, CutError};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The provider names `assets.search`/`assets.fetch` accept (display order).
pub const PROVIDERS: &[&str] = &[
    "local_folder",
    "openverse",
    "archive_org",
    "wikimedia",
    "nasa",
    "stickers",
];

/// Max bytes `assets.fetch` will download for a single provider asset (a stock
/// SFX/clip is small; this fences a pathological response). 200 MB.
const MAX_FETCH_BYTES: u64 = 200 * 1024 * 1024;

/// Openverse API base. Audio + images only (no video endpoint upstream).
const OPENVERSE_BASE: &str = "https://api.openverse.org/v1";

/// One search/resolve result, normalized across providers. All fields the UI /
/// agent needs to show a pick AND credit it. `download_url` is what `fetch`
/// pulls (a `file://`-style local path for `local_folder`, an http(s) URL for
/// `openverse`).
#[derive(Debug, Clone, Serialize)]
pub struct ProviderHit {
    /// Provider name (`local_folder` | `openverse`).
    pub provider: String,
    /// Stable id WITHIN the provider (Openverse uuid; absolute path for local).
    pub id: String,
    /// Human title / filename.
    pub title: String,
    /// Media kind: `audio` | `image` | `video`.
    pub kind: String,
    /// Creator/author when known (drives attribution).
    pub creator: Option<String>,
    /// Short license code (`cc0`, `cc-by`, `cc-by-sa`, …, or `local`).
    pub license: String,
    /// Canonical license URL when known.
    pub license_url: Option<String>,
    /// The provider's landing page for this item (the attribution backlink).
    pub source_url: Option<String>,
    /// The fetchable media URL (or absolute local path for `local_folder`).
    pub download_url: String,
    /// File extension/type when known (`mp3`, `wav`, `png`, …).
    pub filetype: Option<String>,
    /// Duration in ms for time-based media when known.
    pub duration_ms: Option<u64>,
    /// File size in bytes when known.
    pub filesize: Option<u64>,
    /// A ready-to-use one-line credit string.
    pub attribution: String,
    /// Whether the license REQUIRES displaying attribution (false for CC0 / PD /
    /// local files; true for CC-BY and friends).
    pub requires_attribution: bool,
}

impl ProviderHit {
    /// JSON for the verb result (serde derive gives the field shape).
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// Provider catalog for `assets.providers` — name + what it does + whether it
/// needs anything from the caller.
pub fn provider_info() -> Vec<Value> {
    vec![
        json!({
            "name": "local_folder",
            "kinds": ["audio", "image", "video"],
            "needs_key": false,
            "network": false,
            "note": "Search a user-designated folder by filename. Pass `dir` to assets.search; the hit id is the absolute path. No network.",
        }),
        json!({
            "name": "openverse",
            "kinds": ["audio", "image"],
            "needs_key": false,
            "network": true,
            "note": "Creative-Commons media aggregator (Freesound/Jamendo/Wikimedia). Keyless (rate-limited). Results are commercial-use; license + attribution are surfaced and recorded on fetch.",
        }),
        json!({
            "name": "archive_org",
            "kinds": ["video", "audio"],
            "needs_key": false,
            "network": true,
            "note": "Internet Archive (archive.org) — vast public-domain + CC MOVING-image + audio archive. Keyless. mediatype:movies/audio; license parsed from the item's licenseurl (many are PD/CC0). The best video/audio file is resolved at fetch.",
        }),
        json!({
            "name": "wikimedia",
            "kinds": ["video", "image", "audio"],
            "needs_key": false,
            "network": true,
            "note": "Wikimedia Commons — PD/CC media (File: namespace). Keyless API; license + author from the file's extmetadata. Video = webm/ogv.",
        }),
        json!({
            "name": "nasa",
            "kinds": ["video", "image"],
            "needs_key": false,
            "network": true,
            "note": "NASA Image and Video Library (images-api.nasa.gov). Keyless. ALL NASA media is PUBLIC DOMAIN (credit appreciated, not required). The mp4 rendition is resolved at fetch.",
        }),
        json!({
            "name": "stickers",
            "kinds": ["image"],
            "needs_key": false,
            "network": false,
            "note": "Built-in overlay STICKER shapes (arrows, star, heart, check, X, circle, speech bubble, play, pin, burst, plus). OFFLINE — rendered on the fly from a bundled SVG catalog to transparent PNGs (resvg). CC0 / public domain. Place on an overlay video track + edit.transform to size/position.",
        }),
    ]
}

/// True when `license` does NOT require crediting (CC0 / public domain / local).
fn license_is_attribution_free(license: &str) -> bool {
    matches!(license, "cc0" | "pdm" | "pdmark" | "public" | "local")
}

/// Build a one-line credit string from the parts.
fn attribution_line(title: &str, creator: Option<&str>, license: &str, source: &str) -> String {
    let lic = license.to_uppercase();
    match creator {
        Some(c) if !c.is_empty() => format!("\"{title}\" by {c} — {lic} (via {source})"),
        _ => format!("\"{title}\" — {lic} (via {source})"),
    }
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Search a provider. `kind` ∈ {audio,image,video} (provider may not support
/// all). `dir` is required for `local_folder`. Blocking (network/fs) — call from
/// `spawn_blocking`.
pub fn search(
    provider: &str,
    q: &str,
    kind: &str,
    limit: usize,
    dir: Option<&str>,
) -> Result<Vec<ProviderHit>, CutError> {
    let limit = limit.clamp(1, 50);
    match provider {
        "local_folder" => {
            let dir = dir.ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    "local_folder requires `dir`",
                    "pass `dir` (an absolute folder path) to assets.search",
                )
            })?;
            local_search(q, kind, Path::new(dir), limit)
        }
        "openverse" => openverse_search(q, kind, limit),
        "archive_org" => archive_search(q, kind, limit),
        "wikimedia" => wikimedia_search(q, kind, limit),
        "nasa" => nasa_search(q, kind, limit),
        "stickers" => stickers_search(q, kind, limit),
        other => Err(unknown_provider(other)),
    }
}

/// Resolve one hit by id (the authoritative download URL + license). For
/// `local_folder` the id is the path. Blocking — call from `spawn_blocking`.
pub fn resolve(provider: &str, id: &str, kind: &str) -> Result<ProviderHit, CutError> {
    match provider {
        "local_folder" => local_resolve(id),
        "openverse" => openverse_resolve(id, kind),
        "archive_org" => archive_resolve(id, kind),
        "wikimedia" => wikimedia_resolve(id, kind),
        "nasa" => nasa_resolve(id, kind),
        "stickers" => stickers_resolve(id),
        other => Err(unknown_provider(other)),
    }
}

fn unknown_provider(name: &str) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        format!("unknown provider '{name}'"),
        format!("provider must be one of: {}", PROVIDERS.join(", ")),
    )
}

// ---------------------------------------------------------------------------
// local_folder
// ---------------------------------------------------------------------------

/// Media extensions per kind (lowercase, no dot).
fn kind_exts(kind: &str) -> &'static [&'static str] {
    match kind {
        "image" => &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tiff"],
        "video" => &["mp4", "mov", "mkv", "webm", "avi", "m4v"],
        // default audio
        _ => &["mp3", "wav", "flac", "aac", "ogg", "m4a", "opus", "aiff"],
    }
}

/// Classify a file extension into a kind, or None if not a known media type.
fn ext_kind(ext: &str) -> Option<&'static str> {
    let e = ext.to_lowercase();
    for k in ["audio", "image", "video"] {
        if kind_exts(k).contains(&e.as_str()) {
            return Some(match k {
                "image" => "image",
                "video" => "video",
                _ => "audio",
            });
        }
    }
    None
}

/// Walk `dir` (depth ≤ 3) for files whose name contains `q` (case-insensitive)
/// and whose extension matches `kind`. Returns up to `limit` hits.
fn local_search(
    q: &str,
    kind: &str,
    dir: &Path,
    limit: usize,
) -> Result<Vec<ProviderHit>, CutError> {
    if !dir.is_dir() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("folder not found: {}", dir.display()),
            "pass `dir` (an existing folder)",
        ));
    }
    let exts = kind_exts(kind);
    let needle = q.to_lowercase();
    let mut hits = Vec::new();
    // Iterative shallow walk (depth ≤ 3) — avoids unbounded recursion on deep trees.
    let mut stack: Vec<(PathBuf, u32)> = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let rd = match std::fs::read_dir(&d) {
            Ok(rd) => rd,
            Err(_) => continue, // unreadable dir → skip, not fatal
        };
        for ent in rd.flatten() {
            let p = ent.path();
            if p.is_dir() {
                if depth < 3 {
                    stack.push((p, depth + 1));
                }
                continue;
            }
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            if !exts.contains(&ext.as_str()) {
                continue;
            }
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if !needle.is_empty() && !name.to_lowercase().contains(&needle) {
                continue;
            }
            let abs = p.canonicalize().unwrap_or(p.clone());
            hits.push(ProviderHit {
                provider: "local_folder".into(),
                id: abs.display().to_string(),
                title: name.clone(),
                kind: ext_kind(&ext).unwrap_or("audio").into(),
                creator: None,
                license: "local".into(),
                license_url: None,
                source_url: None,
                download_url: abs.display().to_string(),
                filetype: Some(ext),
                duration_ms: None,
                filesize: ent.metadata().ok().map(|m| m.len()),
                attribution: format!("local file: {name}"),
                requires_attribution: false,
            });
            if hits.len() >= limit {
                return Ok(hits);
            }
        }
    }
    Ok(hits)
}

/// Resolve a local id (the path itself).
fn local_resolve(id: &str) -> Result<ProviderHit, CutError> {
    let p = PathBuf::from(id);
    if !p.is_file() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("file not found: {id}"),
            "the local_folder id must be an existing file path (from assets.search)",
        ));
    }
    let abs = p.canonicalize().unwrap_or(p.clone());
    let ext = abs
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    let Some(kind) = ext_kind(&ext) else {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("local_folder item is not a supported media file: {}", abs.display()),
            format!(
                "extension '.{ext}' is not one of the known audio/image/video media extensions"
            ),
        )
        .with_suggested_action("use assets.search for local_folder hits, or import arbitrary files through media.import where ffprobe validates them"));
    };
    let name = abs
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    Ok(ProviderHit {
        provider: "local_folder".into(),
        id: abs.display().to_string(),
        title: name.clone(),
        kind: kind.into(),
        creator: None,
        license: "local".into(),
        license_url: None,
        source_url: None,
        download_url: abs.display().to_string(),
        filetype: Some(ext),
        duration_ms: None,
        filesize: std::fs::metadata(&abs).ok().map(|m| m.len()),
        attribution: format!("local file: {name}"),
        requires_attribution: false,
    })
}

// ---------------------------------------------------------------------------
// openverse
// ---------------------------------------------------------------------------

/// Map our `kind` to the Openverse endpoint segment (audio | images).
fn openverse_endpoint(kind: &str) -> Result<&'static str, CutError> {
    match kind {
        "audio" => Ok("audio"),
        "image" => Ok("images"),
        other => Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("openverse does not support kind '{other}'"),
            "openverse kind must be audio or image",
        )),
    }
}

/// Build the per-request User-Agent (Openverse asks clients to identify).
fn user_agent() -> String {
    format!(
        "shellx-cut/{} (+https://theshellx.com)",
        env!("CARGO_PKG_VERSION")
    )
}

/// GET an Openverse URL → parsed JSON. Maps transport/HTTP errors (incl. 429
/// rate-limit) to actionable CutErrors.
fn openverse_get(url: &str) -> Result<Value, CutError> {
    let resp = ureq::get(url)
        .header("User-Agent", &user_agent())
        .header("Accept", "application/json")
        .call();
    let mut resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(429)) => {
            return Err(CutError::new(
                error_codes::IO,
                "openverse rate-limited (HTTP 429)",
                "the keyless anonymous tier is throttled — retry shortly, or register an Openverse API key for higher limits",
            ))
        }
        Err(e) => {
            return Err(CutError::new(
                error_codes::IO,
                "openverse request failed",
                e.to_string(),
            ))
        }
    };
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| CutError::new(error_codes::IO, "openverse read failed", e.to_string()))?;
    serde_json::from_str(&body).map_err(|e| {
        CutError::new(
            error_codes::IO,
            "openverse returned non-JSON",
            e.to_string(),
        )
    })
}

/// Parse one Openverse result object into a ProviderHit.
fn openverse_hit(v: &Value, kind: &str) -> Option<ProviderHit> {
    let id = v.get("id")?.as_str()?.to_string();
    let url = v.get("url")?.as_str()?.to_string();
    let title = v
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or("untitled")
        .to_string();
    let creator = v.get("creator").and_then(|x| x.as_str()).map(String::from);
    let license = v
        .get("license")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();
    let source = v
        .get("source")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("provider").and_then(|x| x.as_str()))
        .unwrap_or("openverse")
        .to_string();
    let source_url = v
        .get("foreign_landing_url")
        .and_then(|x| x.as_str())
        .map(String::from);
    let filetype = v.get("filetype").and_then(|x| x.as_str()).map(String::from);
    // Openverse audio `duration` is in milliseconds.
    let duration_ms = v.get("duration").and_then(|x| x.as_u64());
    let filesize = v.get("filesize").and_then(|x| x.as_u64());
    let attribution = attribution_line(&title, creator.as_deref(), &license, &source);
    Some(ProviderHit {
        provider: "openverse".into(),
        id,
        title,
        kind: kind.into(),
        creator,
        license_url: v
            .get("license_url")
            .and_then(|x| x.as_str())
            .map(String::from),
        requires_attribution: !license_is_attribution_free(&license),
        license,
        source_url,
        download_url: url,
        filetype,
        duration_ms,
        filesize,
        attribution,
    })
}

fn openverse_search(q: &str, kind: &str, limit: usize) -> Result<Vec<ProviderHit>, CutError> {
    if q.trim().is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "search query is empty",
            "pass a non-empty `q`",
        ));
    }
    let ep = openverse_endpoint(kind)?;
    let url = format!(
        "{OPENVERSE_BASE}/{ep}/?q={}&page_size={}&license_type=commercial&mature=false",
        urlencode(q),
        limit
    );
    let v = openverse_get(&url)?;
    let results = v
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(results
        .iter()
        .filter_map(|r| openverse_hit(r, kind))
        .collect())
}

fn openverse_resolve(id: &str, kind: &str) -> Result<ProviderHit, CutError> {
    let ep = openverse_endpoint(kind)?;
    let url = format!("{OPENVERSE_BASE}/{ep}/{id}/");
    let v = openverse_get(&url)?;
    openverse_hit(&v, kind).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("openverse item '{id}' not found or missing a download url"),
            "re-run assets.search to get a valid id (and the right `kind`)",
        )
    })
}

/// Minimal percent-encoding for a query string value (RFC 3986 unreserved kept).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Public-domain / CC motion-footage providers: Archive.org, Wikimedia, NASA.
// Each is a keyless REST adapter. `search` lists hits (id + metadata); `resolve`
// does the per-item file lookup that yields the authoritative download URL — so a
// search stays one request, and the (heavier) file resolution happens only on the
// hit the user actually fetches. download_to() keeps the SSRF fence.
// ---------------------------------------------------------------------------

/// GET a URL → parsed JSON, with an actionable error labelled by `who`. The shared
/// blocking GET for the network providers (mirrors `openverse_get`).
fn http_json(url: &str, who: &str) -> Result<Value, CutError> {
    let resp = ureq::get(url)
        .header("User-Agent", &user_agent())
        .header("Accept", "application/json")
        .call();
    let mut resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(429)) => {
            return Err(CutError::new(
                error_codes::IO,
                format!("{who} rate-limited (HTTP 429)"),
                "the keyless tier is throttled — retry shortly",
            ))
        }
        Err(e) => {
            return Err(CutError::new(
                error_codes::IO,
                format!("{who} request failed"),
                e.to_string(),
            ))
        }
    };
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| CutError::new(error_codes::IO, format!("{who} read failed"), e.to_string()))?;
    serde_json::from_str(&body).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("{who} returned non-JSON"),
            e.to_string(),
        )
    })
}

/// Map a license URL (or short name) to a (short code, requires_attribution).
/// Recognizes CC0 / public-domain mark / CC-BY(-SA/-NC/-ND) families.
fn license_from_url(s: &str) -> (String, bool) {
    let l = s.to_lowercase();
    if l.contains("publicdomain/zero") || l.contains("cc0") {
        ("cc0".into(), false)
    } else if l.contains("publicdomain/mark") || l.contains("public domain") || l.contains("/pdm") {
        ("pdm".into(), false)
    } else if l.contains("by-sa") {
        ("cc-by-sa".into(), true)
    } else if l.contains("by-nc-sa") {
        ("cc-by-nc-sa".into(), true)
    } else if l.contains("by-nd") {
        ("cc-by-nd".into(), true)
    } else if l.contains("by-nc") {
        ("cc-by-nc".into(), true)
    } else if l.contains("licenses/by") || l.contains("cc by") || l.contains("cc-by") {
        ("cc-by".into(), true)
    } else if l.is_empty() {
        ("unknown".into(), true)
    } else {
        // An unrecognized short name — keep it, assume attribution to be safe.
        (s.trim().to_lowercase(), true)
    }
}

/// Strip simple HTML tags + collapse whitespace (Wikimedia `extmetadata` values
/// are HTML fragments, e.g. an `<a>`-wrapped author).
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The video/audio extensions (preference order) for picking the best file in a
/// multi-file item (Archive.org / NASA asset manifests).
fn pref_exts(kind: &str) -> &'static [&'static str] {
    match kind {
        "audio" => &["mp3", "ogg", "flac", "wav", "m4a"],
        // video (default): mp4 first (most compatible), then open formats.
        _ => &["mp4", "webm", "ogv", "mkv", "mov"],
    }
}

/// Pick the best file URL from candidate `(url, ext)` pairs for `kind` — the most
/// preferred extension wins. Returns (url, ext).
fn best_file<'a>(cands: &'a [(String, String)], kind: &str) -> Option<&'a (String, String)> {
    let prefs = pref_exts(kind);
    cands
        .iter()
        .min_by_key(|(_, ext)| {
            prefs
                .iter()
                .position(|p| p == &ext.to_lowercase())
                .unwrap_or(usize::MAX)
        })
        .filter(|(_, ext)| prefs.contains(&ext.to_lowercase().as_str()))
}

// ---- Archive.org -----------------------------------------------------------

fn archive_search(q: &str, kind: &str, limit: usize) -> Result<Vec<ProviderHit>, CutError> {
    if q.trim().is_empty() {
        return Err(empty_query());
    }
    let mediatype = if kind == "audio" { "audio" } else { "movies" };
    let url = format!(
        "https://archive.org/advancedsearch.php?q={}+AND+mediatype%3A{mediatype}\
         &fl[]=identifier&fl[]=title&fl[]=creator&fl[]=licenseurl&rows={}&page=1&output=json",
        urlencode(q),
        limit
    );
    let v = http_json(&url, "archive.org")?;
    let docs = v
        .pointer("/response/docs")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(docs
        .iter()
        .filter_map(|d| {
            let id = d.get("identifier")?.as_str()?.to_string();
            let title = d
                .get("title")
                .and_then(|x| x.as_str())
                .unwrap_or(&id)
                .to_string();
            // `creator` can be a string or an array of strings.
            let creator = d.get("creator").and_then(|x| {
                x.as_str().map(String::from).or_else(|| {
                    x.as_array()
                        .and_then(|a| a.first()?.as_str().map(String::from))
                })
            });
            let lic_url = d.get("licenseurl").and_then(|x| x.as_str()).unwrap_or("");
            let (license, req) = if lic_url.is_empty() {
                ("unknown".to_string(), true)
            } else {
                license_from_url(lic_url)
            };
            let source_url = Some(format!("https://archive.org/details/{id}"));
            let attribution = attribution_line(&title, creator.as_deref(), &license, "archive.org");
            Some(ProviderHit {
                provider: "archive_org".into(),
                id,
                title,
                kind: kind.into(),
                creator,
                license,
                license_url: (!lic_url.is_empty()).then(|| lic_url.to_string()),
                source_url,
                // Resolved at fetch (the file list needs a 2nd request).
                download_url: String::new(),
                filetype: None,
                duration_ms: None,
                filesize: None,
                attribution,
                requires_attribution: req,
            })
        })
        .collect())
}

fn archive_resolve(id: &str, kind: &str) -> Result<ProviderHit, CutError> {
    let v = http_json(&format!("https://archive.org/metadata/{id}"), "archive.org")?;
    let meta = v.get("metadata").cloned().unwrap_or(Value::Null);
    let title = meta
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or(id)
        .to_string();
    let creator = meta.get("creator").and_then(|x| {
        x.as_str().map(String::from).or_else(|| {
            x.as_array()
                .and_then(|a| a.first()?.as_str().map(String::from))
        })
    });
    let lic_url = meta
        .get("licenseurl")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let (license, req) = license_from_url(lic_url);
    // Build (downloadURL, ext) candidates from the file list.
    let files = v
        .get("files")
        .and_then(|f| f.as_array())
        .cloned()
        .unwrap_or_default();
    let cands: Vec<(String, String)> = files
        .iter()
        .filter_map(|f| {
            let name = f.get("name")?.as_str()?;
            let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
            Some((
                format!("https://archive.org/download/{id}/{}", urlencode(name)),
                ext,
            ))
        })
        .collect();
    let (download_url, filetype) = best_file(&cands, kind)
        .map(|(u, e)| (u.clone(), Some(e.clone())))
        .ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("archive.org item '{id}' has no downloadable {kind} file"),
                "try a different result, or kind:audio vs kind:video",
            )
        })?;
    let attribution = attribution_line(&title, creator.as_deref(), &license, "archive.org");
    Ok(ProviderHit {
        provider: "archive_org".into(),
        id: id.into(),
        title,
        kind: kind.into(),
        creator,
        license,
        license_url: (!lic_url.is_empty()).then(|| lic_url.to_string()),
        source_url: Some(format!("https://archive.org/details/{id}")),
        download_url,
        filetype,
        duration_ms: None,
        filesize: None,
        attribution,
        requires_attribution: req,
    })
}

// ---- Wikimedia Commons ------------------------------------------------------

/// One imageinfo page → ProviderHit (shared by search + resolve). Filters by the
/// requested `kind` via the file's MIME type.
fn wikimedia_hit(title: &str, ii: &Value, kind: &str) -> Option<ProviderHit> {
    let mime = ii.get("mime").and_then(|x| x.as_str()).unwrap_or("");
    let want = match kind {
        "video" => "video/",
        "audio" => "audio/",
        _ => "image/",
    };
    if !mime.starts_with(want) {
        return None;
    }
    let url = ii.get("url")?.as_str()?.to_string();
    let em = ii.get("extmetadata").cloned().unwrap_or(Value::Null);
    let get_em = |k: &str| {
        em.get(k)
            .and_then(|x| x.get("value"))
            .and_then(|x| x.as_str())
            .map(strip_html)
    };
    let lic_short = get_em("LicenseShortName").unwrap_or_default();
    let lic_url = get_em("LicenseUrl").unwrap_or_default();
    let (license, req) = license_from_url(if lic_url.is_empty() {
        &lic_short
    } else {
        &lic_url
    });
    let creator = get_em("Artist").filter(|s| !s.is_empty());
    let title_clean = title.strip_prefix("File:").unwrap_or(title).to_string();
    let filetype = url.rsplit('.').next().map(|e| e.to_lowercase());
    let attribution = attribution_line(
        &title_clean,
        creator.as_deref(),
        &license,
        "Wikimedia Commons",
    );
    Some(ProviderHit {
        provider: "wikimedia".into(),
        id: title.to_string(),
        title: title_clean,
        kind: kind.into(),
        creator,
        license,
        license_url: (!lic_url.is_empty()).then_some(lic_url),
        source_url: Some(format!(
            "https://commons.wikimedia.org/wiki/{}",
            urlencode(title)
        )),
        download_url: url,
        filetype,
        duration_ms: None,
        filesize: ii.get("size").and_then(|x| x.as_u64()),
        attribution,
        requires_attribution: req,
    })
}

fn wikimedia_search(q: &str, kind: &str, limit: usize) -> Result<Vec<ProviderHit>, CutError> {
    if q.trim().is_empty() {
        return Err(empty_query());
    }
    // generator=search over the File: namespace (6), pulling imageinfo in one call.
    // BIAS the CirrusSearch toward the requested media type with `filetype:` — a
    // bare "earth" query returns mostly images that the mime filter would drop.
    let ftype = match kind {
        "video" => "video",
        "audio" => "audio",
        _ => "bitmap",
    };
    let url = format!(
        "https://commons.wikimedia.org/w/api.php?action=query&format=json&generator=search\
         &gsrsearch={}&gsrnamespace=6&gsrlimit={}&prop=imageinfo\
         &iiprop=url%7Csize%7Cmime%7Cextmetadata",
        urlencode(&format!("{q} filetype:{ftype}")),
        limit
    );
    let v = http_json(&url, "Wikimedia")?;
    let pages = v.pointer("/query/pages").and_then(|p| p.as_object());
    let Some(pages) = pages else {
        return Ok(vec![]);
    };
    Ok(pages
        .values()
        .filter_map(|p| {
            let title = p.get("title")?.as_str()?;
            let ii = p.get("imageinfo")?.as_array()?.first()?;
            wikimedia_hit(title, ii, kind)
        })
        .collect())
}

fn wikimedia_resolve(id: &str, kind: &str) -> Result<ProviderHit, CutError> {
    // `id` is the File: title. Re-query imageinfo for the authoritative url.
    let url = format!(
        "https://commons.wikimedia.org/w/api.php?action=query&format=json&titles={}\
         &prop=imageinfo&iiprop=url%7Csize%7Cmime%7Cextmetadata",
        urlencode(id)
    );
    let v = http_json(&url, "Wikimedia")?;
    let pages = v.pointer("/query/pages").and_then(|p| p.as_object());
    pages
        .and_then(|pages| {
            pages.values().find_map(|p| {
                let title = p.get("title")?.as_str()?;
                let ii = p.get("imageinfo")?.as_array()?.first()?;
                wikimedia_hit(title, ii, kind)
            })
        })
        .ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("wikimedia file '{id}' not found or not a {kind}"),
                "re-run assets.search to get a valid File: id and the right kind",
            )
        })
}

// ---- NASA Image and Video Library ------------------------------------------

fn nasa_search(q: &str, kind: &str, limit: usize) -> Result<Vec<ProviderHit>, CutError> {
    if q.trim().is_empty() {
        return Err(empty_query());
    }
    let media = if kind == "image" { "image" } else { "video" };
    let url = format!(
        "https://images-api.nasa.gov/search?q={}&media_type={media}&page_size={}",
        urlencode(q),
        limit
    );
    let v = http_json(&url, "NASA")?;
    let items = v
        .pointer("/collection/items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(items
        .iter()
        .filter_map(|it| {
            let d = it.get("data")?.as_array()?.first()?;
            let id = d.get("nasa_id")?.as_str()?.to_string();
            let title = d
                .get("title")
                .and_then(|x| x.as_str())
                .unwrap_or(&id)
                .to_string();
            // The contributing center is the natural "creator" (PD, credit optional).
            let creator = d.get("center").and_then(|x| x.as_str()).map(String::from);
            let attribution = attribution_line(&title, creator.as_deref(), "public", "NASA");
            Some(ProviderHit {
                provider: "nasa".into(),
                id,
                title,
                kind: kind.into(),
                creator,
                license: "public".into(),
                license_url: Some(
                    "https://www.nasa.gov/nasa-brand-center/images-and-media/".into(),
                ),
                source_url: it.get("href").and_then(|x| x.as_str()).map(String::from),
                download_url: String::new(), // resolved via the asset manifest
                filetype: None,
                duration_ms: None,
                filesize: None,
                attribution,
                requires_attribution: false, // PUBLIC DOMAIN
            })
        })
        .collect())
}

fn nasa_resolve(id: &str, kind: &str) -> Result<ProviderHit, CutError> {
    let v = http_json(
        &format!("https://images-api.nasa.gov/asset/{}", urlencode(id)),
        "NASA",
    )?;
    let items = v
        .pointer("/collection/items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();
    // The asset manifest is a flat list of {href} file urls. Pick the best by ext.
    let cands: Vec<(String, String)> = items
        .iter()
        .filter_map(|it| {
            let href = it.get("href")?.as_str()?.to_string();
            let ext = href
                .split('?')
                .next()
                .unwrap_or(&href)
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_lowercase();
            Some((href, ext))
        })
        .collect();
    let (download_url, filetype) = best_file(&cands, kind)
        .map(|(u, e)| (u.clone(), Some(e.clone())))
        .ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("NASA asset '{id}' has no downloadable {kind} file"),
                "try a different result",
            )
        })?;
    // Prefer HTTPS (some manifests list http: variants → the SSRF/transport layer
    // is happier on https, and NASA serves both). NASA hrefs embed the nasa_id in
    // the path with RAW SPACES (e.g. `…/Moon and Saturn~orig.mp4`) + other unsafe
    // chars → the http client rejects them; percent-encode the unsafe set.
    let download_url = sanitize_url(&download_url.replacen("http://", "https://", 1));
    let attribution = attribution_line(id, None, "public", "NASA");
    Ok(ProviderHit {
        provider: "nasa".into(),
        id: id.into(),
        title: id.into(),
        kind: kind.into(),
        creator: None,
        license: "public".into(),
        license_url: Some("https://www.nasa.gov/nasa-brand-center/images-and-media/".into()),
        source_url: Some(format!("https://images.nasa.gov/details/{}", urlencode(id))),
        download_url,
        filetype,
        duration_ms: None,
        filesize: None,
        attribution,
        requires_attribution: false,
    })
}

/// Percent-encode the characters an http URI rejects (space + the RFC-3986
/// "unsafe" set) while leaving the URL STRUCTURE intact (scheme, `/`, `?`, `&`,
/// `=`, `:`, `%`, `#`). Used on provider-returned URLs that embed raw spaces in the
/// path (NASA's manifest hrefs). Already-encoded `%XX` sequences pass through (we
/// don't touch `%`), so it's idempotent for well-formed URLs.
fn sanitize_url(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for c in url.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '"' | '<' | '>' | '\\' | '^' | '`' | '{' | '|' | '}' => {
                out.push_str(&format!("%{:02X}", c as u32));
            }
            _ => out.push(c),
        }
    }
    out
}

/// Shared "empty query" error.
fn empty_query() -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "search query is empty",
        "pass a non-empty `q`",
    )
}

// ---------------------------------------------------------------------------
// Download (openverse fetch)
// ---------------------------------------------------------------------------

/// Extract the lowercased host from a URL parsed by the same `http` crate ureq
/// uses. This correctly handles bracketed IPv6 authorities.
#[cfg(test)]
fn url_host(url: &str) -> Option<String> {
    let uri: ureq::http::Uri = url.parse().ok()?;
    Some(
        uri.authority()?
            .host()
            .trim_matches(['[', ']'])
            .to_ascii_lowercase(),
    )
}

/// True for an address outside the public-unicast destination policy.
///
/// `IpAddr::is_global` is still unstable on the supported toolchain. Keep this
/// stable predicate in sync with Rust 1.94's special-use tables, and additionally
/// reject multicast plus the well-known NAT64 prefix: neither is a public-unicast
/// origin for provider media.
fn ip_is_internal(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || o[0] == 0
                || (o[0] == 100 && (64..=127).contains(&o[1])) // 100.64/10 shared
                // IETF protocol assignment. .9 and .10 are explicitly globally
                // reachable and remain public-unicast controls.
                || (o[0] == 192
                    && o[1] == 0
                    && o[2] == 0
                    && o[3] != 9
                    && o[3] != 10)
                || (o[0] == 192 && o[1] == 0 && o[2] == 2) // documentation
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19)) // benchmarking
                || (o[0] == 198 && o[1] == 51 && o[2] == 100) // documentation
                || (o[0] == 203 && o[1] == 0 && o[2] == 113) // documentation
                || o[0] >= 224 // multicast, reserved, and limited broadcast
        }
        IpAddr::V6(v6) => {
            if v6.to_ipv4_mapped().is_some() {
                // IPv4-mapped IPv6 is special-use even where its mapped IPv4
                // value would otherwise be public; provider DNS must return a
                // native public-unicast destination.
                return true;
            }
            let s = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // IPv4-compatible IPv6 is deprecated special-use space.
                || matches!(s, [0, 0, 0, 0, 0, 0, _, _])
                // IPv4/IPv6 translation prefixes.
                || matches!(s, [0x64, 0xff9b, 0, 0, 0, 0, _, _])
                || matches!(s, [0x64, 0xff9b, 1, _, _, _, _, _])
                || matches!(s, [0x100, 0, 0, 0, _, _, _, _]) // discard-only
                // IETF protocol assignments (2001::/23), except the ranges
                // Rust 1.94 marks globally reachable.
                || (matches!(s, [0x2001, b, _, _, _, _, _, _] if b < 0x200)
                    && !(matches!(s, [0x2001, 1, 0, 0, 0, 0, 0, 1 | 2])
                        || matches!(s, [0x2001, 3, _, _, _, _, _, _])
                        || matches!(s, [0x2001, 4, 0x112, _, _, _, _, _])
                        || matches!(s, [0x2001, b, _, _, _, _, _, _] if (0x20..=0x3f).contains(&b))))
                || matches!(s, [0x2002, _, _, _, _, _, _, _]) // 6to4
                || matches!(s, [0x2001, 0x0db8, _, _, _, _, _, _]) // documentation
                || matches!(s, [0x3fff, b, _, _, _, _, _, _] if (b & 0xf000) == 0) // documentation
                || matches!(s, [0x5f00, _, _, _, _, _, _, _]) // SRv6 SID
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || (s[0] & 0xffc0) == 0xfec0 // deprecated site-local
        }
    }
}

/// Parse one inet_aton numeric part: decimal, `0x`-hex, or leading-`0` octal.
fn aton_part(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(h, 16).ok()
    } else if s.len() > 1 && s.starts_with('0') {
        u64::from_str_radix(&s[1..], 8).ok()
    } else {
        s.parse::<u64>().ok()
    }
}

/// Canonicalize an OBFUSCATED IPv4 literal — decimal (`2130706433`), hex
/// (`0x7f000001`), octal (`017700000001`), or short forms (`127.1`, `a.b.c`) —
/// to its address, inet_aton style. Returns None for a real hostname. H2 fix:
/// these forms are rejected by `IpAddr::parse` but the OS resolver maps them to
/// the internal address, so the guard canonicalizes + range-checks them itself
/// instead of trusting the string.
fn canonical_ipv4(host: &str) -> Option<std::net::Ipv4Addr> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let nums: Vec<u64> = parts.iter().map(|p| aton_part(p)).collect::<Option<_>>()?;
    let addr: u32 = match nums.as_slice() {
        [a] => u32::try_from(*a).ok()?,
        [a, b] => {
            if *a > 255 || *b > 0x00ff_ffff {
                return None;
            }
            ((*a as u32) << 24) | (*b as u32)
        }
        [a, b, c] => {
            if *a > 255 || *b > 255 || *c > 0xffff {
                return None;
            }
            ((*a as u32) << 24) | ((*b as u32) << 16) | (*c as u32)
        }
        [a, b, c, d] => {
            if nums.iter().any(|x| *x > 255) {
                return None;
            }
            ((*a as u32) << 24) | ((*b as u32) << 16) | ((*c as u32) << 8) | (*d as u32)
        }
        _ => return None,
    };
    Some(std::net::Ipv4Addr::from(addr))
}

fn non_public_download_url() -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "refusing to download from an internal/private host",
        "the provider returned a non-public download URL",
    )
}

/// Resolve one provider host once, reject every non-public-unicast answer, and
/// retain the exact accepted socket addresses for the subsequent connection.
fn vetted_socket_addrs(host: &str, port: u16) -> Result<Vec<std::net::SocketAddr>, CutError> {
    if host.is_empty()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
    {
        return Err(non_public_download_url());
    }

    let addrs = if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        vec![std::net::SocketAddr::new(ip, port)]
    } else if let Some(v4) = canonical_ipv4(host) {
        vec![std::net::SocketAddr::new(std::net::IpAddr::V4(v4), port)]
    } else {
        use std::net::ToSocketAddrs;
        (host, port)
            .to_socket_addrs()
            .map_err(|_| non_public_download_url())?
            .collect()
    };

    if addrs.is_empty() || addrs.iter().any(|addr| ip_is_internal(addr.ip())) {
        return Err(non_public_download_url());
    }

    // ureq's `ResolvedSocketAddrs` holds at most 16 addresses. Every DNS result
    // above was vetted before limiting the list it will be permitted to connect.
    Ok(addrs.into_iter().take(16).collect())
}

/// Test-only host guard; production downloads use `vetted_download_target` to
/// retain accepted addresses rather than resolving again in ureq.
#[cfg(test)]
fn host_is_internal(host: &str) -> bool {
    vetted_socket_addrs(host, 80).is_err()
}

#[derive(Debug)]
struct PinnedResolver {
    host: String,
    port: u16,
    addrs: Vec<std::net::SocketAddr>,
}

impl PinnedResolver {
    fn new(host: String, port: u16, addrs: Vec<std::net::SocketAddr>) -> Self {
        Self { host, port, addrs }
    }

    fn matches_uri(&self, uri: &ureq::http::Uri) -> bool {
        let Some(authority) = uri.authority() else {
            return false;
        };
        let port = authority.port_u16().or_else(|| match uri.scheme_str() {
            Some("http") => Some(80),
            Some("https") => Some(443),
            _ => None,
        });
        authority
            .host()
            .trim_matches(['[', ']'])
            .eq_ignore_ascii_case(&self.host)
            && port == Some(self.port)
    }

    fn addresses_for_uri(
        &self,
        uri: &ureq::http::Uri,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        if !self.matches_uri(uri) {
            return Err(ureq::Error::HostNotFound);
        }
        let mut out = <Self as ureq::unversioned::resolver::Resolver>::empty(self);
        for addr in &self.addrs {
            out.push(*addr);
        }
        Ok(out)
    }
}

impl ureq::unversioned::resolver::Resolver for PinnedResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        _config: &ureq::config::Config,
        _timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        self.addresses_for_uri(uri)
    }
}

#[derive(Debug)]
struct VettedDownloadTarget {
    host: String,
    port: u16,
    addrs: Vec<std::net::SocketAddr>,
}

fn vetted_download_target(url: &str) -> Result<VettedDownloadTarget, CutError> {
    let uri: ureq::http::Uri = url.parse().map_err(|_| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "refusing to download from an unparseable host",
            "the provider returned a URL without a public host",
        )
    })?;
    let scheme = uri.scheme_str();
    if !matches!(scheme, Some("http") | Some("https")) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "refusing to download a non-http(s) url",
            format!("got '{url}'"),
        ));
    }
    let Some(authority) = uri.authority() else {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "refusing to download from an unparseable host",
            "the provider returned a URL without a public host",
        ));
    };
    let host = authority
        .host()
        .trim_matches(['[', ']'])
        .to_ascii_lowercase();
    let port = authority
        .port_u16()
        .unwrap_or(if scheme == Some("https") { 443 } else { 80 });
    let addrs = vetted_socket_addrs(&host, port)?;
    Ok(VettedDownloadTarget { host, port, addrs })
}

/// Download `url` to `dest`, size-capped at [`MAX_FETCH_BYTES`]. Streams the body
/// so a pathological response can't balloon memory. SSRF-fenced: http(s) only,
/// internal/private hosts rejected, redirects DISABLED. Returns bytes written.
pub fn download_to(url: &str, dest: &Path) -> Result<u64, CutError> {
    let target = vetted_download_target(url)?;
    // Connect only to the addresses accepted above. The request URI remains
    // unchanged, so HTTPS still sends the hostname as SNI and verifies its
    // certificate against that hostname. Proxies are disabled because a CONNECT
    // proxy would otherwise resolve the provider hostname independently.
    let config = ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .build();
    let agent = ureq::Agent::with_parts(
        config,
        ureq::unversioned::transport::DefaultConnector::default(),
        PinnedResolver::new(target.host, target.port, target.addrs),
    );
    // Redirects stay disabled so a redirect cannot introduce a second origin.
    let resp = agent
        .get(url)
        .header("User-Agent", &user_agent())
        .call()
        .map_err(|e| CutError::new(error_codes::IO, "asset download failed", e.to_string()))?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CutError::new(error_codes::IO, "create asset dir", e.to_string()))?;
    }
    let mut reader = resp.into_body().into_reader();
    let mut file = std::fs::File::create(dest)
        .map_err(|e| CutError::new(error_codes::IO, "create asset file", e.to_string()))?;
    // Bounded copy: take MAX+1 so we can detect an over-cap response.
    let mut limited = std::io::Read::take(&mut reader, MAX_FETCH_BYTES + 1);
    let written = std::io::copy(&mut limited, &mut file)
        .map_err(|e| CutError::new(error_codes::IO, "write asset file", e.to_string()))?;
    if written > MAX_FETCH_BYTES {
        let _ = std::fs::remove_file(dest);
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "asset exceeds the fetch size cap",
            format!("> {} MB", MAX_FETCH_BYTES / (1024 * 1024)),
        ));
    }
    Ok(written)
}

// ---------------------------------------------------------------------------
// Stickers — built-in offline overlay shapes.
// ---------------------------------------------------------------------------
//
// A bundled catalog of simple, recognizable overlay SHAPES (arrows, star, heart,
// check, X, …) rendered ON DEMAND from SVG to a transparent-background PNG via
// resvg (cut_media::mask::render_svg_png). No network, no hosting — the Openverse
// "no model/asset hosted" philosophy applied to vector stickers. CC0 / public
// domain (they are trivial geometry generated here). Each is a 512×512 transparent
// PNG; the user places it on an overlay video track and sizes/positions it with
// edit.transform. `search` matches the query against id/title/tags; `resolve`
// rasterizes the shape to a per-id cache file and returns it as a local path
// (assets.fetch then imports it in place, exactly like local_folder).

/// One sticker shape: stable id, human title, space-separated search tags, and the
/// inner SVG markup (drawn on a transparent 512×512 canvas).
struct Sticker {
    id: &'static str,
    title: &'static str,
    tags: &'static str,
    body: &'static str,
}

/// The bundled sticker catalog. Shapes use tasteful default fills + a visible
/// outline so they read on any footage; recolor later by re-generating is a future
/// nicety (v1 ships fixed palettes).
fn sticker_catalog() -> &'static [Sticker] {
    &[
        Sticker {
            id: "arrow_right",
            title: "Arrow (right)",
            tags: "arrow right point direction next",
            body: "<polygon points='40,200 300,200 300,120 480,256 300,392 300,312 40,312' fill='#2D9CDB' stroke='#1B6FA0' stroke-width='8' stroke-linejoin='round'/>",
        },
        Sticker {
            id: "arrow_left",
            title: "Arrow (left)",
            tags: "arrow left point direction back previous",
            body: "<polygon points='472,200 212,200 212,120 32,256 212,392 212,312 472,312' fill='#2D9CDB' stroke='#1B6FA0' stroke-width='8' stroke-linejoin='round'/>",
        },
        Sticker {
            id: "star",
            title: "Star",
            tags: "star favorite rating gold sparkle",
            body: "<polygon points='256,32 322,196 498,200 358,312 408,484 256,386 104,484 154,312 14,200 190,196' fill='#F2C94C' stroke='#B8860B' stroke-width='10' stroke-linejoin='round'/>",
        },
        Sticker {
            id: "heart",
            title: "Heart",
            tags: "heart love like favorite",
            body: "<path d='M256 448C256 448 64 322 64 192C64 128 112 96 162 96C210 96 240 130 256 162C272 130 302 96 350 96C400 96 448 128 448 192C448 322 256 448 256 448Z' fill='#EB5757' stroke='#A33' stroke-width='8'/>",
        },
        Sticker {
            id: "circle",
            title: "Circle",
            tags: "circle dot ring round highlight",
            body: "<circle cx='256' cy='256' r='196' fill='none' stroke='#2F80ED' stroke-width='40'/>",
        },
        Sticker {
            id: "check",
            title: "Check mark",
            tags: "check tick correct yes done approve",
            body: "<polyline points='96,272 208,384 416,148' fill='none' stroke='#27AE60' stroke-width='60' stroke-linecap='round' stroke-linejoin='round'/>",
        },
        Sticker {
            id: "cross",
            title: "Cross (X)",
            tags: "cross x wrong no delete close cancel",
            body: "<path d='M144 144 L368 368 M368 144 L144 368' stroke='#EB5757' stroke-width='60' stroke-linecap='round'/>",
        },
        Sticker {
            id: "plus",
            title: "Plus",
            tags: "plus add new cross health",
            body: "<path d='M256 112 V400 M112 256 H400' stroke='#27AE60' stroke-width='60' stroke-linecap='round'/>",
        },
        Sticker {
            id: "speech_bubble",
            title: "Speech bubble",
            tags: "speech bubble chat talk comment callout",
            body: "<path d='M96 80 H416 a32 32 0 0 1 32 32 V296 a32 32 0 0 1 -32 32 H236 L150 414 V328 H96 a32 32 0 0 1 -32 -32 V112 a32 32 0 0 1 32 -32 Z' fill='#56CCF2' stroke='#2D9CDB' stroke-width='8' stroke-linejoin='round'/>",
        },
        Sticker {
            id: "play",
            title: "Play triangle",
            tags: "play triangle video start go",
            body: "<polygon points='168,108 168,404 416,256' fill='#333333' stroke='#000' stroke-width='8' stroke-linejoin='round'/>",
        },
        Sticker {
            id: "pin",
            title: "Location pin",
            tags: "pin location map marker place",
            body: "<path d='M256 56 C162 56 96 128 96 216 C96 320 256 468 256 468 C256 468 416 320 416 216 C416 128 350 56 256 56 Z' fill='#EB5757' stroke='#A33' stroke-width='8'/><circle cx='256' cy='212' r='56' fill='#FFFFFF'/>",
        },
        Sticker {
            id: "burst",
            title: "Starburst badge",
            tags: "burst badge new sale star sticker label",
            body: "<polygon points='256,36 300,168 440,148 352,260 472,332 332,352 360,492 256,402 152,492 180,352 40,332 160,260 72,148 212,168' fill='#F2994A' stroke='#C97B2E' stroke-width='8' stroke-linejoin='round'/>",
        },
    ]
}

/// Wrap a sticker body in a transparent 512×512 SVG document.
fn sticker_svg(body: &str) -> String {
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='512' height='512' \
         viewBox='0 0 512 512'>{body}</svg>"
    )
}

/// Build the ProviderHit for a sticker. `download_url` is the local PNG path when
/// rendered (resolve), else empty (search results — fetch renders on resolve).
fn sticker_hit(s: &Sticker, download_url: String) -> ProviderHit {
    ProviderHit {
        provider: "stickers".into(),
        id: s.id.into(),
        title: s.title.into(),
        kind: "image".into(),
        creator: None,
        license: "cc0".into(),
        license_url: Some("https://creativecommons.org/publicdomain/zero/1.0/".into()),
        source_url: None,
        download_url,
        filetype: Some("png".into()),
        duration_ms: None,
        filesize: None,
        attribution: attribution_line(s.title, None, "cc0", "ShellX Cut stickers"),
        requires_attribution: false,
    }
}

/// Search the built-in sticker catalog. Stickers are images; a non-image `kind`
/// yields no hits (so an audio/video slot won't pick a sticker). Empty query =
/// the whole catalog (browse).
fn stickers_search(q: &str, kind: &str, limit: usize) -> Result<Vec<ProviderHit>, CutError> {
    if !matches!(kind, "image" | "" | "any") {
        return Ok(vec![]);
    }
    let ql = q.trim().to_lowercase();
    let hits: Vec<ProviderHit> = sticker_catalog()
        .iter()
        .filter(|s| {
            ql.is_empty()
                || s.id.contains(&ql)
                || s.title.to_lowercase().contains(&ql)
                || s.tags.contains(&ql)
                || ql.split_whitespace().any(|w| s.tags.contains(w))
        })
        .take(limit)
        .map(|s| sticker_hit(s, String::new()))
        .collect();
    Ok(hits)
}

/// Resolve a sticker by id: rasterize its SVG to a per-id cache PNG (transparent
/// background) and return it as a local path — assets.fetch imports it in place.
fn stickers_resolve(id: &str) -> Result<ProviderHit, CutError> {
    let s = sticker_catalog()
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("unknown sticker '{id}'"),
                "list them with assets.search{provider:\"stickers\"}",
            )
        })?;
    let dir = std::env::temp_dir().join("shellx-cut-stickers");
    let out = dir.join(format!("{id}.png"));
    let svg = sticker_svg(s.body);
    cut_media::mask::render_svg_png(&svg, 512, 512, &out)?;
    Ok(sticker_hit(s, out.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_catalog_matches_registry() {
        let info = provider_info();
        let names: Vec<String> = info
            .iter()
            .map(|p| p["name"].as_str().unwrap().to_string())
            .collect();
        // The catalog and the PROVIDERS dispatch list must stay in lock-step.
        assert_eq!(
            names, PROVIDERS,
            "provider_info() must list every PROVIDERS entry"
        );
        assert!(names.contains(&"archive_org".to_string()));
        assert!(names.contains(&"wikimedia".to_string()));
        assert!(names.contains(&"nasa".to_string()));
    }

    /// License-URL → (code, requires_attribution) for the PD/CC families.
    #[test]
    fn license_url_maps_pd_and_cc() {
        assert_eq!(
            super::license_from_url("https://creativecommons.org/publicdomain/zero/1.0/"),
            ("cc0".to_string(), false)
        );
        assert_eq!(
            super::license_from_url("https://creativecommons.org/publicdomain/mark/1.0/"),
            ("pdm".to_string(), false)
        );
        assert_eq!(
            super::license_from_url("https://creativecommons.org/licenses/by-sa/4.0/"),
            ("cc-by-sa".to_string(), true)
        );
        assert_eq!(
            super::license_from_url("https://creativecommons.org/licenses/by/4.0/"),
            ("cc-by".to_string(), true)
        );
    }

    /// best_file picks the most-preferred extension (mp4 > webm) for video.
    #[test]
    fn best_file_prefers_mp4_for_video() {
        let cands = vec![
            ("a.ogv".to_string(), "ogv".to_string()),
            ("b.mp4".to_string(), "mp4".to_string()),
            ("c.txt".to_string(), "txt".to_string()),
        ];
        let pick = super::best_file(&cands, "video").unwrap();
        assert_eq!(pick.1, "mp4");
        // Nothing playable → None (a metadata-only item).
        let none = vec![("x.txt".to_string(), "txt".to_string())];
        assert!(super::best_file(&none, "video").is_none());
    }

    /// sanitize_url percent-encodes raw spaces (NASA hrefs) but keeps structure.
    #[test]
    fn sanitize_url_encodes_spaces_keeps_structure() {
        assert_eq!(
            super::sanitize_url("https://x.nasa.gov/video/Moon and Saturn/clip~orig.mp4"),
            "https://x.nasa.gov/video/Moon%20and%20Saturn/clip~orig.mp4"
        );
        // Idempotent on an already-clean URL.
        let clean = "https://a.org/b?x=1&y=2";
        assert_eq!(super::sanitize_url(clean), clean);
    }

    /// strip_html unwraps the HTML `extmetadata` author fragments.
    #[test]
    fn strip_html_unwraps_author() {
        assert_eq!(
            super::strip_html("<a href=\"/wiki/User:Jane\">Jane Doe</a>"),
            "Jane Doe"
        );
        assert_eq!(super::strip_html("Plain  Name"), "Plain Name");
    }

    #[test]
    fn ssrf_guard_blocks_internal_encodings_and_mapped() {
        // plain internal literals + names
        for h in [
            "127.0.0.1",
            "10.0.0.5",
            "172.16.0.2",
            "172.16.0.1",
            "169.254.169.254", // cloud metadata
            "100.64.0.1",      // CGNAT
            "0.0.0.0",
            "localhost",
            "foo.local",
            "x.internal",
            "::1",
        ] {
            assert!(host_is_internal(h), "{h} must be blocked");
        }
        // H2: obfuscated IPv4 literals IpAddr::parse rejects but the OS resolver
        // maps to 127.0.0.1 — now canonicalized + range-checked by the guard.
        for h in [
            "2130706433",
            "0x7f000001",
            "017700000001",
            "127.1",
            "127.0.1",
        ] {
            assert!(
                host_is_internal(h),
                "{h} (obfuscated 127.0.0.1) must be blocked"
            );
        }
        // H2: IPv4-mapped IPv6 — the old V6 branch missed these entirely.
        for h in [
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:10.0.0.1",
        ] {
            assert!(
                host_is_internal(h),
                "{h} (IPv4-mapped internal) must be blocked"
            );
        }
        // public IP literals are NOT internal (deterministic, no DNS path).
        for h in [
            "93.184.216.34",
            "8.8.8.8",
            "1.1.1.1",
            "2606:2800:220:1:248:1893:25c8:1946",
        ] {
            assert!(!host_is_internal(h), "{h} is public, must be allowed");
        }
    }

    #[test]
    fn ssrf_classifier_rejects_non_public_special_use_addresses() {
        // The provider downloader promises a public-unicast destination, not
        // merely a destination outside the common private ranges.
        for address in [
            "192.0.0.8",           // IETF protocol assignment
            "192.0.2.1",           // documentation
            "198.18.0.1",          // benchmarking
            "198.51.100.1",        // documentation
            "203.0.113.1",         // documentation
            "224.0.0.1",           // multicast
            "240.0.0.1",           // reserved
            "255.255.255.255",     // limited broadcast
            "64:ff9b::c000:201",   // IPv4/IPv6 translation
            "64:ff9b:1::c000:201", // IPv4/IPv6 translation
            "100::1",              // discard-only
            "2001:2::1",           // IETF protocol assignment
            "2001:db8::1",         // documentation
            "3fff::1",             // documentation
            "5f00::1",             // SRv6 SID
            "fec0::1",             // deprecated site-local
            "ff00::1",             // multicast
        ] {
            let ip = address.parse().unwrap();
            assert!(ip_is_internal(ip), "{address} must not be public-unicast");
        }
    }

    #[test]
    fn ssrf_host_guard_rejects_non_public_special_use_literals() {
        for host in ["198.18.0.1", "192.0.2.1", "64:ff9b:1::c000:201", "ff00::1"] {
            assert!(host_is_internal(host), "{host} must be blocked");
        }
    }

    #[test]
    fn ssrf_classifier_keeps_public_unicast_controls() {
        for address in [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.216.34",
            "192.0.0.9",
            "192.0.0.10",
            "2606:2800:220:1:248:1893:25c8:1946",
        ] {
            let ip = address.parse().unwrap();
            assert!(!ip_is_internal(ip), "{address} must remain public-unicast");
        }
        assert_eq!(
            vetted_socket_addrs("93.184.216.34", 443).unwrap(),
            vec!["93.184.216.34:443".parse().unwrap()]
        );
        let target = vetted_download_target("https://93.184.216.34/media.mp4").unwrap();
        assert_eq!(target.host, "93.184.216.34");
        assert_eq!(target.port, 443);
        assert_eq!(target.addrs, vec!["93.184.216.34:443".parse().unwrap()]);

        let ipv6_target =
            vetted_download_target("https://[2606:2800:220:1:248:1893:25c8:1946]/media.mp4")
                .unwrap();
        assert_eq!(ipv6_target.port, 443);
        assert_eq!(
            ipv6_target.addrs,
            vec!["[2606:2800:220:1:248:1893:25c8:1946]:443".parse().unwrap()]
        );
    }

    #[test]
    fn vetted_download_target_rejects_special_use_literal_before_connecting() {
        let err = vetted_download_target("https://198.18.0.1/media.mp4").unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_ARGS);
    }

    #[test]
    fn pinned_resolver_returns_only_vetted_addresses_for_original_authority() {
        let vetted: std::net::SocketAddr = "93.184.216.34:443".parse().unwrap();
        let resolver = PinnedResolver::new("cdn.example.invalid".to_string(), 443, vec![vetted]);
        let uri: ureq::http::Uri = "https://cdn.example.invalid/media.mp4".parse().unwrap();
        let resolved: Vec<_> = resolver
            .addresses_for_uri(&uri)
            .unwrap()
            .iter()
            .copied()
            .collect();
        assert_eq!(resolved, vec![vetted]);

        let wrong_authority: ureq::http::Uri =
            "https://other.example.invalid/media.mp4".parse().unwrap();
        assert!(resolver.addresses_for_uri(&wrong_authority).is_err());
    }

    #[test]
    fn canonical_ipv4_forms() {
        use std::net::Ipv4Addr;
        let lo = Ipv4Addr::new(127, 0, 0, 1);
        assert_eq!(canonical_ipv4("2130706433"), Some(lo)); // decimal
        assert_eq!(canonical_ipv4("0x7f000001"), Some(lo)); // hex
        assert_eq!(canonical_ipv4("017700000001"), Some(lo)); // octal
        assert_eq!(canonical_ipv4("127.1"), Some(lo)); // a.b short form
        assert_eq!(canonical_ipv4("127.0.1"), Some(lo)); // a.b.c short form
        assert_eq!(canonical_ipv4("example.com"), None); // a real hostname
        assert_eq!(canonical_ipv4("s3.amazonaws.com"), None);
        assert_eq!(canonical_ipv4("1.2.3.4.5"), None); // too many parts
    }

    #[test]
    fn attribution_and_license_logic() {
        assert!(license_is_attribution_free("cc0"));
        assert!(license_is_attribution_free("local"));
        assert!(!license_is_attribution_free("cc-by"));
        let a = attribution_line("Deep Whoosh", Some("Kinoton"), "cc0", "freesound");
        assert!(a.contains("Deep Whoosh") && a.contains("Kinoton") && a.contains("CC0"));
        let b = attribution_line("Clip", None, "cc-by", "jamendo");
        assert!(b.contains("Clip") && b.contains("CC-BY") && !b.contains(" by "));
    }

    #[test]
    fn openverse_hit_parses() {
        let v = json!({
            "id": "abc-123",
            "title": "Deep Whoosh #1",
            "url": "https://cdn.freesound.org/previews/351/351256_2247456-hq.mp3",
            "creator": "Kinoton",
            "license": "cc0",
            "license_url": "https://creativecommons.org/publicdomain/zero/1.0/",
            "source": "freesound",
            "foreign_landing_url": "https://freesound.org/people/Kinoton/sounds/351256",
            "filetype": "mp3",
            "duration": 3155,
            "filesize": 69196
        });
        let h = openverse_hit(&v, "audio").expect("parses");
        assert_eq!(h.id, "abc-123");
        assert_eq!(
            h.download_url,
            "https://cdn.freesound.org/previews/351/351256_2247456-hq.mp3"
        );
        assert_eq!(h.license, "cc0");
        assert_eq!(h.duration_ms, Some(3155));
        assert!(!h.requires_attribution, "cc0 needs no attribution");
        assert!(h.attribution.contains("Kinoton"));
        // A missing url → None (can't fetch).
        let bad = json!({"id":"x","title":"t"});
        assert!(openverse_hit(&bad, "audio").is_none());
    }

    #[test]
    fn local_search_matches_by_name_and_kind() {
        // Build a temp tree with a couple of files.
        let dir = std::env::temp_dir().join(format!("cut-prov-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("whoosh_fast.wav"), b"RIFF....").unwrap();
        std::fs::write(dir.join("photo.png"), b"\x89PNG").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        // audio search for "whoosh" → the wav only.
        let hits = local_search("whoosh", "audio", &dir, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, "audio");
        assert_eq!(hits[0].license, "local");
        assert!(hits[0].title.contains("whoosh"));
        // image search → the png; txt is never a hit (not a media ext).
        let imgs = local_search("", "image", &dir, 10).unwrap();
        assert_eq!(imgs.len(), 1);
        assert!(imgs[0].title.ends_with(".png"));
        // resolve the wav id (its path) round-trips.
        let r = local_resolve(&hits[0].id).unwrap();
        assert_eq!(r.kind, "audio");
        let err = local_resolve(dir.join("notes.txt").to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_ARGS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ssrf_host_guard() {
        // public hosts pass.
        assert_eq!(
            url_host("https://93.184.216.34/previews/1.mp3").as_deref(),
            Some("93.184.216.34")
        );
        assert!(!host_is_internal("93.184.216.34"));
        assert!(!host_is_internal("8.8.8.8"));
        // internal / private / loopback / link-local / CGNAT / TLDs → blocked.
        for h in [
            "localhost",
            "foo.local",
            "svc.internal",
            "127.0.0.1",
            "10.0.0.5",
            "172.16.0.2",
            "172.16.0.1",
            "169.254.169.254",
            "0.0.0.0",
            "100.64.0.1",
            "::1",
        ] {
            assert!(host_is_internal(h), "{h} must be blocked");
        }
        // url_host strips userinfo + port; a userinfo trick can't smuggle a host.
        assert_eq!(
            url_host("http://user@127.0.0.1:8080/x").as_deref(),
            Some("127.0.0.1")
        );
        assert!(host_is_internal(&url_host("http://127.0.0.1:9/x").unwrap()));
    }

    #[test]
    fn urlencode_query() {
        assert_eq!(urlencode("hello world"), "hello%20world");
        assert_eq!(urlencode("a&b=c"), "a%26b%3Dc");
        assert_eq!(urlencode("plain-text_1.0"), "plain-text_1.0");
    }
}
