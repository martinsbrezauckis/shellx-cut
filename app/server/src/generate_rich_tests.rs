use crate::generate::{
    interpolate_args, registry, resolve_params, GenerateCatalog,
    GENERATE_RICH_MOTION_TEMPLATES_JSON,
};
use serde_json::{json, Map};

const RICH_FAMILIES: [(&str, &str, &str); 4] = [
    (
        "builtin.motion.cinematic-fog-title",
        "cinematic-fog-title",
        "fogDensity",
    ),
    (
        "builtin.motion.editorial-liquid-surface",
        "editorial-liquid-surface",
        "waveHeight",
    ),
    (
        "builtin.motion.keyed-subject-promo",
        "keyed-subject-promo",
        "spillSuppression",
    ),
    (
        "builtin.motion.tracked-callout-overlay",
        "tracked-callout-overlay",
        "calloutTitle",
    ),
];

#[test]
fn rich_motion_fragment_is_discoverable_and_lowers_to_product_pack_aliases() {
    let fragment: GenerateCatalog = serde_json::from_str(GENERATE_RICH_MOTION_TEMPLATES_JSON)
        .expect("rich Motion Generate fragment parses");
    assert_eq!(fragment.templates.len(), RICH_FAMILIES.len());
    assert!(registry().templates.len() >= 14);

    for (id, alias, distinguishing_param) in RICH_FAMILIES {
        let template = registry().get(id).expect("rich Motion family is listed");
        assert_eq!(template.kind, "motion");
        assert_eq!(template.lowering.verb, "motion.template_to_cut");
        assert_eq!(template.lowering.args["template"], json!(alias));
        assert!(template.params.contains_key(distinguishing_param));
        for capability in [
            "preview",
            "insert",
            "rendered_media",
            "motion_template",
            "quality_manifest",
            "text_fit",
        ] {
            assert!(
                template
                    .capabilities
                    .iter()
                    .any(|entry| entry == capability),
                "{id}:{capability}"
            );
        }
    }
}

#[test]
fn decimal_motion_controls_preserve_bounds_and_interpolate_as_numbers() {
    let template = registry()
        .get("builtin.motion.cinematic-fog-title")
        .expect("fog template");
    let density = template.params.get("fogDensity").expect("fog density");
    assert_eq!(density.minimum, Some(0.2));
    assert_eq!(density.maximum, Some(0.9));
    assert_eq!(density.step, Some(0.02));

    let mut request = Map::new();
    request.insert("title".to_string(), json!("Beyond local"));
    request.insert("fogDensity".to_string(), json!(0.74));
    let resolved = resolve_params(template, &request).expect("bounded decimal params resolve");
    let args = interpolate_args(template, &resolved, [0, 6000]).expect("lowering args interpolate");
    assert_eq!(args["params"]["fogDensity"], json!(0.74));
    assert_eq!(args["duration_ms"], json!(6000));

    request.insert("fogDensity".to_string(), json!(1.2));
    let error = resolve_params(template, &request).expect_err("out-of-range density must fail");
    assert!(error.message.contains("outside its allowed range"));
}
