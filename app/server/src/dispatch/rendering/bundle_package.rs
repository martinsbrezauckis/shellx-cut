use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize)]
pub(super) struct PublishPackageIssue {
    pub(super) code: String,
    pub(super) severity: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) aspect: Option<String>,
    pub(super) detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct PublishPackageAssessment {
    pub(super) status: &'static str,
    pub(super) pass: bool,
    pub(super) issues: Vec<PublishPackageIssue>,
}

fn issue(
    code: &str,
    severity: &'static str,
    aspect: Option<&str>,
    detail: impl Into<String>,
) -> PublishPackageIssue {
    PublishPackageIssue {
        code: code.to_string(),
        severity,
        aspect: aspect.map(str::to_string),
        detail: detail.into(),
    }
}

pub(super) fn assess_publish_package(
    platforms: &[Value],
    brand: Option<&Value>,
) -> PublishPackageAssessment {
    let mut issues = Vec::new();
    if platforms.is_empty() {
        issues.push(issue(
            "no_platforms",
            "error",
            None,
            "the package contains no platform deliverables",
        ));
    }
    for platform in platforms {
        let aspect = platform
            .get("aspect")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        match platform.get("pass").and_then(Value::as_bool) {
            Some(true) => {}
            Some(false) => issues.push(issue(
                "platform_qc_failed",
                "error",
                Some(aspect),
                "the rendered platform receipt failed",
            )),
            None => issues.push(issue(
                "platform_qc_unverified",
                "warning",
                Some(aspect),
                "the platform render has no completed output-fact receipt",
            )),
        }
        for (field, code, detail) in [
            (
                "caption_write_failed",
                "caption_write_failed",
                "one or more caption sidecars could not be written",
            ),
            (
                "receipt_persist_failed",
                "receipt_persist_failed",
                "the platform receipt could not be persisted",
            ),
            (
                "artifact_hash_failed",
                "artifact_hash_failed",
                "one or more package artifacts could not be hashed",
            ),
        ] {
            if platform.get(field).and_then(Value::as_str).is_some() {
                issues.push(issue(code, "error", Some(aspect), detail));
            }
        }
        if platform.get("hash").and_then(Value::as_str).is_none() {
            issues.push(issue(
                "video_hash_missing",
                "error",
                Some(aspect),
                "the primary video has no content hash",
            ));
        }
        if platform.get("thumb").and_then(Value::as_str).is_none() {
            issues.push(issue(
                "thumbnail_missing",
                "warning",
                Some(aspect),
                "the optional platform thumbnail was not created",
            ));
        }
    }
    if let Some(brand) = brand {
        match brand.get("pass").and_then(Value::as_bool) {
            Some(true) => {}
            Some(false) => issues.push(issue(
                "brand_check_failed",
                "error",
                None,
                "one or more platform outputs violate the active brand constraints",
            )),
            None => issues.push(issue(
                "brand_check_unverified",
                "warning",
                None,
                "the active brand constraints did not produce a verdict",
            )),
        }
    }

    let blocked = issues.iter().any(|entry| entry.severity == "error");
    let status = if blocked {
        "blocked"
    } else if issues.is_empty() {
        "ready"
    } else {
        "needs_review"
    };
    PublishPackageAssessment {
        status,
        pass: status == "ready",
        issues,
    }
}

pub(super) fn publish_package_manifest(
    bundle_id: &str,
    range_ms: [u64; 2],
    source_op_id: &str,
    platforms: &[Value],
    brand: Option<&Value>,
    assessment: &PublishPackageAssessment,
) -> Value {
    json!({
        "schema": "shellx-cut/publish-package/1",
        "bundle_id": bundle_id,
        "created_ts": cut_core::OpRecord::now_ts(),
        "source_op_id": source_op_id,
        "range_ms": range_ms,
        "status": assessment.status,
        "pass": assessment.pass,
        "issues": assessment.issues,
        "platforms": platforms,
        "brand": brand,
    })
}

pub(super) fn optional_artifact_hash(path: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(path) = path else {
        return (None, None);
    };
    match cut_core::hash_file(std::path::Path::new(path)) {
        Ok(hash) => (Some(hash), None),
        Err(error) => (None, Some(format!("{path}: {error}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn platform(pass: Value, thumb: Value) -> Value {
        json!({
            "aspect": "9:16",
            "pass": pass,
            "hash": "sha256:video",
            "thumb": thumb,
            "caption_write_failed": null,
            "receipt_persist_failed": null,
            "artifact_hash_failed": null,
        })
    }

    #[test]
    fn ready_requires_verified_hashed_platforms() {
        let assessment = assess_publish_package(&[platform(json!(true), json!("thumb.jpg"))], None);
        assert_eq!(assessment.status, "ready");
        assert!(assessment.pass);
        assert!(assessment.issues.is_empty());
    }

    #[test]
    fn unverified_or_missing_optional_artifacts_need_review() {
        let assessment = assess_publish_package(&[platform(Value::Null, Value::Null)], None);
        assert_eq!(assessment.status, "needs_review");
        assert!(!assessment.pass);
        assert_eq!(assessment.issues.len(), 2);
    }

    #[test]
    fn failed_platform_or_brand_blocks_package() {
        let mut item = platform(json!(false), json!("thumb.jpg"));
        item["caption_write_failed"] = json!("disk full");
        let assessment = assess_publish_package(&[item], Some(&json!({"pass": false})));
        assert_eq!(assessment.status, "blocked");
        assert!(!assessment.pass);
        assert!(assessment
            .issues
            .iter()
            .all(|entry| entry.severity == "error"));
    }

    #[test]
    fn manifest_binds_source_and_assessment() {
        let platforms = [platform(json!(true), json!("thumb.jpg"))];
        let assessment = assess_publish_package(&platforms, None);
        let manifest = publish_package_manifest(
            "bundle_0_1000",
            [0, 1000],
            "op_000007",
            &platforms,
            None,
            &assessment,
        );
        assert_eq!(manifest["schema"], "shellx-cut/publish-package/1");
        assert_eq!(manifest["source_op_id"], "op_000007");
        assert_eq!(manifest["status"], "ready");
    }
}
