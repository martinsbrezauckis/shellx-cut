//! Merge frozen scene layouts with explicit Studio camera edits for polish.
//! Visibility remains scene-owned; only chosen layout fields carry across scene
//! switches until a camera layout reset restores the active preset.

use record_core::WebcamKeyframe;

pub(crate) enum CameraTimelineEdit {
    Transform { order: u64, key: WebcamKeyframe },
    Reset { order: u64, t_ms: u64 },
}

#[derive(Default)]
struct LayoutOverride {
    x: Option<f64>,
    y: Option<f64>,
    size: Option<f64>,
    shape: Option<record_core::WebcamShape>,
}

impl LayoutOverride {
    fn remember(&mut self, key: &WebcamKeyframe) {
        if key.x.is_some() {
            self.x = key.x;
        }
        if key.y.is_some() {
            self.y = key.y;
        }
        if key.size.is_some() {
            self.size = key.size;
        }
        if key.shape.is_some() {
            self.shape = key.shape;
        }
    }

    fn apply(&self, key: &mut WebcamKeyframe) {
        key.x = self.x.or(key.x);
        key.y = self.y.or(key.y);
        key.size = self.size.or(key.size);
        key.shape = self.shape.or(key.shape);
    }
}

pub(crate) fn merge_camera_timeline(
    scene_keys: Vec<WebcamKeyframe>,
    studio_edits: Vec<CameraTimelineEdit>,
    static_layout: WebcamKeyframe,
) -> Vec<WebcamKeyframe> {
    enum Item {
        Scene(WebcamKeyframe),
        Transform(WebcamKeyframe),
        Reset(u64),
    }
    let has_scenes = !scene_keys.is_empty();
    let mut items = Vec::with_capacity(scene_keys.len() + studio_edits.len());
    for (order, key) in scene_keys.into_iter().enumerate() {
        items.push((key.t_ms, 0u8, order as u64, Item::Scene(key)));
    }
    for edit in studio_edits {
        match edit {
            CameraTimelineEdit::Transform { order, key } => {
                items.push((key.t_ms, 1, order, Item::Transform(key)));
            }
            CameraTimelineEdit::Reset { order, t_ms } => {
                items.push((t_ms, 1, order, Item::Reset(t_ms)));
            }
        }
    }
    // The scene journal and Studio journal do not share a sequence. At equal
    // milliseconds the explicit Studio action wins, as it did before this merge.
    items.sort_by_key(|(t_ms, source, order, _)| (*t_ms, *source, *order));

    let mut active_preset = (!has_scenes).then_some(static_layout);
    let mut selected = LayoutOverride::default();
    let mut merged = Vec::with_capacity(items.len());
    for (_, _, _, item) in items {
        match item {
            Item::Scene(mut key) => {
                active_preset = (key.visible == Some(true)).then_some(key);
                if active_preset.is_some() {
                    selected.apply(&mut key);
                }
                merged.push(key);
            }
            Item::Transform(key) => {
                selected.remember(&key);
                merged.push(key);
            }
            Item::Reset(t_ms) => {
                selected = LayoutOverride::default();
                if let Some(mut key) = active_preset {
                    key.t_ms = t_ms;
                    key.visible = None;
                    merged.push(key);
                }
            }
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_edit_wins_an_equal_millisecond_scene_switch() {
        let scene = WebcamKeyframe {
            t_ms: 500,
            visible: Some(true),
            x: Some(0.70),
            y: Some(0.04),
            size: Some(0.22),
            shape: Some(record_core::WebcamShape::Circle),
        };
        let edit = WebcamKeyframe {
            t_ms: 500,
            visible: None,
            x: Some(0.04),
            y: Some(0.74),
            size: Some(0.22),
            shape: None,
        };
        let merged = merge_camera_timeline(
            vec![scene],
            vec![CameraTimelineEdit::Transform {
                order: 1,
                key: edit,
            }],
            scene,
        );
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].x, Some(0.70));
        assert_eq!(merged[1].x, Some(0.04));
    }
}
