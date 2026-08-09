//! Bounded path fields returned by the local Motion connector.

use cut_core::{error_codes, CutError};
use serde_json::Value;

pub(crate) fn render_output(connector: &Value) -> Result<&str, CutError> {
    connector
        .get("render")
        .and_then(|value| value.get("outputPath"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            CutError::new(
                error_codes::SIDECAR,
                "ShellX Motion connector returned no rendered media path",
                "render.outputPath is required for Cut insert",
            )
        })
}

pub(crate) fn cut_plan_path(connector: &Value) -> Result<&str, CutError> {
    required_path(connector, "cutPlanPath", "Cut import plan")
}

pub(crate) fn package_dir(connector: &Value) -> Option<&str> {
    connector
        .get("packageDir")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn required_path<'a>(connector: &'a Value, field: &str, label: &str) -> Result<&'a str, CutError> {
    connector
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            CutError::new(
                error_codes::SIDECAR,
                format!("ShellX Motion connector returned no {label}"),
                format!("{field} is required for atomic Cut insert"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn retains_the_editable_package_binding_from_current_motion_connectors() {
        let connector = json!({
            "render": { "outputPath": "/tmp/render.mp4" },
            "cutPlanPath": "/tmp/cut-import-plan.json",
            "packageDir": "/tmp/editable-motion-package",
        });

        assert_eq!(render_output(&connector).unwrap(), "/tmp/render.mp4");
        assert_eq!(
            cut_plan_path(&connector).unwrap(),
            "/tmp/cut-import-plan.json"
        );
        assert_eq!(
            package_dir(&connector),
            Some("/tmp/editable-motion-package")
        );
    }

    #[test]
    fn legacy_connectors_without_a_package_remain_importable_but_not_editable() {
        let connector = json!({
            "render": { "outputPath": "/tmp/render.mp4" },
            "cutPlanPath": "/tmp/cut-import-plan.json",
        });

        assert_eq!(package_dir(&connector), None);
    }
}
