use super::*;
use std::io::Write;

fn write_tmp(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let p = dir.join(name);
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(bytes).unwrap();
    p
}

/// Full branch (file ≤ limit): exact, deterministic, "sha256:" tag; distinct
/// content → distinct hash. This is the unchanged identity for normal files.
#[test]
fn hash_full_branch_is_exact_and_deterministic() {
    let d = tempfile::tempdir().unwrap();
    let a = write_tmp(d.path(), "a.bin", b"hello shellx cut");
    let b = write_tmp(d.path(), "b.bin", b"hello shellx cut"); // identical content
    let c = write_tmp(d.path(), "c.bin", b"different content!"); // distinct
    let ha = hash_file(&a).unwrap();
    assert!(ha.starts_with("sha256:") && !ha.starts_with("sha256s:"));
    assert_eq!(ha, hash_file(&b).unwrap(), "same content → same hash");
    assert_ne!(
        ha,
        hash_file(&c).unwrap(),
        "distinct content → distinct hash"
    );
}

/// Sampled branch (file > limit): O(1) key, "sha256s:" tag, never equal to a
/// full hash; differs on head/tail/size. Driven with a tiny limit so a small
/// fixture exercises it (no absurd >256 MB CI file).
#[test]
fn hash_sampled_branch_keys_on_size_head_tail() {
    let d = tempfile::tempdir().unwrap();
    // 4 KB files; limit 1 KB, sample 256 B → sampled branch. Sampled regions
    // are bytes [0,256) (head) and [3840,4096) (tail); [256,3840) is the
    // unsampled middle. Build explicit head/middle/tail so each test varies
    // exactly one region.
    let mk = |name: &str, head_byte: u8, mid_byte: u8, tail_byte: u8| {
        let mut v = vec![mid_byte; 4096];
        for b in v[..256].iter_mut() {
            *b = head_byte;
        }
        for b in v[3840..].iter_mut() {
            *b = tail_byte;
        }
        write_tmp(d.path(), name, &v)
    };
    let base = mk("base.bin", 0xAA, 0x11, 0xBB);
    let same = mk("same.bin", 0xAA, 0x11, 0xBB); // identical
    let mid = mk("mid.bin", 0xAA, 0x22, 0xBB); // differs ONLY in the middle
    let head2 = mk("head2.bin", 0xCC, 0x11, 0xBB); // differs in head

    let hb = hash_file_impl(&base, 1024, 256).unwrap();
    assert!(hb.starts_with("sha256s:"), "big-file tag");
    assert_eq!(
        hb,
        hash_file_impl(&same, 1024, 256).unwrap(),
        "same sample → same key"
    );
    // Documented tradeoff: sampling can't see the middle — two files equal in
    // size+head+tail collide. Acceptable for a dedup/cache key on real media.
    assert_eq!(
        hb,
        hash_file_impl(&mid, 1024, 256).unwrap(),
        "middle-only diff collides (by design)"
    );
    assert_ne!(
        hb,
        hash_file_impl(&head2, 1024, 256).unwrap(),
        "head diff → distinct key"
    );
    // A sampled key never equals the full key of the same file.
    assert_ne!(hb, hash_file_impl(&base, u64::MAX, 256).unwrap());
}

#[test]
fn replay_format_rejects_oversized_dimensions_instead_of_wrapping() {
    let mut project = Project::new("demo", ProjectSettings::default());
    let op = OpRecord {
        op_id: "op_000002".into(),
        ts: "2026-06-29T00:00:00.000Z".into(),
        actor: Actor::system(),
        verb: "project.format".into(),
        args: json!({
            "width": u64::from(u32::MAX) + 1,
            "height": 720,
            "fps": 30.0
        }),
        rationale: None,
        effects: vec![],
        inverse: None,
        status: OpStatus::Applied,
    };

    let err = apply_record(&mut project, &op, &[]).expect_err("oversized replay width");
    assert_eq!(err.code, codes::CONFLICT);
    assert_eq!(project.settings.width, ProjectSettings::default().width);
}
