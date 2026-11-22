//! Replay saved edit steps without applying the new-edit keyframe ceiling.
use super::*;

pub fn apply_saved_record(
    project: &mut Project,
    op: &OpRecord,
    prior: &[OpRecord],
) -> Result<(), CutError> {
    if crate::redaction_limit::apply_saved_over_cap(project, op)? {
        return Ok(());
    }
    apply_record(project, op, prior)
}

pub(super) fn apply_saved_edit_verb_pinned(
    project: &mut Project,
    verb: &str,
    args: &serde_json::Value,
    pinned: Option<&mut crate::rebase::PinnedIds>,
) -> Result<Vec<crate::OpEffect>, CutError> {
    if verb != "edit.keyframe" {
        return apply_edit_verb_pinned(project, verb, args, pinned);
    }
    #[derive(serde::Deserialize)]
    struct Args {
        clip: String,
        param: crate::types::KfParam,
        #[serde(default)]
        points: Vec<crate::types::KfPoint>,
        #[serde(default)]
        interp: crate::types::KfInterp,
    }
    let args: Args = serde_json::from_value(args.clone())?;
    crate::edit::keyframe_legacy(project, &args.clip, args.param, args.points, args.interp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{types::Asset, KfPoint};

    #[test]
    fn old_oversized_redaction_journal_opens_without_rewriting_mask() {
        use crate::{types::MaskRegion, ClipMask, MaskEffect, MaskShape};
        let regions = 257;
        let root = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(root.path(), "legacy redact", None).unwrap();
        store
            .record_import(
                None,
                Asset {
                    path: "/legacy/source.mp4".into(),
                    hash: "sha256:legacy".into(),
                    probe: Some(json!({"duration_ms": 3000})),
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                Actor::system(),
                None,
            )
            .unwrap();
        store
            .apply(
                "edit.insert",
                json!({"asset":"a1","track":"v1","at_ms":0}),
                Actor::system(),
                None,
            )
            .unwrap();
        let clip = store.project.track("v1").unwrap().clips[0]
            .id()
            .unwrap()
            .to_string();
        let mask = ClipMask {
            shape: MaskShape::Rect,
            points: vec![[0.1, 0.1], [0.2, 0.2]],
            feather: 0.0,
            invert: false,
            effect: MaskEffect::Blur,
            strength: Some(25.0),
            range_ms: None,
            track: None,
            regions: (1..regions)
                .map(|_| MaskRegion {
                    shape: MaskShape::Rect,
                    points: vec![[0.1, 0.1], [0.2, 0.2]],
                    track: None,
                })
                .collect(),
        };
        let op = OpRecord {
            op_id: "op_000004".into(),
            ts: "2026-01-01T00:00:00.000Z".into(),
            actor: Actor::system(),
            verb: "edit.redact".into(),
            args: json!({"clip":clip,"shape":"rect","points":mask.points,
                    "track":mask.track,"boxes":mask.regions}),
            rationale: None,
            effects: vec![edit::fx(
                Some("v1"),
                json!({"clip":clip,"old_mask":null,"mask":mask}),
            )],
            inverse: None,
            status: OpStatus::Applied,
        };
        store.log.append(&op).unwrap();
        let before = std::fs::read(store.dir.join("ops.jsonl")).unwrap();
        let loaded = ProjectStore::open(&store.dir).expect("saved over-cap redaction opens");
        let crate::Clip::Media(media) = &loaded.project.track("v1").unwrap().clips[0] else {
            panic!("expected media clip")
        };
        assert_eq!(media.mask.as_ref().unwrap().regions.len() + 1, regions);
        assert!(media.mask.as_ref().unwrap().track.is_none());
        assert_eq!(std::fs::read(store.dir.join("ops.jsonl")).unwrap(), before);
    }

    #[test]
    fn old_oversized_keyframe_journal_still_opens_without_rewriting_points() {
        let root = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(root.path(), "legacy", None).unwrap();
        store
            .record_import(
                None,
                Asset {
                    path: "/legacy/source.mp4".into(),
                    hash: "sha256:legacy".into(),
                    probe: Some(json!({"duration_ms": 1_000})),
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                Actor::system(),
                None,
            )
            .unwrap();
        store
            .apply(
                "edit.insert",
                json!({"asset":"a1","track":"v1","at_ms":0}),
                Actor::system(),
                None,
            )
            .unwrap();
        let clip = store.project.track("v1").unwrap().clips[0]
            .id()
            .unwrap()
            .to_string();
        let points: Vec<KfPoint> = (0..=crate::keyframe_limit::MAX_KEYFRAME_POINTS)
            .map(|t_ms| KfPoint {
                t_ms: t_ms as u64,
                value: 0.5,
            })
            .collect();
        let op = OpRecord {
            op_id: "op_000004".into(),
            ts: "2026-01-01T00:00:00.000Z".into(),
            actor: Actor::system(),
            verb: "edit.keyframe".into(),
            args: json!({"clip":clip,"param":"opacity","points":points,"interp":"hold"}),
            rationale: None,
            effects: vec![],
            inverse: None,
            status: OpStatus::Applied,
        };
        store.log.append(&op).unwrap();
        let journal_before = std::fs::read(store.dir.join("ops.jsonl")).unwrap();
        let mut loaded = ProjectStore::open(&store.dir).unwrap();
        let crate::Clip::Media(media) = &loaded.project.track("v1").unwrap().clips[0] else {
            panic!("expected media clip")
        };
        assert_eq!(media.keyframes[0].points, points);
        assert_eq!(
            std::fs::read(store.dir.join("ops.jsonl")).unwrap(),
            journal_before
        );
        loaded
            .apply(
                "edit.keyframe",
                json!({"clip":clip,"param":"opacity","points":[]}),
                Actor::system(),
                None,
            )
            .unwrap();
        let crate::Clip::Media(media) = &loaded.project.track("v1").unwrap().clips[0] else {
            panic!("expected media clip")
        };
        assert!(media.keyframes.is_empty());
    }

    #[test]
    fn saved_journal_over_absolute_keyframe_cap_still_opens() {
        let root = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(root.path(), "over absolute cap", None).unwrap();
        store
            .record_import(
                None,
                Asset {
                    path: "/legacy/source.mp4".into(),
                    hash: "sha256:legacy".into(),
                    probe: Some(json!({"duration_ms": 1000})),
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                Actor::system(),
                None,
            )
            .unwrap();
        store
            .apply(
                "edit.insert",
                json!({"asset":"a1","track":"v1","at_ms":0}),
                Actor::system(),
                None,
            )
            .unwrap();
        let op = OpRecord {
            op_id: "op_000004".into(),
            ts: "2026-01-01T00:00:00.000Z".into(),
            actor: Actor::system(),
            verb: "edit.keyframe".into(),
            args: json!({"clip":"c1","param":"opacity","interp":"hold",
                "points":vec![KfPoint { t_ms: 0, value: 0.5 };
                    crate::keyframe_limit::MAX_KEYFRAME_TOTAL_POINTS + 1]}),
            rationale: None,
            effects: vec![],
            inverse: None,
            status: OpStatus::Applied,
        };
        store.log.append(&op).unwrap();
        let original = std::fs::read(store.dir.join("ops.jsonl")).unwrap();
        let loaded = ProjectStore::open(&store.dir).unwrap();
        let crate::Clip::Media(media) = &loaded.project.track("v1").unwrap().clips[0] else {
            panic!("media clip")
        };
        assert_eq!(media.keyframes[0].points.len(), 200_001);
        assert_eq!(
            std::fs::read(store.dir.join("ops.jsonl")).unwrap(),
            original
        );
    }
}
