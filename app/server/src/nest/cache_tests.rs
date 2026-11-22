use super::*;

#[test]
fn nested_clip_look_fields_change_the_bake_identity() {
    let mut project = Project::new("nested look", cut_core::ProjectSettings::default());
    let clip: cut_core::MediaClip = serde_json::from_value(serde_json::json!({
        "id":"c1","asset":"a1","src_in_ms":0,"src_out_ms":1000
    }))
    .unwrap();
    project.tracks[0].clips.push(cut_core::Clip::Media(clip));
    let edl = edl_from_project(&project);
    let before = nest_cache_tag(&project, &edl);
    let cut_core::Clip::Media(clip) = &mut project.tracks[0].clips[0] else {
        unreachable!()
    };
    clip.transform = Some(
        serde_json::from_value(serde_json::json!({
            "x":0.1,"y":0.2,"scale":0.4,"shape":"circle","opacity":1.0,
            "ring":{"colour":"#ffffff","width":0.04}
        }))
        .unwrap(),
    );
    assert_ne!(
        before,
        nest_cache_tag(&project, &edl),
        "nested look was omitted from its cache key"
    );
}
