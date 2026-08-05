//! Shared brand-kit validation and verification adapters.

use super::*;

pub(super) fn normalize_brand(
    brand: cut_core::BrandKit,
    source: &'static str,
) -> Result<cut_core::BrandKit, CutError> {
    brand.normalized().map_err(|cause| {
        let (code, message, action) = if source == "stored" {
            (
                error_codes::CONFLICT,
                "saved brand kit is invalid",
                "clear or replace it with project.brand",
            )
        } else {
            (
                error_codes::INVALID_ARGS,
                "brand constraints are invalid",
                "correct the brand fields and retry",
            )
        };
        CutError::new(code, message, cause).with_suggested_action(action)
    })
}

fn perception_spec(brand: &cut_core::BrandKit) -> cut_perception::BrandSpec {
    cut_perception::BrandSpec {
        fonts: brand.fonts.clone(),
        colors: brand.colors.clone(),
        position: brand.position.clone(),
        min_size: brand.min_size,
        max_size: brand.max_size,
        aspect: brand.aspect_ratio(),
    }
}

pub(super) fn check_project_brand(
    project: &cut_core::Project,
    brand: &cut_core::BrandKit,
    source: &'static str,
) -> Value {
    let mut result = cut_perception::brand_check(
        &project.caption_styles,
        &project.settings,
        &perception_spec(brand),
    );
    if let Some(object) = result.as_object_mut() {
        object.insert("source".into(), Value::String(source.into()));
    }
    result
}

pub(super) fn check_bundle_brand(
    project: &cut_core::Project,
    brand: &cut_core::BrandKit,
    dims: &[(String, (u32, u32))],
    source: &'static str,
) -> Value {
    let spec = perception_spec(brand);
    let platform_results: Vec<Value> = dims
        .iter()
        .map(|(aspect, (width, height))| {
            let mut settings = project.settings.clone();
            settings.width = *width;
            settings.height = *height;
            let check = cut_perception::brand_check(&project.caption_styles, &settings, &spec);
            json!({
                "aspect": aspect,
                "width": width,
                "height": height,
                "check": check,
            })
        })
        .collect();
    let pass = platform_results
        .iter()
        .all(|result| result.pointer("/check/pass").and_then(Value::as_bool) == Some(true));
    json!({
        "pass": pass,
        "source": source,
        "platforms": platform_results,
        "brand": brand,
    })
}
