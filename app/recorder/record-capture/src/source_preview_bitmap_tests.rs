use super::source_preview::MAX_SOURCE_PREVIEW_FRAME_BYTES;
use super::source_preview_bitmap::{bgra_bmp, native_bmp, SourcePreviewPixelFormat};
use super::CaptureRegion;

#[test]
fn emits_a_top_down_bmp_without_storing_a_file() {
    let source = [1, 2, 3, 4, 5, 6, 7, 8];
    let bmp = bgra_bmp(2, 1, 8, &source, None).unwrap();
    assert_eq!(&bmp[..2], b"BM");
    assert_eq!(bmp.len(), 54 + source.len());
    assert_eq!(&bmp[54..], &source);
    assert_eq!(i32::from_le_bytes(bmp[22..26].try_into().unwrap()), -1);
}

#[test]
fn strips_padded_source_rows_and_rejects_incomplete_frames() {
    let source = [1, 2, 3, 4, 9, 9, 9, 9, 5, 6, 7, 8, 9, 9, 9, 9];
    let bmp = bgra_bmp(1, 2, 8, &source, None).unwrap();
    assert_eq!(&bmp[54..], &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert!(bgra_bmp(2, 1, 8, &[0; 7], None).is_err());
}

#[test]
fn downscales_before_exceeding_the_preview_memory_budget() {
    let width = 2_000u32;
    let height = 1_000u32;
    let source = vec![7; width as usize * height as usize * 4];
    let bmp = bgra_bmp(width, height, width as usize * 4, &source, None).unwrap();
    assert!(bmp.len() <= MAX_SOURCE_PREVIEW_FRAME_BYTES);
    assert!(bmp.len() < source.len() + 54);
}

#[test]
fn crops_only_a_matching_private_display_region() {
    let source = [
        1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, 5, 0, 0, 0, 6, 0, 0, 0, 7, 0, 0, 0, 8, 0,
        0, 0,
    ];
    let region = CaptureRegion::new(2, 0, 2, 2, 4, 2).unwrap();
    let bmp = bgra_bmp(4, 2, 16, &source, Some(region)).unwrap();
    assert_eq!(i32::from_le_bytes(bmp[18..22].try_into().unwrap()), 2);
    assert_eq!(
        &bmp[54..],
        &[3, 0, 0, 0, 4, 0, 0, 0, 7, 0, 0, 0, 8, 0, 0, 0]
    );
    assert!(bgra_bmp(2, 2, 8, &[0; 16], Some(region)).is_err());
}

#[test]
fn converts_pipewire_rgb_rows_to_bounded_bgra_without_a_full_frame_copy() {
    let source = [10, 20, 30, 99, 40, 50, 60, 88];
    let bmp = native_bmp(2, 1, SourcePreviewPixelFormat::Rgbx, 8, &source, 0, None).unwrap();

    assert_eq!(&bmp[54..], &[30, 20, 10, 255, 60, 50, 40, 255]);
}

#[test]
fn rejects_a_pipewire_chunk_whose_offset_or_stride_exceeds_its_mapped_bytes() {
    assert!(native_bmp(1, 2, SourcePreviewPixelFormat::Bgrx, 8, &[0; 12], 1, None,).is_err());
}
