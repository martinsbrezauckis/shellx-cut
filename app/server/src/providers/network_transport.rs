//! Finite HTTP transport policy for external asset providers.
//!
//! This stays separate from provider catalog parsing so the availability rule is
//! reviewable: `assets.fetch` pins project ownership during resolve/download,
//! therefore every network stage must have an end-to-end deadline.

use cut_core::{error_codes, CutError};
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

/// ureq defaults all timeouts to `None`. Sixty seconds permits normal media
/// downloads while ensuring a dead source cannot block a project transition
/// forever. ureq applies the global timeout from its DNS through body read;
/// the pinned-download preflight below has its own cap.
const PROVIDER_HTTP_TIMEOUT: Duration = Duration::from_secs(60);
/// DNS preflight is outside ureq's pinned connection. Keep that worker bounded
/// too; a timed-out worker holds only a URL/host and can never write a file.
const PROVIDER_DNS_TIMEOUT: Duration = Duration::from_secs(10);

/// A timed-out blocking resolver cannot be cancelled. One permit remains held
/// until its worker actually exits, so repeated hostile/stuck names cannot fill
/// Tokio's blocking pool while a project transition is waiting on DNS.
static DNS_RESOLUTION_PERMIT: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();

fn provider_http_config() -> ureq::config::Config {
    ureq::Agent::config_builder()
        .timeout_global(Some(PROVIDER_HTTP_TIMEOUT))
        .build()
}

pub(super) fn provider_http_agent() -> ureq::Agent {
    provider_http_config().new_agent()
}

fn provider_download_config() -> ureq::config::Config {
    ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .timeout_global(Some(PROVIDER_HTTP_TIMEOUT))
        .build()
}

/// Build the SSRF-fenced download agent while retaining the shared deadline.
pub(super) fn pinned_download_agent(target: &super::VettedDownloadTarget) -> ureq::Agent {
    ureq::Agent::with_parts(
        provider_download_config(),
        ureq::unversioned::transport::DefaultConnector::default(),
        super::PinnedResolver::new(target.host.clone(), target.port, target.addrs.clone()),
    )
}

/// Resolve and SSRF-vet the download target before the pinned ureq stage.
///
/// `to_socket_addrs` is blocking and is not governed by ureq's timeout. The
/// worker receives no destination path, so if the deadline elapses it may
/// finish detached but cannot create or modify an asset file.
pub(super) async fn resolve_pinned_download_target(
    url: String,
) -> Result<super::VettedDownloadTarget, CutError> {
    resolve_with_deadline(PROVIDER_DNS_TIMEOUT, dns_resolution_permit(), move || {
        super::vetted_download_target(&url)
    })
    .await
}

fn dns_resolution_permit() -> Arc<tokio::sync::Semaphore> {
    DNS_RESOLUTION_PERMIT
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(1)))
        .clone()
}

async fn resolve_with_deadline<T, F>(
    deadline: Duration,
    permit_pool: Arc<tokio::sync::Semaphore>,
    resolve: F,
) -> Result<T, CutError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CutError> + Send + 'static,
{
    let permit = tokio::time::timeout(deadline, permit_pool.acquire_owned())
        .await
        .map_err(|_| dns_timeout_error(deadline, "provider DNS resolver remained busy"))?
        .map_err(|_| {
            CutError::new(
                error_codes::IO,
                "provider DNS resolver is unavailable",
                "the resolver admission semaphore closed",
            )
        })?;
    let worker = tokio::task::spawn_blocking(resolve);
    let (sender, receiver) = tokio::sync::oneshot::channel();
    // The reaper owns BOTH cancellation-sensitive resources before this
    // function starts awaiting. A dropped caller only drops its receiver; the
    // non-cancellable DNS worker still keeps the permit until it actually exits.
    tokio::spawn(async move {
        let _permit = permit;
        let joined = worker.await.map_err(|error| {
            CutError::new(
                error_codes::IO,
                "provider DNS lookup task panicked",
                error.to_string(),
            )
        });
        let _ = sender.send(joined.and_then(|result| result));
    });
    match tokio::time::timeout(deadline, receiver).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(CutError::new(
            error_codes::IO,
            "provider DNS lookup reaper stopped",
            "the resolver worker result channel closed before publishing a result",
        )),
        Err(_) => Err(dns_timeout_error(
            deadline,
            "public-host preflight exceeded its deadline",
        )),
    }
}

fn dns_timeout_error(deadline: Duration, cause: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::IO,
        "provider DNS lookup timed out",
        format!("{} seconds: {}", deadline.as_secs_f64(), cause.into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    };

    struct ResolverWorkerRelease(Option<Sender<()>>);

    impl ResolverWorkerRelease {
        fn release(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    impl Drop for ResolverWorkerRelease {
        fn drop(&mut self) {
            self.release();
        }
    }

    #[test]
    fn provider_network_transports_have_a_finite_end_to_end_timeout() {
        assert_eq!(
            provider_http_config().timeouts().global,
            Some(PROVIDER_HTTP_TIMEOUT),
            "provider JSON resolve calls must not hold project ownership indefinitely"
        );
        assert_eq!(
            provider_download_config().timeouts().global,
            Some(PROVIDER_HTTP_TIMEOUT),
            "pinned provider downloads must not hold project ownership indefinitely"
        );
        assert!(
            !PROVIDER_DNS_TIMEOUT.is_zero(),
            "pinned-download DNS preflight must not wait indefinitely"
        );
        assert!(
            PROVIDER_DNS_TIMEOUT <= PROVIDER_HTTP_TIMEOUT,
            "DNS preflight must fit within the provider fetch budget"
        );
    }

    #[tokio::test]
    async fn timed_dns_resolution_caps_stuck_workers_and_recovers_the_permit() {
        let permits = Arc::new(tokio::sync::Semaphore::new(1));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let mut release = ResolverWorkerRelease(Some(release_tx));
        let first_permits = permits.clone();
        let first = tokio::spawn(resolve_with_deadline(
            Duration::from_millis(50),
            first_permits,
            move || {
                let _ = started_tx.send(());
                let _ = release_rx.recv();
                Ok::<(), CutError>(())
            },
        ));
        tokio::time::timeout(Duration::from_secs(1), started_rx)
            .await
            .expect("first resolver worker did not start")
            .expect("first resolver start signal dropped");

        let first_result = tokio::time::timeout(Duration::from_secs(1), first)
            .await
            .expect("first resolver call did not time out")
            .expect("first resolver task panicked");
        assert_eq!(
            first_result.unwrap_err().code,
            error_codes::IO,
            "a blocked DNS worker must fail the caller promptly"
        );

        let second_started = Arc::new(AtomicBool::new(false));
        let second_started_in_worker = second_started.clone();
        let second_result =
            resolve_with_deadline(Duration::from_millis(50), permits.clone(), move || {
                second_started_in_worker.store(true, Ordering::Release);
                Ok::<(), CutError>(())
            })
            .await;
        assert_eq!(
            second_result.unwrap_err().code,
            error_codes::IO,
            "a second resolver must fail while the timed-out worker owns the permit"
        );
        assert!(
            !second_started.load(Ordering::Acquire),
            "the second resolver worker must not be admitted while the first is stuck"
        );

        release.release();
        let recovered = tokio::time::timeout(Duration::from_secs(1), permits.acquire_owned())
            .await
            .expect("resolver permit did not recover after the worker exited")
            .expect("resolver permit pool unexpectedly closed");
        drop(recovered);
    }

    #[tokio::test]
    async fn cancelled_dns_caller_keeps_the_worker_capped_until_the_reaper_finishes() {
        let permits = Arc::new(tokio::sync::Semaphore::new(1));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let mut release = ResolverWorkerRelease(Some(release_tx));
        let first = tokio::spawn(resolve_with_deadline(
            Duration::from_secs(1),
            permits.clone(),
            move || {
                let _ = started_tx.send(());
                let _ = release_rx.recv();
                Ok::<(), CutError>(())
            },
        ));
        tokio::time::timeout(Duration::from_secs(1), started_rx)
            .await
            .expect("first resolver worker did not start")
            .expect("first resolver start signal dropped");

        first.abort();
        assert!(
            first
                .await
                .expect_err("aborted DNS caller unexpectedly completed")
                .is_cancelled(),
            "the outer resolver caller must be cancelled while its internal reaper remains live"
        );

        let second_started = Arc::new(AtomicBool::new(false));
        let second_started_in_worker = second_started.clone();
        let second_result =
            resolve_with_deadline(Duration::from_millis(50), permits.clone(), move || {
                second_started_in_worker.store(true, Ordering::Release);
                Ok::<(), CutError>(())
            })
            .await;
        assert_eq!(
            second_result.unwrap_err().code,
            error_codes::IO,
            "a cancelled caller must not free its still-running DNS worker permit"
        );
        assert!(
            !second_started.load(Ordering::Acquire),
            "the second resolver worker must remain unadmitted after caller cancellation"
        );

        release.release();
        let recovered = tokio::time::timeout(Duration::from_secs(1), permits.acquire_owned())
            .await
            .expect("resolver permit did not recover after the cancelled worker exited")
            .expect("resolver permit pool unexpectedly closed");
        drop(recovered);
    }
}
