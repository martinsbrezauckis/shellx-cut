//! Wait operations with a test-only error seam.

use std::io;
use std::process::ExitStatus;
use std::time::Duration;
use tokio::process::Child;

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(test)]
use std::sync::Arc;

#[derive(Clone, Default)]
pub(super) struct WaitErrors {
    #[cfg(test)]
    remaining: Arc<AtomicUsize>,
}

impl WaitErrors {
    #[cfg(test)]
    pub(super) fn with_count(count: usize) -> Self {
        Self {
            remaining: Arc::new(AtomicUsize::new(count)),
        }
    }

    #[cfg(test)]
    pub(super) fn remaining(&self) -> usize {
        self.remaining.load(Ordering::SeqCst)
    }

    fn check(&self) -> io::Result<()> {
        #[cfg(test)]
        if self
            .remaining
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                (count > 0).then(|| count - 1)
            })
            .is_ok()
        {
            return Err(io::Error::other("injected child wait error"));
        }
        Ok(())
    }
}

pub(super) async fn wait_once(child: &mut Child, errors: &WaitErrors) -> io::Result<ExitStatus> {
    errors.check()?;
    child.wait().await
}

fn try_wait_once(child: &mut Child, errors: &WaitErrors) -> io::Result<Option<ExitStatus>> {
    errors.check()?;
    child.try_wait()
}

pub(super) async fn confirmed_reap(child: &mut Child, errors: &WaitErrors) {
    let mut pause = Duration::from_millis(25);
    loop {
        let _ = child.start_kill();
        match try_wait_once(child, errors) {
            Ok(Some(_)) => return,
            Ok(None) if wait_once(child, errors).await.is_ok() => return,
            _ => {}
        }
        // 25, 50, 100, 200, 400, 800, then 1000 ms between retries.
        tokio::time::sleep(pause).await;
        pause = (pause * 2).min(Duration::from_secs(1));
    }
}
