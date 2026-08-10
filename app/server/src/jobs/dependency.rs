//! Durable facts for one active job waiting on another active job.

use super::JobManager;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobDependencyInfo {
    pub job_id: String,
    pub kind: String,
}

impl JobManager {
    /// Name only the child currently awaited by an orchestrator. This is an
    /// observable relationship, not a promise that the child can be retried.
    pub(crate) fn set_waiting_on(&self, job_id: &str, waiting_on: Option<JobDependencyInfo>) {
        self.update(job_id, |record| record.waiting_on = waiting_on);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;

    #[test]
    fn active_dependency_is_persisted_and_can_clear() {
        let dir = tempfile::tempdir().unwrap();
        let manager = JobManager::new(EventBus::new());
        manager.attach_project(dir.path()).unwrap();
        let parent = manager.create("render_queue");
        let child = manager.create("render");

        manager.set_waiting_on(
            &parent.job_id,
            Some(JobDependencyInfo {
                job_id: child.job_id.clone(),
                kind: child.kind.clone(),
            }),
        );
        assert_eq!(
            manager.get(&parent.job_id).unwrap().waiting_on,
            Some(JobDependencyInfo {
                job_id: child.job_id,
                kind: child.kind,
            })
        );
        let persisted: crate::jobs::JobRecord = serde_json::from_slice(
            &std::fs::read(
                dir.path()
                    .join("jobs")
                    .join(format!("{}.json", parent.job_id)),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            persisted.waiting_on,
            manager.get(&parent.job_id).unwrap().waiting_on
        );

        manager.set_waiting_on(&parent.job_id, None);
        assert_eq!(manager.get(&parent.job_id).unwrap().waiting_on, None);

        manager.set_waiting_on(
            &parent.job_id,
            Some(JobDependencyInfo {
                job_id: "job_999".into(),
                kind: "render".into(),
            }),
        );
        manager.finish(&parent.job_id, serde_json::json!({"ok": true}));
        assert_eq!(manager.get(&parent.job_id).unwrap().waiting_on, None);
    }
}
