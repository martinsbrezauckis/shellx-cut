//! One bounded geometry observer for the selected Windows capture owner.
//! The low-level hook only retains transitions and attempts a nonblocking enqueue.
use crate::window_click_coordinates::{WindowClicks, WindowRect, MAX_EVENT_AGE_MS};
use record_core::MouseButton;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const QUEUE_CAPACITY: usize = 64;
const FINISH_WAIT: Duration = Duration::from_millis(100);
struct ClickJob {
    index: usize,
    callback_ms: u64,
    point: Option<(f64, f64, u32)>,
}
enum Work {
    Click(ClickJob),
    Drain,
}

pub(crate) struct WindowClickWorker {
    state: Arc<Mutex<WindowClicks>>,
    tx: mpsc::SyncSender<Work>,
    done: Mutex<mpsc::Receiver<()>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl WindowClickWorker {
    pub(crate) fn new(
        state: Arc<Mutex<WindowClicks>>,
        start: Instant,
        mut geometry: impl FnMut((f64, f64)) -> Option<WindowRect> + Send + 'static,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::sync_channel(QUEUE_CAPACITY);
        let (done_tx, done) = mpsc::channel();
        let observed = state.clone();
        let thread = thread::Builder::new()
            .name("cut-window-click-geometry".into())
            .spawn(move || {
                loop {
                    if observed
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .is_closed()
                    {
                        break;
                    }
                    let work = match rx.recv_timeout(Duration::from_millis(25)) {
                        Ok(work) => work,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let Work::Click(job) = work else {
                        break;
                    };
                    let age_at = |age: u32| {
                        let now = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                        age.saturating_add(
                            u32::try_from(now.saturating_sub(job.callback_ms)).unwrap_or(u32::MAX),
                        )
                    };
                    let rect = job.point.and_then(|(x, y, age)| {
                        (age_at(age) <= MAX_EVENT_AGE_MS && x.is_finite() && y.is_finite())
                            .then(|| geometry((x, y)))
                            .flatten()
                    });
                    // Queue and native query time both count. Never admit an old point
                    // merely because it was fresh when the hook enqueued it.
                    let point = job.point.map(|(x, y, age)| (x, y, age_at(age)));
                    observed
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .resolve_click(job.index, point, rect);
                }
                let _ = done_tx.send(());
            })?;
        Ok(Self {
            state,
            tx,
            done: Mutex::new(done),
            thread: Mutex::new(Some(thread)),
        })
    }

    pub(crate) fn enqueue(
        &self,
        callback_ms: u64,
        point: Option<(f64, f64, u32)>,
        button: MouseButton,
        down: bool,
    ) {
        if point.is_some_and(|(_, _, age)| u64::from(age) > callback_ms) {
            return;
        }
        let t_ms = callback_ms.saturating_sub(point.map_or(0, |(_, _, age)| u64::from(age)));
        let index = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .queue_click(t_ms, point, button, down);
        let job = Work::Click(ClickJob {
            index,
            callback_ms,
            point,
        });
        if let Err(error) = self.tx.try_send(job) {
            let reason = match error {
                mpsc::TrySendError::Full(_) => "geometry queue full",
                mpsc::TrySendError::Disconnected(_) => "geometry observer ended",
            };
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .refuse(index, reason);
        }
    }

    /// Drain already-retained transitions for at most 100ms, then revoke all
    /// admission before the caller snapshots. A stalled query cannot mutate later.
    pub(crate) fn seal(&self) {
        let sent = self.tx.try_send(Work::Drain).is_ok();
        let exited = sent
            && self
                .done
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .recv_timeout(FINISH_WAIT)
                .is_ok();
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .close();
        if exited {
            self.join_observed();
        }
    }

    fn join_observed(&self) {
        if let Some(thread) = self
            .thread
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = thread.join();
        }
    }
}

impl Drop for WindowClickWorker {
    fn drop(&mut self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .close();
        if self
            .done
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .try_recv()
            .is_ok()
        {
            self.join_observed();
            return;
        }
        let Some(thread) = self
            .thread
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        else {
            return;
        };
        // Do not detach an unobserved native query or block a recorder caller.
        // The reaper owns the join until query return and worker exit.
        let retained = Arc::new(Mutex::new(Some(thread)));
        let reaper = retained.clone();
        if let Err(error) = thread::Builder::new()
            .name("cut-window-click-reaper".into())
            .spawn(move || {
                if let Some(thread) = reaper
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                {
                    let _ = thread.join();
                }
            })
        {
            eprintln!("window click reaper could not start; ownership retained: {error}");
            Box::leak(Box::new(retained));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> Arc<Mutex<WindowClicks>> {
        let mut state = WindowClicks::new(800, 600);
        state.frame(0, Some((800, 600)));
        Arc::new(Mutex::new(state))
    }
    fn rect() -> WindowRect {
        WindowRect {
            left: 100,
            top: 200,
            width: 800,
            height: 600,
        }
    }
    fn finish(
        state: &Arc<Mutex<WindowClicks>>,
    ) -> (
        Vec<record_core::ClickSample>,
        record_core::CursorCorrelation,
    ) {
        std::mem::replace(&mut *state.lock().unwrap(), WindowClicks::new(800, 600)).finish(1000)
    }
    #[test]
    fn worker_drains_actual_transition_before_seal() {
        let state = state();
        let worker =
            WindowClickWorker::new(state.clone(), Instant::now(), |_| Some(rect())).unwrap();
        worker.enqueue(0, Some((500.0, 500.0, 0)), MouseButton::Left, true);
        worker.seal();
        let (clicks, correlation) = finish(&state);
        assert_eq!(clicks.len(), 1);
        assert_eq!((clicks[0].x, clicks[0].y), (400.0, 300.0));
        assert_eq!(correlation.exact_clicks, 1);
    }
    #[test]
    fn slow_query_seals_bounded_and_never_mutates_late() {
        let state = state();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (exit_tx, exit_rx) = mpsc::channel();
        let worker = WindowClickWorker::new(state.clone(), Instant::now(), move |_| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            exit_tx.send(()).unwrap();
            Some(rect())
        })
        .unwrap();
        worker.enqueue(0, Some((500.0, 500.0, 0)), MouseButton::Left, true);
        entered_rx.recv_timeout(Duration::from_millis(250)).unwrap();
        let started = Instant::now();
        worker.seal();
        assert!(started.elapsed() < Duration::from_millis(250));
        release_tx.send(()).unwrap();
        exit_rx.recv_timeout(Duration::from_millis(250)).unwrap();
        worker
            .done
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_millis(250))
            .unwrap();
        worker.join_observed();
        drop(worker);
        let (clicks, correlation) = finish(&state);
        assert_eq!(clicks.len(), 1);
        assert_eq!(correlation.unavailable_clicks, 1);
    }
    #[test]
    fn full_queue_keeps_every_transition_visible_unavailable() {
        let state = state();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = WindowClickWorker::new(state.clone(), Instant::now(), move |_| {
            let _ = entered_tx.send(());
            let _ = release_rx.recv();
            Some(rect())
        })
        .unwrap();
        worker.enqueue(0, Some((500.0, 500.0, 0)), MouseButton::Left, true);
        entered_rx.recv_timeout(Duration::from_millis(250)).unwrap();
        for _ in 0..QUEUE_CAPACITY + 1 {
            worker.enqueue(0, Some((500.0, 500.0, 0)), MouseButton::Left, true);
        }
        worker.seal();
        drop(worker);
        let _ = release_tx.send(());
        let (clicks, correlation) = finish(&state);
        assert_eq!(clicks.len(), QUEUE_CAPACITY + 2);
        assert_eq!(correlation.exact_clicks, 0);
        assert!(correlation.detail.unwrap().contains("geometry queue full"));
    }
    #[test]
    fn stale_job_refuses_without_native_query_and_closed_owner_stays_unavailable() {
        let state = state();
        let worker = WindowClickWorker::new(state.clone(), Instant::now(), |_| {
            panic!("stale job must not query native geometry")
        })
        .unwrap();
        worker.enqueue(200, Some((500.0, 500.0, 101)), MouseButton::Left, true);
        worker.seal();
        let (clicks, correlation) = finish(&state);
        assert_eq!(clicks.len(), 1);
        assert_eq!(correlation.unavailable_clicks, 1);
    }
    #[test]
    fn native_query_time_counts_toward_freshness() {
        let state = state();
        let start = Instant::now() - Duration::from_millis(100);
        let worker = WindowClickWorker::new(state.clone(), start, |_| {
            thread::sleep(Duration::from_millis(20));
            Some(rect())
        })
        .unwrap();
        worker.enqueue(100, Some((500.0, 500.0, 90)), MouseButton::Left, true);
        worker.tx.try_send(Work::Drain).unwrap();
        worker
            .done
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        worker.join_observed();
        worker.seal();
        let (_, correlation) = finish(&state);
        assert_eq!(correlation.exact_clicks, 0);
        assert_eq!(correlation.unavailable_clicks, 1);
        assert!(correlation
            .detail
            .unwrap()
            .contains("queue/query age exceeded"));
    }

    #[test]
    fn closed_source_never_runs_new_native_geometry_query() {
        let state = state();
        let worker = WindowClickWorker::new(state.clone(), Instant::now(), |_| {
            panic!("closed source cannot run geometry query")
        })
        .unwrap();
        state.lock().unwrap().close();
        worker.enqueue(0, Some((500.0, 500.0, 0)), MouseButton::Left, true);
        worker.seal();
        let (clicks, correlation) = finish(&state);
        assert_eq!(clicks.len(), 1);
        assert_eq!(correlation.unavailable_clicks, 1);
    }
    #[test]
    fn drop_retains_worker_until_stalled_query_returns_then_retires() {
        struct NativeQueryOwner(mpsc::Sender<()>);
        impl Drop for NativeQueryOwner {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let state = state();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (retired_tx, retired_rx) = mpsc::channel();
        let owned = NativeQueryOwner(retired_tx);
        let worker = WindowClickWorker::new(state.clone(), Instant::now(), move |_| {
            let _owner = &owned;
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Some(rect())
        })
        .unwrap();
        worker.enqueue(0, Some((500.0, 500.0, 0)), MouseButton::Left, true);
        entered_rx.recv_timeout(Duration::from_millis(250)).unwrap();
        let started = Instant::now();
        drop(worker);
        assert!(started.elapsed() < Duration::from_millis(100));
        assert!(retired_rx.try_recv().is_err());
        release_tx.send(()).unwrap();
        retired_rx.recv_timeout(Duration::from_millis(250)).unwrap();
        let (_, correlation) = finish(&state);
        assert_eq!(correlation.exact_clicks, 0);
        assert_eq!(correlation.unavailable_clicks, 1);
    }
}
