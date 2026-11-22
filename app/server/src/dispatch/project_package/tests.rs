#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::dispatch;
    use cut_core::Asset;
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use std::time::Duration;

    #[tokio::test]
    async fn package_carries_named_files_windows_take_id_and_copy_of_copy() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        let project_dir = root.path().join("source.cutproj");
        assert!(
            dispatch(
                &state,
                "project.create",
                json!({"name":"source","dir":project_dir}),
                actor()
            )
            .await
            .ok
        );
        let capture = project_dir.join("cache/screen_record/cap-copy");
        let take_id = format!(
            "take-{:x}",
            Sha256::digest(b"cache\\screen_record\\cap-copy")
        );
        fs::create_dir_all(capture.join("camera")).unwrap();
        for (name, bytes) in [
            ("source.mp4", "screen"),
            ("events.json", "[]"),
            ("camera/camera.mp4", "camera"),
            ("mic.wav", "mic"),
        ] {
            fs::write(capture.join(name), bytes).unwrap();
        }
        fs::write(capture.join("private-notes.txt"), b"do not share").unwrap();
        let plan_path =
            project_dir.join(format!("cache/screen_record/plan-{}.json", "a".repeat(64)));
        let mut edit_plan = record_core::EditPlan::empty(640, 360, 1000, 30.0);
        edit_plan.webcam = Some(record_core::WebcamOverlay {
            source: capture.join("camera/camera.mp4").display().to_string(),
            shape: record_core::WebcamShape::Circle,
            anchor: record_core::Anchor::BottomRight,
            margin: 0.05,
            size: 0.3,
            camera_clock: None,
            timeline: vec![],
        });
        fs::write(&plan_path, serde_json::to_vec(&edit_plan).unwrap()).unwrap();
        let applied_plan =
            project_dir.join(format!("cache/screen_record/plan-{}.json", "b".repeat(64)));
        fs::write(&applied_plan, serde_json::to_vec(&edit_plan).unwrap()).unwrap();
        let baked = project_dir.join("baked.mp4");
        fs::write(&baked, b"baked").unwrap();
        {
            let mut guard = state.project.write().await;
            let store = guard.as_mut().unwrap();
            store
                .record_import(
                    Some("a1".into()),
                    Asset {
                        path: baked.display().to_string(),
                        hash: format!("sha256:{:x}", Sha256::digest(b"baked")),
                        probe: None,
                        transcript: None,
                        perception: None,
                        proxy: None,
                        filmstrip: None,
                    },
                    actor(),
                    None,
                )
                .unwrap();
            store.apply("edit.insert", json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,1000],"ripple":false}), actor(), None).unwrap();
            store.apply("screen_record.take_link", json!({"recording":{"id":take_id,"capture_folder":capture,"source":capture.join("source.mp4"),"events":capture.join("events.json"),"pending_plan":plan_path,"applied_plan":applied_plan,"clip_ids":[],"asset":"a1","camera_artifact":{"video":"camera/camera.mp4"},"segments":[]}}), actor(), None).unwrap();
        }
        let dest = root.path().join("copies");
        fs::create_dir(&dest).unwrap();
        // A missing named input drops this take, while its rendered clip stays.
        let event_path = capture.join("events.json");
        fs::remove_file(&event_path).unwrap();
        let missing = prepare(&state, dest.display().to_string(), "missing".into(), None)
            .await
            .unwrap();
        assert_eq!(missing.plan.recordings_not_carried.len(), 1);
        fs::create_dir(&event_path).unwrap();
        let omitted = prepare(&state, dest.display().to_string(), "omitted".into(), None)
            .await
            .unwrap();
        assert_eq!(omitted.plan.recordings_not_carried.len(), 1);
        assert_eq!(omitted.plan.recordings_not_carried[0].take, take_id);
        assert!(omitted.plan.recordings_not_carried[0]
            .reason
            .contains("events"));
        let omitted_job = dispatch(
            &state,
            "project.package_create",
            json!({
                "destination":dest,"name":"omitted","plan_hash":hash_json(&omitted.plan).unwrap()
            }),
            actor(),
        )
        .await;
        assert!(omitted_job.ok, "{:?}", omitted_job.error);
        let omitted_job = wait_for_job(
            &state,
            omitted_job.result.unwrap()["job_id"].as_str().unwrap(),
        )
        .await;
        assert_eq!(
            omitted_job.state,
            crate::jobs::JobState::Done,
            "{:?}",
            omitted_job.error
        );
        assert_eq!(
            omitted_job.result.as_ref().unwrap()["recordings_not_carried"][0]["take"],
            take_id
        );
        let ordinary_copy = ProjectStore::open(&dest.join("omitted.cutproj")).unwrap();
        assert!(ordinary_copy.project.recordings.is_empty());
        assert!(!ordinary_copy.project.tracks[0].clips.is_empty());
        drop(ordinary_copy);
        fs::remove_dir(&event_path).unwrap();
        fs::write(&event_path, b"[]").unwrap();
        let prepared = prepare(&state, dest.display().to_string(), "copy".into(), None)
            .await
            .unwrap();
        let mut unsafe_plan = prepared.clone();
        unsafe_plan.files[0].package_path = "../../../escaped.txt".into();
        let escaped = root.path().join("escaped.txt");
        assert!(publish_package(
            unsafe_plan,
            &crate::jobs::JobCancellation::test_active(),
            |_, _| {},
            |_, _| Ok(()),
        )
        .is_err());
        assert!(!escaped.exists());
        assert!(
            prepared.plan.total_source_bytes
                >= 5 + 6 + 2 + 6 + 3 + fs::metadata(&plan_path).unwrap().len(),
            "copy plan must count recording files"
        );
        let created = dispatch(&state, "project.package_create", json!({"destination":dest,"name":"copy","plan_hash":hash_json(&prepared.plan).unwrap()}), actor()).await;
        assert!(created.ok, "{:?}", created.error);
        let job = wait_for_job(&state, created.result.unwrap()["job_id"].as_str().unwrap()).await;
        assert_eq!(job.state, crate::jobs::JobState::Done, "{:?}", job.error);
        let copied_dir = dest.join("copy.cutproj");
        let copy = ProjectStore::open(&copied_dir).unwrap();
        let take = &copy.project.recordings[0];
        assert_ne!(take.pending_plan, take.applied_plan);
        assert!(copied_dir.join(&take.applied_plan).is_file());
        assert_eq!(fs::read(copied_dir.join(&take.source)).unwrap(), b"screen");
        assert_eq!(fs::read(copied_dir.join(&take.events)).unwrap(), b"[]");
        assert_eq!(
            fs::read(
                copied_dir
                    .join(&take.capture_folder)
                    .join("camera/camera.mp4")
            )
            .unwrap(),
            b"camera"
        );
        assert_eq!(
            fs::read(copied_dir.join(&take.capture_folder).join("mic.wav")).unwrap(),
            b"mic"
        );
        assert!(!copied_dir
            .join(&take.capture_folder)
            .join("private-notes.txt")
            .exists());
        let plan = crate::screen_record::plan_inputs::load(
            &copied_dir,
            &copy.project,
            &copied_dir.join(&take.pending_plan),
        )
        .unwrap();
        // Compare resolved paths: on Windows the loaded plan carries the verbatim (\\?\) form.
        let camera_in_copy = Path::new(&plan.webcam.unwrap().source)
            .canonicalize()
            .unwrap();
        assert!(camera_in_copy.starts_with(copied_dir.canonicalize().unwrap()));
        assert!(!serde_json::to_string(&copy.project)
            .unwrap()
            .contains(project_dir.to_str().unwrap()));
        drop(copy);
        let copied_state = AppState::new();
        let opened = dispatch(
            &copied_state,
            "project.open",
            json!({"path":copied_dir}),
            actor(),
        )
        .await;
        assert!(opened.ok, "{:?}", opened.error);
        let again_dest = root.path().join("copies-again");
        fs::create_dir(&again_dest).unwrap();
        let again = prepare(
            &copied_state,
            again_dest.display().to_string(),
            "copy-again".into(),
            None,
        )
        .await
        .unwrap();
        let created_again = dispatch(
            &copied_state,
            "project.package_create",
            json!({"destination":again_dest,"name":"copy-again","plan_hash":hash_json(&again.plan).unwrap()}),
            actor(),
        )
        .await;
        assert!(created_again.ok, "{:?}", created_again.error);
        let second_job = wait_for_job(
            &copied_state,
            created_again.result.unwrap()["job_id"].as_str().unwrap(),
        )
        .await;
        assert_eq!(
            second_job.state,
            crate::jobs::JobState::Done,
            "{:?}",
            second_job.error
        );

        // Change a valid plan after its hash has been admitted. Length and
        // modification time still match, including on Windows.
        let original_plan = fs::read(&plan_path).unwrap();
        let original_stamp = file_stamp(&fs::metadata(&plan_path).unwrap());
        let mut changed_plan = original_plan.clone();
        let width = changed_plan.windows(3).position(|bytes| bytes == b"640").unwrap();
        changed_plan[width + 2] = b'1';
        assert_eq!(changed_plan.len(), original_plan.len());
        assert!(serde_json::from_slice::<record_core::EditPlan>(&changed_plan).is_ok());
        let source = snapshot_source(&state).await.unwrap();
        let changed_target = dest.join("changed-plan.cutproj");
        let changed_take_id = take_id.clone();
        let changed_plan_path = plan_path.clone();
        let prepared = build_prepared_with_recordings(
            source,
            dest.clone(),
            changed_target.clone(),
            "changed-plan".into(),
            None,
            move |source, files| {
                let take = &source.project.recordings[0];
                let result = add_one_take_after_plan_admission(source, take, || {
                    fs::write(&changed_plan_path, &changed_plan).unwrap();
                    File::options()
                        .write(true)
                        .open(&changed_plan_path)
                        .unwrap()
                        .set_modified(original_stamp.modified.unwrap())
                        .unwrap();
                    assert_eq!(
                        file_stamp(&fs::metadata(&changed_plan_path).unwrap()),
                        original_stamp
                    );
                });
                match result {
                    Ok(named) => {
                        files.extend(named);
                        vec![]
                    }
                    Err(reason) => vec![RecordingNotCarried {
                        take: take.id.clone(),
                        reason,
                    }],
                }
            },
        )
        .unwrap();
        assert_eq!(prepared.plan.recordings_not_carried.len(), 1);
        assert_eq!(prepared.plan.recordings_not_carried[0].take, changed_take_id);
        assert_eq!(
            prepared.plan.recordings_not_carried[0].reason,
            "plan changed while making the copy"
        );
        let result = publish_package(
            prepared,
            &crate::jobs::JobCancellation::test_active(),
            |_, _| {},
            publish_new_directory,
        )
        .unwrap();
        assert_eq!(result.recordings_not_carried.len(), 1);
        assert_eq!(result.recordings_not_carried[0].take, changed_take_id);
        let changed_copy = ProjectStore::open(&changed_target).unwrap();
        assert!(changed_copy.project.recordings.is_empty());
        assert!(!changed_copy.project.tracks[0].clips.is_empty());
    }

    fn actor() -> Actor {
        Actor {
            kind: cut_core::ActorKind::Agent,
            name: "portable-test".into(),
            via: "test".into(),
            request: None,
        }
    }

    async fn wait_for_job(state: &AppState, job_id: &str) -> crate::jobs::JobRecord {
        for _ in 0..100 {
            let record = state.jobs.get(job_id).expect("package job exists");
            if matches!(
                record.state,
                crate::jobs::JobState::Done | crate::jobs::JobState::Failed
            ) {
                return record;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("portable package job did not finish")
    }

    fn spawn_bounded_package_worker(
        state: &AppState,
        timeout: Duration,
        work: impl FnOnce(crate::jobs::JobCancellation) -> Result<(), CutError> + Send + 'static,
    ) -> String {
        let job = state.jobs.create(PACKAGE_JOB_KIND);
        let job_id = job.job_id.clone();
        let task_id = job_id.clone();
        let jobs = state.jobs.clone();
        state
            .jobs
            .spawn_limited(&job_id, PACKAGE_JOB_KIND, 1, async move {
                jobs.progress(
                    &task_id,
                    0.02,
                    Some("validating portable package destination".into()),
                );
                let bounded = await_bounded_package_work(
                    timeout,
                    crate::dispatch::run_blocking_cancellable(
                        "project.package_create.timeout-test",
                        work,
                    ),
                )
                .await;
                match bounded {
                    Ok(()) => jobs.finish(&task_id, json!({"status": "published"})),
                    Err(error) => jobs.fail(&task_id, error),
                }
            });
        job_id
    }

    #[test]
    fn package_timeout_gives_zero_media_packages_a_prompt_deadline_and_scales_for_media() {
        assert_eq!(package_timeout(0), Duration::from_secs(15));
        assert_eq!(
            package_timeout(PACKAGE_BYTES_PER_SECOND + 1),
            Duration::from_secs(17),
        );
        assert_eq!(
            package_timeout(u64::MAX),
            PACKAGE_MAX_TIMEOUT,
            "large packages must remain bounded",
        );
    }

    #[test]
    fn package_hashing_fits_a_bounded_blocking_worker_stack() {
        let root = tempfile::tempdir().unwrap();
        let fixture = root.path().join("fixture.bin");
        let bytes = vec![0x5a; 2 * 1024 * 1024];
        fs::write(&fixture, &bytes).unwrap();
        let expected = format!("{:x}", Sha256::digest(&bytes));

        let result = std::thread::Builder::new()
            .name("portable-package-bounded-stack".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                let mut file = open_plain_regular(&fixture).unwrap();
                stream_sha256(&mut file, None).unwrap()
            })
            .unwrap()
            .join()
            .unwrap();

        assert_eq!(result, (bytes.len() as u64, expected));
    }

    /// A filesystem worker may remain blocked after cancellation. The job must
    /// still terminalize promptly, while the worker remains confined to its
    /// private stage, so later project close/create calls stay available.
    #[tokio::test]
    async fn package_deadline_keeps_runtime_live_while_worker_stops_later() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        let source = dispatch(
            &state,
            "project.create",
            json!({"name":"source", "dir":root.path().join("source.cutproj")} ),
            actor(),
        )
        .await;
        assert!(source.ok, "{:?}", source.error);

        let worker_stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped_for_job = worker_stopped.clone();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let job_id =
            spawn_bounded_package_worker(&state, Duration::from_millis(20), move |_cancel| {
                release_rx.recv().unwrap();
                worker_stopped_for_job.store(true, Ordering::SeqCst);
                Ok(())
            });

        // The bounded Tokio-time poll proves prompt terminalization without a
        // host wall-clock assertion that becomes scheduler-sensitive when the
        // complete Windows suite runs many blocking tests in parallel.
        let terminal = wait_for_job(&state, &job_id).await;
        assert_eq!(terminal.state, crate::jobs::JobState::Failed);
        assert_eq!(
            terminal.error.as_ref().map(|error| error.code.as_str()),
            Some(error_codes::JOB_FAILED),
        );
        assert_eq!(
            terminal.error.as_ref().map(|error| error.message.as_str()),
            Some("portable package timed out"),
        );
        assert!(!worker_stopped.load(Ordering::SeqCst));

        let closed = dispatch(&state, "project.close", json!({}), actor()).await;
        assert!(closed.ok, "{:?}", closed.error);
        let next = dispatch(
            &state,
            "project.create",
            json!({"name":"next", "dir":root.path().join("next.cutproj")} ),
            actor(),
        )
        .await;
        assert!(next.ok, "{:?}", next.error);

        release_tx.send(()).unwrap();
        for _ in 0..100 {
            if worker_stopped.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("detached package worker did not stop after its test release");
    }

    #[tokio::test]
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    async fn packages_only_referenced_media_without_mutating_source_ops_or_cache() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let destination = root_path.join("packages");
        fs::create_dir(&destination).unwrap();
        let source = root_path.join("source.mov");
        let duplicate = root_path.join("duplicate.mov");
        fs::write(&source, b"portable package fixture").unwrap();
        fs::write(&duplicate, b"portable package fixture").unwrap();
        let source = source.canonicalize().unwrap();
        let duplicate = duplicate.canonicalize().unwrap();
        let source_hash = format!("sha256:{:x}", Sha256::digest(b"portable package fixture"));
        let state = AppState::new();
        let created = dispatch(
            &state,
            "project.create",
            json!({"name":"source", "dir":root_path.join("source.cutproj")}),
            actor(),
        )
        .await;
        assert!(created.ok, "{:?}", created.error);
        let (source_dir, before_log, before_cache, revision) = {
            let mut guard = state.project.write().await;
            let store = guard.as_mut().unwrap();
            store
                .record_import(
                    Some("a1".into()),
                    Asset {
                        path: source.to_string_lossy().into_owned(),
                        hash: source_hash,
                        probe: Some(json!({"duration_ms": 100})),
                        transcript: Some("receipts/a1.words.json".into()),
                        perception: None,
                        proxy: Some("proxies/a1.mp4".into()),
                        filmstrip: None,
                    },
                    actor(),
                    None,
                )
                .unwrap();
            store
                .record_import(
                    Some("a2".into()),
                    Asset {
                        path: duplicate.to_string_lossy().into_owned(),
                        hash: format!("sha256:{:x}", Sha256::digest(b"portable package fixture")),
                        probe: None,
                        transcript: None,
                        perception: None,
                        proxy: None,
                        filmstrip: None,
                    },
                    actor(),
                    None,
                )
                .unwrap();
            store
                .apply(
                    "edit.insert",
                    json!({"asset":"a1", "track":"v1", "at_ms":0, "src_range_ms":[0,100], "ripple":false}),
                    actor(),
                    None,
                )
                .unwrap();
            store
                .apply(
                    "edit.insert",
                    json!({"asset":"a2", "track":"v1", "at_ms":100, "src_range_ms":[0,100], "ripple":false}),
                    actor(),
                    None,
                )
                .unwrap();
            (
                store.dir.clone(),
                fs::read(store.dir.join("ops.jsonl")).unwrap(),
                fs::read(store.dir.join("project.json")).unwrap(),
                store.log.current_revision().unwrap().unwrap(),
            )
        };
        let planned = dispatch(
            &state,
            "project.package_plan",
            json!({"destination":destination, "name":"packed"}),
            actor(),
        )
        .await;
        assert!(planned.ok, "{:?}", planned.error);
        let plan = planned.result.unwrap();
        assert_eq!(plan["plan"]["unique_media_count"], 1);
        assert_eq!(plan["plan"]["source_file_count"], 2);
        assert_eq!(
            plan["plan"]["assets"][0]["source_path"],
            source.to_string_lossy().as_ref()
        );
        assert_eq!(
            plan["plan"]["assets"][1]["source_path"],
            duplicate.to_string_lossy().as_ref()
        );
        let mut other_source = plan["plan"].clone();
        other_source["assets"][0]["source_path"] = json!(duplicate.to_string_lossy());
        assert_ne!(hash_json(&other_source).unwrap(), plan["plan_hash"]);
        let created = dispatch(
            &state,
            "project.package_create",
            json!({"destination":destination, "name":"packed", "plan_hash":plan["plan_hash"]}),
            actor(),
        )
        .await;
        assert!(created.ok, "{:?}", created.error);
        let job = wait_for_job(&state, created.result.unwrap()["job_id"].as_str().unwrap()).await;
        assert_eq!(job.state, crate::jobs::JobState::Done, "{:?}", job.error);
        let packed = destination.join("packed.cutproj");
        assert!(packed.join("package.manifest.json").is_file());
        assert!(!packed.join("proxies").exists());
        assert!(!packed.join("receipts").exists());
        let reopened = ProjectStore::open(&packed).unwrap();
        assert_eq!(reopened.project.assets.len(), 2);
        assert!(reopened.project.assets["a1"]
            .path
            .starts_with("media/sha256/"));
        assert_eq!(
            reopened.project.assets["a1"].path,
            reopened.project.assets["a2"].path
        );
        assert!(reopened.project.assets["a1"].proxy.is_none());
        assert!(!fs::read(packed.join("ops.jsonl"))
            .unwrap()
            .windows(source.to_string_lossy().len())
            .any(|chunk| chunk == source.to_string_lossy().as_bytes()));
        assert_eq!(fs::read(source_dir.join("ops.jsonl")).unwrap(), before_log);
        assert_eq!(
            fs::read(source_dir.join("project.json")).unwrap(),
            before_cache
        );
        let source_revision = state
            .project
            .read()
            .await
            .as_ref()
            .unwrap()
            .log
            .current_revision()
            .unwrap()
            .unwrap();
        assert_eq!(source_revision, revision);
    }

    #[tokio::test]
    async fn plan_reports_current_destination_collision_without_creating_a_job() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("packages");
        fs::create_dir(&destination).unwrap();
        let state = AppState::new();
        assert!(
            dispatch(
                &state,
                "project.create",
                json!({"name":"source", "dir":root.path().join("source.cutproj")}),
                actor(),
            )
            .await
            .ok
        );

        let available = dispatch(
            &state,
            "project.package_plan",
            json!({"destination":destination, "name":"packed"}),
            actor(),
        )
        .await;
        assert!(available.ok, "{:?}", available.error);
        assert_eq!(
            available.result.as_ref().unwrap()["plan"]["target_status"],
            "available"
        );

        fs::create_dir(destination.join("packed.cutproj")).unwrap();
        let occupied = dispatch(
            &state,
            "project.package_plan",
            json!({"destination":destination, "name":"packed"}),
            actor(),
        )
        .await;
        assert!(occupied.ok, "{:?}", occupied.error);
        assert_eq!(
            occupied.result.as_ref().unwrap()["plan"]["target_status"],
            "occupied"
        );
        assert_ne!(
            available.result.unwrap()["plan_hash"],
            occupied.result.unwrap()["plan_hash"]
        );
        assert!(state.jobs.list().is_empty());
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn plan_refuses_source_path_that_cannot_be_shown_exactly() {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("packages");
        fs::create_dir(&destination).unwrap();
        let target = root
            .path()
            .join(std::ffi::OsStr::from_bytes(b"hidden-\x80.mov"));
        let alias = root.path().join("alias.mov");
        fs::write(&target, b"media fixture").unwrap();
        symlink(&target, &alias).unwrap();
        let state = AppState::new();
        assert!(
            dispatch(
                &state,
                "project.create",
                json!({"name":"source", "dir":root.path().join("source.cutproj")}),
                actor(),
            )
            .await
            .ok
        );
        {
            let mut guard = state.project.write().await;
            let store = guard.as_mut().unwrap();
            store
                .record_import(
                    Some("a1".into()),
                    Asset {
                        path: alias.to_string_lossy().into_owned(),
                        hash: format!("sha256:{:x}", Sha256::digest(b"media fixture")),
                        probe: None,
                        transcript: None,
                        perception: None,
                        proxy: None,
                        filmstrip: None,
                    },
                    actor(),
                    None,
                )
                .unwrap();
            store.apply(
                "edit.insert",
                json!({"asset":"a1", "track":"v1", "at_ms":0, "src_range_ms":[0,100], "ripple":false}),
                actor(),
                None,
            ).unwrap();
        }
        let planned = dispatch(
            &state,
            "project.package_plan",
            json!({"destination":destination, "name":"packed"}),
            actor(),
        )
        .await;
        assert!(!planned.ok);
        assert_eq!(planned.error.unwrap().code, error_codes::INVALID_ARGS);
        assert!(state.jobs.list().is_empty());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn plan_reports_a_dangling_destination_symlink_as_occupied_without_creating_a_job() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("packages");
        fs::create_dir(&destination).unwrap();
        let state = AppState::new();
        assert!(
            dispatch(
                &state,
                "project.create",
                json!({"name":"source", "dir":root.path().join("source.cutproj")}),
                actor(),
            )
            .await
            .ok
        );

        let target = destination.join("packed.cutproj");
        symlink(root.path().join("missing-package"), &target).unwrap();
        assert!(fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(!target.exists(), "fixture must remain a dangling link");

        let plan = dispatch(
            &state,
            "project.package_plan",
            json!({"destination":destination, "name":"packed"}),
            actor(),
        )
        .await;
        assert!(plan.ok, "{:?}", plan.error);
        assert_eq!(
            plan.result.as_ref().unwrap()["plan"]["target_status"],
            "occupied"
        );
        assert!(state.jobs.list().is_empty());
    }

    #[tokio::test]
    async fn rejects_a_stale_b5_receipt_before_creating_a_job() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("packages");
        fs::create_dir(&destination).unwrap();
        let state = AppState::new();
        let created = dispatch(
            &state,
            "project.create",
            json!({"name":"source", "dir":root.path().join("source.cutproj")}),
            actor(),
        )
        .await;
        assert!(created.ok);
        let plan = dispatch(
            &state,
            "project.package_plan",
            json!({
                "destination":destination,
                "name":"packed",
                "b5_receipt": {"schema":B5_RECEIPT_SCHEMA, "immutable":true, "post_revision":"stale"}
            }),
            actor(),
        )
        .await;
        assert!(!plan.ok);
        assert_eq!(plan.error.unwrap().code, error_codes::CONFLICT);
        assert!(state.jobs.list().is_empty());
    }

    #[tokio::test]
    async fn consumes_the_committed_b5_receipt_instead_of_matching_media_again() {
        let root = tempfile::tempdir().unwrap();
        let recovery = root.path().join("recovery");
        let destination = root.path().join("packages");
        fs::create_dir(&recovery).unwrap();
        fs::create_dir(&destination).unwrap();
        let restored = recovery.join("restored.mov");
        fs::write(&restored, b"B5-to-B6 exact fixture").unwrap();
        let hash = format!("sha256:{:x}", Sha256::digest(b"B5-to-B6 exact fixture"));
        let state = AppState::new();
        let created = dispatch(
            &state,
            "project.create",
            json!({"name":"source", "dir":root.path().join("source.cutproj")}),
            actor(),
        )
        .await;
        assert!(created.ok);
        {
            let mut guard = state.project.write().await;
            let store = guard.as_mut().unwrap();
            store
                .record_import(
                    Some("a1".into()),
                    Asset {
                        path: root
                            .path()
                            .join("missing.mov")
                            .to_string_lossy()
                            .into_owned(),
                        hash,
                        probe: None,
                        transcript: None,
                        perception: None,
                        proxy: None,
                        filmstrip: None,
                    },
                    actor(),
                    None,
                )
                .unwrap();
            store
                .apply(
                    "edit.insert",
                    json!({"asset":"a1", "track":"v1", "at_ms":0, "src_range_ms":[0,100], "ripple":false}),
                    actor(),
                    None,
                )
                .unwrap();
        }
        let preview = dispatch(
            &state,
            "media.relink_preview",
            json!({"root":recovery}),
            actor(),
        )
        .await;
        assert!(preview.ok, "{:?}", preview.error);
        let preview = preview.result.unwrap();
        let receipt = dispatch(
            &state,
            "media.relink_apply",
            json!({
                "root":recovery,
                "plan_hash":preview["plan_hash"],
                "accept":["a1"],
                "request_id":"b6-b5-receipt-001",
                "expected_revision":preview["project_revision"],
            }),
            actor(),
        )
        .await;
        assert!(receipt.ok, "{:?}", receipt.error);
        let receipt = receipt.result.unwrap();
        let missing = dispatch(
            &state,
            "project.package_plan",
            json!({"destination":destination, "name":"packed"}),
            actor(),
        )
        .await;
        assert!(
            !missing.ok,
            "a current B5 relink must require its durable receipt"
        );
        assert_eq!(missing.error.unwrap().code, error_codes::CONFLICT);

        let mut tampered = receipt.clone();
        tampered["assets"][0]["chosen"]["path"] = json!("/tampered/restored.mov");
        let tampered_plan = dispatch(
            &state,
            "project.package_plan",
            json!({"destination":destination, "name":"packed", "b5_receipt":tampered}),
            actor(),
        )
        .await;
        assert!(
            !tampered_plan.ok,
            "a B5 receipt cannot be substituted or edited"
        );
        assert_eq!(tampered_plan.error.unwrap().code, error_codes::CONFLICT);

        let source_dir = state.project.read().await.as_ref().unwrap().dir.clone();
        let reopened = ProjectStore::open(&source_dir).unwrap();
        *state.project.write().await = Some(reopened);
        let reopened_state = dispatch(&state, "project.state", json!({}), actor()).await;
        assert!(reopened_state.ok, "{:?}", reopened_state.error);
        let durable_receipt = reopened_state.result.unwrap()["portable_b5_receipt"].clone();
        assert_eq!(
            durable_receipt, receipt,
            "reopen must reconstruct the exact B5 receipt"
        );
        let planned = dispatch(
            &state,
            "project.package_plan",
            json!({"destination":destination, "name":"packed", "b5_receipt":durable_receipt}),
            actor(),
        )
        .await;
        assert!(planned.ok, "{:?}", planned.error);
        assert!(planned.result.unwrap()["plan"]["b5_receipt_sha256"]
            .as_str()
            .is_some_and(is_exact_sha256));
    }

    #[tokio::test]
    async fn refuses_offline_referenced_media_before_creating_a_job() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("packages");
        fs::create_dir(&destination).unwrap();
        let state = AppState::new();
        let created = dispatch(
            &state,
            "project.create",
            json!({"name":"source", "dir":root.path().join("source.cutproj")}),
            actor(),
        )
        .await;
        assert!(created.ok);
        {
            let mut guard = state.project.write().await;
            let store = guard.as_mut().unwrap();
            store
                .record_import(
                    Some("a1".into()),
                    Asset {
                        path: root
                            .path()
                            .join("offline.mov")
                            .to_string_lossy()
                            .into_owned(),
                        hash: format!("sha256:{}", "a".repeat(64)),
                        probe: None,
                        transcript: None,
                        perception: None,
                        proxy: None,
                        filmstrip: None,
                    },
                    actor(),
                    None,
                )
                .unwrap();
            store
                .apply(
                    "edit.insert",
                    json!({"asset":"a1", "track":"v1", "at_ms":0, "src_range_ms":[0,100], "ripple":false}),
                    actor(),
                    None,
                )
                .unwrap();
        }
        let planned = dispatch(
            &state,
            "project.package_plan",
            json!({"destination":destination, "name":"packed"}),
            actor(),
        )
        .await;
        assert!(!planned.ok);
        assert_eq!(planned.error.unwrap().code, error_codes::NOT_FOUND);
        assert!(state.jobs.list().is_empty());
        assert!(fs::read_dir(&destination).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn cancellation_after_media_copy_removes_the_private_stage() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("packages");
        fs::create_dir(&destination).unwrap();
        let source = root.path().join("source.mov");
        fs::write(&source, b"cancel after copy").unwrap();
        let state = AppState::new();
        assert!(
            dispatch(
                &state,
                "project.create",
                json!({"name":"source", "dir":root.path().join("source.cutproj")}),
                actor(),
            )
            .await
            .ok
        );
        {
            let mut guard = state.project.write().await;
            let store = guard.as_mut().unwrap();
            store
                .record_import(
                    Some("a1".into()),
                    Asset {
                        path: source.to_string_lossy().into_owned(),
                        hash: format!("sha256:{:x}", Sha256::digest(b"cancel after copy")),
                        probe: None,
                        transcript: None,
                        perception: None,
                        proxy: None,
                        filmstrip: None,
                    },
                    actor(),
                    None,
                )
                .unwrap();
            store
                .apply(
                    "edit.insert",
                    json!({"asset":"a1", "track":"v1", "at_ms":0, "src_range_ms":[0,100], "ripple":false}),
                    actor(),
                    None,
                )
                .unwrap();
        }
        let prepared = prepare(
            &state,
            destination.to_string_lossy().into_owned(),
            "packed".into(),
            None,
        )
        .await
        .unwrap();
        let cancellation = crate::jobs::JobCancellation::test_active();
        let progress_cancellation = cancellation.clone();
        let result = publish_package(
            prepared,
            &cancellation,
            move |progress, _| {
                if progress >= 0.75 {
                    progress_cancellation.request_cancel();
                }
            },
            |_, _| panic!("cancelled package must not reach publication"),
        );
        let error = match result {
            Ok(_) => panic!("cancelled package unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.code, "job_cancelled");
        assert!(fs::read_dir(&destination).unwrap().next().is_none());
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    fn directory_publication_never_replaces_a_racing_destination() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join("stage.cutproj");
        let target = root.path().join("target.cutproj");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("stage.txt"), b"stage").unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("existing.txt"), b"existing").unwrap();
        assert!(publish_new_directory(&stage, &target).is_err());
        assert!(stage.join("stage.txt").is_file());
        assert_eq!(fs::read(target.join("existing.txt")).unwrap(), b"existing");
    }

    #[test]
    fn cleanup_retains_a_stage_with_an_unexpected_entry() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join(".packed.package-test.stage");
        let package = stage.join("packed.cutproj");
        fs::create_dir(&stage).unwrap();
        fs::create_dir(&package).unwrap();
        fs::write(package.join("unexpected.txt"), b"do not delete").unwrap();
        cleanup_stage(&stage, "packed");
        assert!(stage.exists());
        assert!(package.join("unexpected.txt").is_file());
    }
}
