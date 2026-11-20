//! Guard: a media test process never resolves the person's real per-user
//! locations (tools folder, private work folder, state home).
use cut_core::test_isolation::is_isolated_path;

cut_core::isolate_test_process!();

#[test]
fn per_user_locations_resolve_inside_the_test_process_folder() {
    let tools = cut_media::toolpath::appdata_tools_dir().expect("tools dir resolves");
    assert!(
        is_isolated_path(&tools),
        "tools folder is outside the test folder: {tools:?}"
    );
    let work = cut_media::private_work::work_root().expect("work root resolves");
    assert!(
        is_isolated_path(&work),
        "private work folder is outside the test folder: {work:?}"
    );
    let home = std::env::var_os("SHELLX_CUT_HOME")
        .map(std::path::PathBuf::from)
        .unwrap();
    assert!(
        is_isolated_path(&home),
        "SHELLX_CUT_HOME is outside the test folder: {home:?}"
    );
}
