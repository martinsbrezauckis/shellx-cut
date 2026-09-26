//! Lower scene camera spans into the established editable webcam-plan primitive.

use crate::scene_editable_timeline::EditableSceneTimeline;
use crate::{
    Anchor, CameraClockRange, EditPlan, PipCorner, PipShape, PresenterPip, SceneError, SceneResult,
    WebcamKeyframe, WebcamOverlay, WebcamShape,
};

impl EditableSceneTimeline {
    /// Apply only journal-bound camera visibility/composition keys to a plan.
    pub fn apply_to_edit_plan(
        &self,
        plan: &mut EditPlan,
        camera_source: Option<String>,
        camera_clock: Option<CameraClockRange>,
    ) -> SceneResult<()> {
        self.validate()?;
        let has_presenter = self
            .camera
            .iter()
            .any(|segment| segment.presenter.is_some());
        let source =
            camera_source.or_else(|| plan.webcam.as_ref().map(|webcam| webcam.source.clone()));
        if has_presenter && source.is_none() {
            return Err(SceneError::CameraSourceRequired);
        }
        if let Some(source) = source {
            let mut timeline = Vec::with_capacity(self.camera.len());
            let mut base = None;
            for segment in &self.camera {
                if base.is_none() {
                    base = segment.presenter;
                }
                timeline.push(camera_keyframe(plan, segment.start_ms, segment.presenter)?);
            }
            let (shape, anchor, size) = base.map(default_camera_style).unwrap_or((
                WebcamShape::Circle,
                Anchor::BottomRight,
                0.22,
            ));
            plan.webcam = Some(WebcamOverlay {
                source,
                shape,
                anchor,
                margin: 0.04,
                size,
                camera_clock: camera_clock
                    .or_else(|| plan.webcam.as_ref().and_then(|webcam| webcam.camera_clock)),
                timeline,
            });
        }
        plan.scene_timeline = Some(self.clone());
        Ok(())
    }
}

fn camera_keyframe(
    plan: &EditPlan,
    t_ms: u64,
    presenter: Option<PresenterPip>,
) -> SceneResult<WebcamKeyframe> {
    let Some(presenter) = presenter else {
        return Ok(WebcamKeyframe {
            t_ms,
            visible: Some(false),
            x: None,
            y: None,
            size: None,
            shape: None,
        });
    };
    if plan.source_w == 0 || plan.source_h == 0 {
        return Err(SceneError::InvalidScenePersistence);
    }
    let (_, _, size) = default_camera_style(presenter);
    let vertical_margin = 0.04;
    let horizontal_margin = vertical_margin * f64::from(plan.source_h) / f64::from(plan.source_w);
    let horizontal_size = size * f64::from(plan.source_h) / f64::from(plan.source_w);
    let (x, y) = match presenter.corner() {
        PipCorner::TopLeft => (horizontal_margin, vertical_margin),
        PipCorner::TopRight => (1.0 - horizontal_margin - horizontal_size, vertical_margin),
        PipCorner::BottomRight => (
            1.0 - horizontal_margin - horizontal_size,
            1.0 - vertical_margin - size,
        ),
        PipCorner::BottomLeft => (horizontal_margin, 1.0 - vertical_margin - size),
    };
    Ok(WebcamKeyframe {
        t_ms,
        visible: Some(true),
        x: Some(x.clamp(0.0, 1.0)),
        y: Some(y.clamp(0.0, 1.0)),
        size: Some(size),
        shape: Some(default_camera_style(presenter).0),
    })
}

fn default_camera_style(presenter: PresenterPip) -> (WebcamShape, Anchor, f64) {
    let shape = match presenter.shape() {
        PipShape::Circle => WebcamShape::Circle,
        PipShape::RoundedRect => WebcamShape::RoundedRect { radius: 18.0 },
    };
    let anchor = match presenter.corner() {
        PipCorner::TopLeft => Anchor::TopLeft,
        PipCorner::TopRight => Anchor::TopRight,
        PipCorner::BottomRight => Anchor::BottomRight,
        PipCorner::BottomLeft => Anchor::BottomLeft,
    };
    (
        shape,
        anchor,
        f64::from(presenter.size_percent().get()) / 100.0,
    )
}
