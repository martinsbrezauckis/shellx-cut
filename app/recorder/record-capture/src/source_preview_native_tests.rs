use super::source_preview_native::{
    NativeSourcePreviewSession, SourcePreviewMailbox, SourcePreviewMailboxTerminal,
};

#[test]
fn mailbox_retains_only_the_latest_bounded_native_frame() {
    let mailbox = SourcePreviewMailbox::default();
    mailbox
        .publish_bgra(1, 1, 1, 4, &[1, 2, 3, 4], None)
        .unwrap();
    mailbox
        .publish_bgra(2, 1, 1, 4, &[5, 6, 7, 8], None)
        .unwrap();

    let frame = mailbox.latest().unwrap();
    assert_eq!(frame.captured_at_ms(), 2);
    assert_eq!(&frame.encoded()[54..], &[5, 6, 7, 8]);
}

#[test]
fn session_stop_releases_its_frame_and_runs_once() {
    let mailbox = SourcePreviewMailbox::default();
    mailbox
        .publish_bgra(1, 1, 1, 4, &[1, 2, 3, 4], None)
        .unwrap();
    let stopped = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let stopped_for_closure = stopped.clone();
    let mut session = NativeSourcePreviewSession::new(7, mailbox.clone(), move || {
        stopped_for_closure.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    });

    session.stop().unwrap();
    session.stop().unwrap();
    assert!(session.latest_frame().is_none());
    assert_eq!(stopped.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn source_loss_is_explicit_without_retaining_a_file_or_path() {
    let mailbox = SourcePreviewMailbox::default();
    assert!(!mailbox.source_lost());
    mailbox.mark_source_lost();
    assert!(mailbox.source_lost());
    assert_eq!(
        mailbox.terminal(),
        Some(SourcePreviewMailboxTerminal::SourceLost)
    );
}

#[test]
fn startup_failure_is_explicit_without_retaining_a_file_or_path() {
    let mailbox = SourcePreviewMailbox::default();
    assert!(!mailbox.unavailable());
    mailbox.mark_unavailable();
    assert!(mailbox.unavailable());
    assert_eq!(
        mailbox.terminal(),
        Some(SourcePreviewMailboxTerminal::Unavailable)
    );
}

#[test]
fn first_native_terminal_reason_wins_over_a_later_cleanup_signal() {
    let mailbox = SourcePreviewMailbox::default();
    mailbox.mark_unavailable();
    mailbox.mark_source_lost();
    assert_eq!(
        mailbox.terminal(),
        Some(SourcePreviewMailboxTerminal::Unavailable)
    );
}

#[test]
fn explicit_permission_denial_is_not_collapsed_into_unavailable() {
    let mailbox = SourcePreviewMailbox::default();
    mailbox.mark_permission_denied();
    assert!(mailbox.permission_denied());
    assert_eq!(
        mailbox.terminal(),
        Some(SourcePreviewMailboxTerminal::PermissionDenied)
    );
}
