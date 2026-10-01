//! Default-output chapter serialization regression.

use super::*;

/// export.chapters: markers → a time-sorted "M:SS Label" chapter file.
#[test]
fn export_chapters_writes_sorted_chapter_list() {
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new();
        dispatch(
            &state,
            "project.create",
            json!({"name":"t","dir": dir.path().join("t.cutproj")}),
            test_actor(),
        )
        .await;
        // add out of order — export must time-sort.
        dispatch(
            &state,
            "edit.add_marker",
            json!({"at_ms": 65000, "label": "Part 2"}),
            test_actor(),
        )
        .await;
        dispatch(
            &state,
            "edit.add_marker",
            json!({"at_ms": 0, "label": "Intro"}),
            test_actor(),
        )
        .await;
        let r = dispatch(&state, "export.chapters", json!({}), test_actor()).await;
        assert!(r.ok, "{:?}", r.error);
        let res = r.result.unwrap();
        assert_eq!(res["chapter_count"], 2);
        assert_eq!(res["first_at_zero"], true);
        let content = std::fs::read_to_string(res["path"].as_str().unwrap()).unwrap();
        assert_eq!(
            content, "0:00 Intro\n1:05 Part 2\n",
            "sorted, M:SS formatted"
        );
    });
}
