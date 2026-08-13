use super::*;

fn overlay_ops(text: &str, split_count: u64) -> Vec<OpRecord> {
    let dir = tempfile::tempdir().expect("temporary project root");
    let mut store =
        ProjectStore::create(dir.path(), "overlay-metadata", None).expect("create project store");
    let duration_ms = split_count + 2;
    let (asset_id, _) = store
        .record_import(
            Some("a1".into()),
            cut_core::Asset {
                path: "/testdata/title.mov".into(),
                hash: "sha256:title".into(),
                probe: Some(json!({"duration_ms": duration_ms})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
            Actor::system(),
            None,
        )
        .expect("record title asset");
    store
        .apply_lowered(
            "title.add",
            json!({"text": text, "range_ms": [0, duration_ms]}),
            Actor::system(),
            None,
            vec![
                InverseOp {
                    verb: "edit.add_track".into(),
                    args: json!({"kind": "video", "id": "title1"}),
                },
                InverseOp {
                    verb: "edit.insert".into(),
                    args: json!({
                        "asset": asset_id,
                        "track": "title1",
                        "at_ms": 0,
                        "src_range_ms": [0, duration_ms],
                        "ripple": false,
                    }),
                },
            ],
            vec![],
        )
        .expect("record title placement");
    for at_ms in 1..=split_count {
        store
            .apply(
                "edit.split",
                json!({"track": "title1", "at_ms": at_ms}),
                Actor::system(),
                None,
            )
            .expect("record split");
    }
    store.log.read_all().expect("read overlay log")
}

#[test]
fn split_metadata_projection_accepts_normal_title_splits() {
    validate_split_metadata_projection(&overlay_ops("A normal title", 16))
        .expect("normal title metadata stays within the projection budget");
}

#[test]
fn split_metadata_projection_rejects_oversized_imported_specs() {
    let ops = overlay_ops(&"x".repeat(MAX_OVERLAY_METADATA_OPERATION_BYTES + 1), 1);
    let error = validate_split_metadata_projection(&ops)
        .expect_err("oversized imported metadata must fail closed");
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert!(error.message.contains("metadata exceeds"), "{error:?}");

    let recovery_error = super::super::title_shape::recover_title_args_for_tests(&ops, "c1")
        .expect_err("title recovery must reject the same oversized metadata");
    assert_eq!(recovery_error.code, error_codes::INVALID_ARGS);
}

#[test]
fn split_metadata_projection_rejects_many_descendants_of_a_large_spec() {
    let ops = overlay_ops(&"x".repeat(MAX_OVERLAY_METADATA_OPERATION_BYTES - 256), 64);
    let error = validate_split_metadata_projection(&ops)
        .expect_err("large metadata cannot be copied to unbounded descendants");
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert!(error.message.contains("projection exceeds"), "{error:?}");
}

#[test]
fn split_metadata_projection_rejects_forged_left_right_provenance() {
    let mut ops = overlay_ops("A normal title", 1);
    let split = ops
        .iter_mut()
        .find(|op| op.verb == "edit.split")
        .expect("split operation");
    split.effects[0]
        .detail
        .insert("left".into(), Value::String("forged-left".into()));

    let error = validate_split_metadata_projection(&ops)
        .expect_err("forged split provenance must not propagate metadata");
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert!(error.message.contains("topology"), "{error:?}");
}
