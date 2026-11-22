use cut_core::{edit, Clip, Keyframe, Project, ProjectSettings};
use serde_json::json;

fn fixture(interp: &str) -> Project {
    let mut p = Project::new("cut keys", ProjectSettings::default());
    p.tracks[0].clips.push(serde_json::from_value(json!({"id":"overlay","asset":"a1","src_in_ms":0,"src_out_ms":3000,"keyframes":[{"param":"pos_x","interp":interp,"points":[{"t_ms":0,"value":0.8},{"t_ms":500,"value":0.1},{"t_ms":2500,"value":0.6}]}]})).unwrap());
    p
}
fn keys(p: &Project, index: usize) -> &Keyframe {
    let Clip::Media(c) = &p.tracks[0].clips[index] else {
        panic!()
    };
    &c.keyframes[0]
}
fn sample(k: &Keyframe, t: f64) -> f64 {
    let data = serde_json::to_value(k).unwrap();
    let mut pts: Vec<_> = k.points.iter().map(|p| (p.t_ms as f64, p.value)).collect();
    if let Some(lead) = data.get("lead_in") {
        pts[0] = (
            lead["t_ms"].as_i64().unwrap() as f64,
            lead["value"].as_f64().unwrap(),
        );
    }
    if t < pts[0].0 {
        return pts[0].1;
    }
    for pair in pts.windows(2) {
        if t < pair[1].0 {
            return pair[0].1
                + (pair[1].1 - pair[0].1)
                    * k.interp.sample((t - pair[0].0) / (pair[1].0 - pair[0].0));
        }
    }
    pts.last().unwrap().1
}
#[test]
fn split_carries_hold_linear_and_every_eased_curve() {
    for interp in [
        "hold",
        "linear",
        "ease_in_quad",
        "ease_out_quad",
        "ease_in_out_quad",
        "ease_in_cubic",
        "ease_out_cubic",
        "ease_in_out_cubic",
        "ease_in_expo",
        "ease_out_expo",
        "ease_in_out_expo",
        "ease_in_back",
        "ease_out_back",
        "ease_in_out_back",
        "ease_in_elastic",
        "ease_out_elastic",
        "ease_in_out_elastic",
        "ease_in_bounce",
        "ease_out_bounce",
        "ease_in_out_bounce",
    ] {
        let mut p = fixture(interp);
        let original = keys(&p, 0).clone();
        let track = p.tracks[0].id.clone();
        edit::split(&mut p, &track, 1000).unwrap();
        for at in [0, 125, 500, 1000, 1500, 1999] {
            assert!(
                (sample(keys(&p, 1), at as f64) - sample(&original, (1000 + at) as f64)).abs()
                    < 1e-10,
                "{interp} right at {at}"
            );
        }
        for at in [0, 250, 750, 999] {
            assert_eq!(sample(keys(&p, 0), at as f64), sample(&original, at as f64));
        }
        let right = match &p.tracks[0].clips[1] {
            Clip::Media(c) => c.id.clone(),
            _ => panic!(),
        };
        edit::trim(&mut p, &right, Some(1250), None).unwrap();
        assert!(
            (sample(keys(&p, 1), 125.0) - sample(&original, 1375.0)).abs() < 1e-10,
            "{interp} repeated trim"
        );
        edit::speed(&mut p, &right, 2.0).unwrap();
        assert!(
            (sample(keys(&p, 1), 125.0) - sample(&original, 1500.0)).abs() < 1e-10,
            "{interp} retime after cut"
        );
    }
}
#[test]
fn ripple_and_lift_carry_the_right_hand_state() {
    for ripple in [true, false] {
        let mut p = fixture("linear");
        let original = keys(&p, 0).clone();
        edit::ripple_delete(&mut p, None, [750, 1250], ripple).unwrap();
        let right = if ripple { 1 } else { 2 };
        assert_eq!(sample(keys(&p, right), 250.0), sample(&original, 1500.0));
    }
}
#[test]
fn cutting_a_full_64_point_track_stays_bounded() {
    let mut p = fixture("ease_in_out_quad");
    let Clip::Media(c) = &mut p.tracks[0].clips[0] else {
        panic!()
    };
    c.keyframes[0].points = serde_json::from_value(json!((0..64)
        .map(|i| json!({"t_ms":i*40,"value":i as f64/100.0}))
        .collect::<Vec<_>>()))
    .unwrap();
    let original = c.keyframes[0].clone();
    let track = p.tracks[0].id.clone();
    edit::split(&mut p, &track, 110).unwrap();
    assert!(keys(&p, 1).points.len() <= 64);
    assert!((sample(keys(&p, 1), 15.0) - sample(&original, 125.0)).abs() < 1e-10);
    let mut short = fixture("ease_in_quad");
    let track = short.tracks[0].id.clone();
    edit::split(&mut short, &track, 1).unwrap();
    let id = match &short.tracks[0].clips[1] {
        Clip::Media(c) => c.id.clone(),
        _ => panic!(),
    };
    edit::speed(&mut short, &id, 4.0).unwrap();
    assert!(
        serde_json::to_value(keys(&short, 1)).unwrap()["lead_in"]["t_ms"]
            .as_i64()
            .unwrap()
            < 0
    );
}
