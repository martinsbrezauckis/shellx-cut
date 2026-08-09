//! Portable, render-bound review handoff.

use super::*;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

const FEEDBACK_SCHEMA: &str = "shellx-cut/review-feedback/1";
const PACKAGE_SCHEMA: &str = "shellx-cut/review-package/1";
const MAX_FEEDBACK_BYTES: u64 = 2 * 1024 * 1024;
const MAX_FEEDBACK_COMMENTS: usize = 500;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FeedbackFile {
    schema: String,
    project: String,
    source_op_id: String,
    render_id: String,
    render_hash: String,
    comments: Vec<FeedbackComment>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FeedbackComment {
    at_ms: u64,
    #[serde(default)]
    end_ms: Option<u64>,
    text: String,
    #[serde(default)]
    author: Option<String>,
}

#[derive(Debug, Serialize)]
struct PackageComment {
    id: String,
    at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_ms: Option<u64>,
    text: String,
    author: String,
    status: String,
}

fn review_op_changes_render(op: &OpRecord) -> bool {
    !matches!(
        op.verb.as_str(),
        "comment.add"
            | "comment.import"
            | "comment.resolve"
            | "comment.draft"
            | "project.checkpoint"
            | "project.rename"
            | "project.brand"
            | "project.sequence_rename"
            | "grade.save"
            | "captions.save_style"
            | "media.bin_save"
            | "media.bin_delete"
    )
}

fn review_state(store: &ProjectStore, source_op_id: &str) -> Result<(String, bool), CutError> {
    let ops = store.log.read_all()?;
    let current_head = ops
        .last()
        .map(|op| op.op_id.clone())
        .unwrap_or_else(|| "op_000000".into());
    let after = if source_op_id == "op_000000" {
        ops.as_slice()
    } else {
        let index = ops
            .iter()
            .position(|op| op.op_id == source_op_id)
            .ok_or_else(|| {
                CutError::new(
                    error_codes::CONFLICT,
                    "the review receipt points to an unknown project operation",
                    format!("source op '{source_op_id}' is not in the open project's log"),
                )
                .with_suggested_action(
                    "render the current cut again before sharing or importing feedback",
                )
            })?;
        &ops[index + 1..]
    };
    Ok((current_head, after.iter().any(review_op_changes_render)))
}

pub(super) fn require_history_tip(
    store: &ProjectStore,
    expected_tip: Option<&str>,
) -> Result<(), CutError> {
    let Some(expected_tip) = expected_tip else {
        return Ok(());
    };
    let current_tip = store.log.read_all()?.last().map(|op| op.op_id.clone());
    if current_tip.as_deref() == Some(expected_tip) {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::CONFLICT,
        "the project changed after this revert was prepared",
        format!(
            "expected history tip {expected_tip}, current tip is {}",
            current_tip.as_deref().unwrap_or("empty")
        ),
    )
    .with_suggested_action("review the newer changes before trying the revert again"))
}

fn require_current_render(
    current_head: &str,
    source_op_id: &str,
    render_state_changed: bool,
    allow_stale: bool,
    rationale: Option<&str>,
) -> Result<bool, CutError> {
    let stale = render_state_changed;
    if stale && !allow_stale {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the review render is older than the current project state",
            format!("render is at {source_op_id}; current project head is {current_head}"),
        )
        .with_suggested_action("render the current cut, then export/import its review package"));
    }
    if stale && rationale.is_none_or(|r| r.trim().is_empty()) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "allow_stale requires a rationale",
            "importing feedback against an older timeline is an explicit override",
        )
        .with_suggested_action(
            "include rationale explaining why the old render feedback still applies",
        ));
    }
    Ok(stale)
}

fn copy_file_atomic(verified_render: &Path, destination: &Path) -> Result<(), CutError> {
    let tmp = temp_output_path_for_render(destination);
    let result = (|| {
        let mut input = std::fs::File::open(verified_render)?;
        let mut output = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        std::io::copy(&mut input, &mut output)?;
        output.flush()?;
        output.sync_all()?;
        drop(output);
        publish_output_atomic(&tmp, destination)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn embedded_json(value: &Value) -> Result<String, CutError> {
    Ok(serde_json::to_string(value)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029"))
}

fn review_html(manifest: &Value) -> Result<String, CutError> {
    let data = embedded_json(manifest)?;
    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; media-src 'self' data: blob:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'none'; img-src 'none'; font-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'">
<title>ShellX Cut review</title>
<style>
:root{{color-scheme:dark;--bg:#111316;--panel:#1a1d21;--line:#34383f;--ink:#f2f4f7;--muted:#9ba3ad;--cut:#3d8bfd;--ok:#44b887}}*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--ink);font:14px system-ui,sans-serif}}header{{height:52px;display:flex;align-items:center;justify-content:space-between;padding:0 20px;border-bottom:1px solid var(--line)}}h1{{font-size:15px;margin:0}}#meta{{color:var(--muted);font:12px ui-monospace,monospace}}main{{display:grid;grid-template-columns:minmax(0,1fr) 340px;min-height:calc(100vh - 52px)}}.viewer{{padding:18px;min-width:0}}video{{display:block;width:100%;max-height:calc(100vh - 90px);background:#000}}aside{{border-left:1px solid var(--line);background:var(--panel);padding:16px;overflow:auto}}.clock{{font:13px ui-monospace,monospace;color:var(--cut);margin-bottom:10px}}textarea,input{{width:100%;color:var(--ink);background:#111316;border:1px solid var(--line);border-radius:5px;padding:9px;font:inherit}}textarea{{min-height:90px;resize:vertical}}input{{margin-top:8px}}.actions{{display:flex;gap:8px;margin:9px 0 18px}}button{{border:1px solid var(--line);border-radius:5px;background:#24282e;color:var(--ink);padding:8px 11px;cursor:pointer}}button.primary{{border-color:var(--cut);background:var(--cut)}}button:disabled{{opacity:.45;cursor:not-allowed}}h2{{font-size:12px;text-transform:uppercase;color:var(--muted);margin:18px 0 8px}}ol{{list-style:none;padding:0;margin:0}}li{{border-top:1px solid var(--line);padding:10px 0}}.tc{{font:11px ui-monospace,monospace;color:var(--cut)}}.who{{font-size:11px;color:var(--muted);margin-top:4px}}.empty{{color:var(--muted);padding:10px 0}}@media(max-width:850px){{main{{grid-template-columns:1fr}}aside{{border-left:0;border-top:1px solid var(--line)}}video{{max-height:60vh}}}}
</style>
</head>
<body>
<header><h1 id="project">ShellX Cut review</h1><div id="meta"></div></header>
<main><section class="viewer"><video id="video" controls preload="metadata"></video></section><aside>
<div class="clock" id="clock">00:00.000</div>
<textarea id="note" maxlength="2000" placeholder="Leave a note at the current time"></textarea>
<input id="author" maxlength="80" placeholder="Your name (optional)">
<div class="actions"><button class="primary" id="add">Add at playhead</button><button id="download" disabled>Download feedback</button></div>
<h2>Your feedback</h2><ol id="feedback"></ol>
<h2>Existing context</h2><ol id="context"></ol>
</aside></main>
<script>
'use strict';
const manifest={data};const notes=[];
const video=document.getElementById('video'),clock=document.getElementById('clock'),note=document.getElementById('note'),author=document.getElementById('author'),feedback=document.getElementById('feedback'),context=document.getElementById('context'),download=document.getElementById('download');
video.src=manifest.media_file;document.getElementById('project').textContent=manifest.project;document.getElementById('meta').textContent=manifest.render.render_id+' · '+(manifest.render.qc_pass?'QC passed':'QC needs attention');
const tc=ms=>{{const n=Math.max(0,Math.round(ms)),m=Math.floor(n/60000),s=Math.floor((n%60000)/1000),x=n%1000;return String(m).padStart(2,'0')+':'+String(s).padStart(2,'0')+'.'+String(x).padStart(3,'0')}};
const row=(item,editable)=>{{const li=document.createElement('li'),t=document.createElement('div'),body=document.createElement('div'),who=document.createElement('div');t.className='tc';t.textContent=tc(item.at_ms);body.textContent=item.text;who.className='who';who.textContent=item.author||'external reviewer';li.append(t,body,who);if(editable)li.dataset.feedbackNote='';return li}};
const renderNotes=()=>{{feedback.replaceChildren(...notes.map(n=>row(n,true)));download.disabled=notes.length===0}};
if(manifest.comments.length)context.replaceChildren(...manifest.comments.map(c=>row(c,false)));else{{const e=document.createElement('div');e.className='empty';e.textContent='No existing comments';context.replaceChildren(e)}}
const updateClock=()=>clock.textContent=tc(video.currentTime*1000);video.addEventListener('timeupdate',updateClock);video.addEventListener('seeked',updateClock);
document.getElementById('add').addEventListener('click',()=>{{const text=note.value.trim();if(!text)return;notes.push({{at_ms:Math.round(video.currentTime*1000),text,author:author.value.trim()||'external reviewer'}});note.value='';renderNotes()}});
download.addEventListener('click',()=>{{const payload={{schema:'{FEEDBACK_SCHEMA}',project:manifest.project,source_op_id:manifest.source_op_id,render_id:manifest.render.render_id,render_hash:manifest.render.output_hash,comments:notes}};const blob=new Blob([JSON.stringify(payload,null,2)+'\n'],{{type:'application/json'}}),a=document.createElement('a');a.href=URL.createObjectURL(blob);a.download='feedback_'+manifest.render.render_id+'.json';a.click();setTimeout(()=>URL.revokeObjectURL(a.href),1000)}});
</script>
</body></html>
"#
    ))
}

pub(super) async fn comment_export(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(Deserialize)]
    struct Args {
        path: Option<String>,
        #[serde(default)]
        allow_stale: bool,
    }
    let a: Args = parse_args(args)?;
    let (project, dir, receipt, current_head, render_state_changed) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let receipt = rendering::read_receipt(&store.receipts_dir(), None)?;
        let (current_head, render_state_changed) = review_state(store, &receipt.at_op)?;
        (
            store.project.clone(),
            store.dir.clone(),
            receipt,
            current_head,
            render_state_changed,
        )
    };
    let stale = require_current_render(
        &current_head,
        &receipt.at_op,
        render_state_changed,
        a.allow_stale,
        a.allow_stale.then_some("export override"),
    )?;

    let receipt_path = PathBuf::from(&receipt.output_path);
    let receipt_path = if receipt_path.is_absolute() {
        receipt_path
    } else {
        dir.join(receipt_path)
    };
    // The review render is read from wherever the engine was AUTHORIZED to deliver
    // it — the project's exports subtree, CUTD_OUTPUTS_DIR, or the folder the user
    // chose with project.set_output_dir. Fencing this to <project>/exports alone
    // made the whole verb unreachable for anyone with a default export folder set:
    // render.final delivers there, so the receipt's output_path was outside the
    // fence and comment.export refused a file the engine had just written itself.
    // (Symptom: render=done, export=false, with the project's receipt pointing
    // at the configured output dir.) The package
    // itself is unaffected — the media is COPIED next to the .html inside
    // <project>/exports, so a shared package stays self-contained.
    let source = fenced_existing_export_read(
        &dir,
        &receipt_path,
        "review render",
        "render the current cut, then export its review package",
    )?;
    let source_for_fingerprint = source.clone();
    let actual_fingerprint = run_blocking("comment.export.verify_render", move || {
        cut_core::hash_file(&source_for_fingerprint)
    })
    .await?;
    if actual_fingerprint != receipt.output_hash {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the render file no longer matches its receipt",
            format!(
                "receipt hash={} actual hash={actual_fingerprint}",
                receipt.output_hash
            ),
        )
        .with_suggested_action("render the current cut again before sharing it"));
    }

    let default = format!("exports/review_{}.html", receipt.render_id);
    let html_path =
        fence_project_output_path(&dir, a.path.as_deref(), &default, OutputPathPolicy::HTML)?;
    let stem = html_path
        .file_stem()
        .and_then(|x| x.to_str())
        .ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "review package path needs a file name",
                html_path.display().to_string(),
            )
        })?;
    let parent = html_path.parent().unwrap_or_else(|| Path::new("."));
    let media_ext = source.extension().and_then(|x| x.to_str()).unwrap_or("mp4");
    let media_policy = match media_ext.to_ascii_lowercase().as_str() {
        "mp4" => OutputPathPolicy::MP4,
        "webm" => OutputPathPolicy::WEBM,
        "mov" => OutputPathPolicy::MOV,
        other => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "review package render has an unsupported container",
                format!("the verified render ends in .{other}"),
            ))
        }
    };
    let media_path = parent.join(format!("{stem}.{media_ext}"));
    let manifest_path = parent.join(format!("{stem}.json"));
    if media_path.exists() || manifest_path.exists() {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "review package sidecar already exists",
            format!("{} or {}", media_path.display(), manifest_path.display()),
        )
        .with_suggested_action("choose a different .html package name"));
    }
    let media_path = fence_output_path(
        &dir,
        Some(&media_path.display().to_string()),
        "unused",
        media_policy,
    )?;
    let manifest_path = fence_output_path(
        &dir,
        Some(&manifest_path.display().to_string()),
        "unused",
        OutputPathPolicy::JSON,
    )?;

    let comments: Vec<PackageComment> = project
        .comments
        .iter()
        .map(|comment| {
            let at_ms = project
                .resolve_comment_anchor_ms(comment)
                .unwrap_or(comment.at_ms);
            let end_ms = comment
                .end_ms
                .map(|end| at_ms.saturating_add(end.saturating_sub(comment.at_ms)));
            PackageComment {
                id: comment.id.clone(),
                at_ms,
                end_ms,
                text: comment.text.clone(),
                author: comment.author.clone(),
                status: comment.status.clone(),
            }
        })
        .collect();
    let media_file = media_path
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or_default();
    let manifest = json!({
        "schema": PACKAGE_SCHEMA,
        "project": project.name,
        "source_op_id": receipt.at_op,
        "exported_at": OpRecord::now_ts(),
        "media_file": media_file,
        "render": {
            "render_id": receipt.render_id,
            "output_hash": receipt.output_hash,
            "duration_ms": receipt.duration_ms,
            "preset": receipt.preset,
            "qc_pass": receipt.pass,
            "checks": receipt.checks,
        },
        "comments": comments,
    });

    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    let html_bytes = review_html(&manifest)?.into_bytes();
    let source_for_copy = source.clone();
    let expected_copy_fingerprint = actual_fingerprint.clone();
    let media_for_write = media_path.clone();
    let manifest_for_write = manifest_path.clone();
    let html_for_write = html_path.clone();
    run_blocking("comment.export.package", move || {
        copy_file_atomic(&source_for_copy, &media_for_write)?;
        let copied_fingerprint = cut_core::hash_file(&media_for_write)?;
        if copied_fingerprint != expected_copy_fingerprint {
            let _ = std::fs::remove_file(&media_for_write);
            return Err(CutError::new(
                error_codes::IO,
                "the review render copy failed verification",
                format!("source hash={expected_copy_fingerprint} copied hash={copied_fingerprint}"),
            ));
        }
        let result = (|| {
            write_output_atomic(&manifest_for_write, manifest_bytes)?;
            write_output_atomic(&html_for_write, html_bytes)?;
            Ok::<(), CutError>(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&media_for_write);
            let _ = std::fs::remove_file(&manifest_for_write);
            let _ = std::fs::remove_file(&html_for_write);
        }
        result
    })
    .await?;

    Ok(VerbResult::ok(json!({
        "path": html_path,
        "manifest_path": manifest_path,
        "media_path": media_path,
        "render_id": receipt.render_id,
        "source_op_id": receipt.at_op,
        "render_hash": actual_fingerprint,
        "stale": stale,
    })))
}

pub(super) async fn comment_import(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(Deserialize)]
    struct Args {
        path: String,
        #[serde(default)]
        allow_stale: bool,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let path = PathBuf::from(&a.path);
    if path
        .extension()
        .and_then(|x| x.to_str())
        .is_none_or(|x| !x.eq_ignore_ascii_case("json"))
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "review feedback path must end in .json",
            a.path,
        ));
    }
    let path = path.canonicalize().map_err(|e| {
        CutError::new(
            error_codes::NOT_FOUND,
            "review feedback file was not found",
            e.to_string(),
        )
    })?;
    let meta = path.metadata()?;
    if !meta.is_file() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "review feedback path is not a regular file",
            path.display().to_string(),
        ));
    }
    if meta.len() > MAX_FEEDBACK_BYTES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "review feedback JSON is too large",
            format!(
                "{} bytes exceeds the {} byte limit",
                meta.len(),
                MAX_FEEDBACK_BYTES
            ),
        ));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&path)?
        .take(MAX_FEEDBACK_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FEEDBACK_BYTES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "review feedback JSON is too large",
            "bounded read exceeded 2 MiB",
        ));
    }
    let feedback: FeedbackFile = serde_json::from_slice(&bytes).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "review feedback JSON is invalid",
            e.to_string(),
        )
    })?;
    if feedback.schema != FEEDBACK_SCHEMA {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "unsupported review feedback schema",
            feedback.schema,
        ));
    }
    if feedback.comments.is_empty() || feedback.comments.len() > MAX_FEEDBACK_COMMENTS {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "review feedback must contain between 1 and 500 comments",
            format!("got {} comments", feedback.comments.len()),
        ));
    }

    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let receipt = rendering::read_receipt(&store.receipts_dir(), Some(&feedback.render_id))?;
    if receipt.at_op != feedback.source_op_id || receipt.output_hash != feedback.render_hash {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "review feedback does not match the saved render receipt",
            "source_op_id or render_hash differs from the local receipt",
        ));
    }
    let (current_head, render_state_changed) = review_state(store, &feedback.source_op_id)?;
    let stale = require_current_render(
        &current_head,
        &feedback.source_op_id,
        render_state_changed,
        a.allow_stale,
        a.rationale.as_deref(),
    )?;
    let mut notes = Vec::new();
    for comment in feedback.comments {
        if comment.at_ms > receipt.duration_ms
            || comment
                .end_ms
                .is_some_and(|end| end > receipt.duration_ms || end <= comment.at_ms)
        {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "review comment time is outside the rendered duration",
                format!(
                    "at_ms={} end_ms={:?} duration_ms={}",
                    comment.at_ms, comment.end_ms, receipt.duration_ms
                ),
            ));
        }
        notes.push(cut_core::ReviewFeedbackNote {
            at_ms: comment.at_ms,
            end_ms: comment.end_ms,
            text: comment.text,
            author: comment.author.unwrap_or_else(|| "external reviewer".into()),
        });
    }
    let source = cut_core::CommentReviewSource {
        source_op_id: feedback.source_op_id.clone(),
        render_id: feedback.render_id.clone(),
        render_hash: feedback.render_hash.clone(),
    };
    let reviewed_project = feedback.project.clone();
    let current_project = store.project.name.clone();
    let (comments, op) = guard_call("comment.import", || {
        store.import_review_comments(notes, source, actor, a.rationale)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({
            "comments": comments,
            "count": comments.len(),
            "source_op_id": feedback.source_op_id,
            "render_id": feedback.render_id,
            "reviewed_project": reviewed_project,
            "current_project": current_project,
            "stale": stale,
        }),
        vec![op_id],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_reviewer_has_offline_csp_and_escaped_manifest() {
        let html = review_html(&json!({
            "project": "</script><script>window.bad=true</script>",
            "media_file": "review.mp4",
            "source_op_id": "op_1",
            "render": {"render_id":"render_001","output_hash":"sha256:x","qc_pass":true},
            "comments": [],
        }))
        .unwrap();
        assert!(html.contains("connect-src 'none'"));
        assert!(html.contains("default-src 'none'"));
        assert!(!html.contains("</script><script>window.bad"));
        assert!(html.contains("shellx-cut/review-feedback/1"));
    }

    #[test]
    fn stale_override_requires_a_reason() {
        let err = require_current_render("op_2", "op_1", true, true, None).unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_ARGS);
        assert!(!require_current_render("op_2", "op_1", false, false, None).unwrap());
    }

    #[test]
    fn review_metadata_does_not_make_render_stale() {
        let op = |verb: &str| OpRecord {
            op_id: "op_2".into(),
            ts: OpRecord::now_ts(),
            actor: Actor::system(),
            verb: verb.into(),
            args: json!({}),
            rationale: None,
            effects: vec![],
            inverse: None,
            status: cut_core::OpStatus::Applied,
        };
        assert!(!review_op_changes_render(&op("comment.import")));
        assert!(!review_op_changes_render(&op("project.rename")));
        assert!(review_op_changes_render(&op("edit.add_marker")));
    }
}
