use super::binding::{
    binding_fingerprint, bounded_caller_id, validate_job_id, validate_submission,
};
use super::contract::{ConsumerContract, PreparedMotionRequest};
use super::delivery::validate_delivery;
use super::events::{project_event, MotionJobEvent};
use super::status::project_job;
use super::{ConsumerError, MotionFailure};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GenericConnectorSubmit {
    pub(crate) capability_id: String,
    pub(crate) descriptor_revision: u64,
    pub(crate) descriptor_fingerprint: String,
    pub(crate) request_schema_id: String,
    pub(crate) job_id: String,
    pub(crate) request: Value,
}

/// The only execution surface the generic consumer can use. Implementations
/// bind these fixed operations to a CLI or authenticated coordinator; neither
/// descriptor data nor a request can select a command, executable, path, URL,
/// provider, callback, or caller identity.
pub(crate) trait GenericMotionTransport {
    fn runtime_probe(&mut self) -> Result<Value, ConsumerError>;
    fn capability_catalog(&mut self) -> Result<Value, ConsumerError>;
    /// Fixed `connector describe <capability-id>` discovery operation.
    fn capability_describe(&mut self, capability_id: &str) -> Result<Value, ConsumerError>;
    /// This comes from the existing authenticated transport, never from data.
    fn caller_id(&mut self) -> Result<String, ConsumerError>;
    fn submit_connector(&mut self, request: GenericConnectorSubmit)
        -> Result<Value, ConsumerError>;
    fn job_get(&mut self, job_id: &str) -> Result<Value, ConsumerError>;
    fn job_list(&mut self) -> Result<Vec<Value>, ConsumerError>;
    fn job_events(&mut self, job_id: &str, after: Option<u64>)
        -> Result<Vec<Value>, ConsumerError>;
    fn job_cancel(&mut self, job_id: &str, reason: &str) -> Result<Value, ConsumerError>;
    fn job_retry(&mut self, job_id: &str) -> Result<Value, ConsumerError>;
    /// Cut-owned trusted delivery resolution, not a Motion public command.
    fn connector_delivery(&mut self, job_id: &str) -> Result<Value, ConsumerError>;
}

pub(crate) struct GenericMotionConsumer<T> {
    transport: T,
    contract: ConsumerContract,
}
#[derive(Clone, Debug)]
pub(crate) struct BoundMotionJob {
    pub(super) prepared: PreparedMotionRequest,
    pub(crate) job_id: String,
    pub(super) caller_id: String,
    pub(super) binding_fingerprint: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JobProjection {
    Pending {
        job_id: String,
        poll_after_ms: u64,
        cancel_requested: bool,
    },
    Running {
        job_id: String,
        poll_after_ms: u64,
        cancel_requested: bool,
    },
    Succeeded {
        job_id: String,
        receipt_id: Option<String>,
        receipt_available: bool,
    },
    Failed {
        job_id: String,
        error: MotionFailure,
    },
    Cancelled {
        job_id: String,
    },
    Skipped {
        job_id: String,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AcceptedMotionDelivery {
    pub(crate) artifacts: Vec<Value>,
    pub(crate) receipts: Vec<Value>,
    pub(crate) import_plan: Value,
}

impl<T: GenericMotionTransport> GenericMotionConsumer<T> {
    pub(crate) fn discover(mut transport: T) -> Result<Self, ConsumerError> {
        let probe = transport.runtime_probe()?;
        let catalog = transport.capability_catalog()?;
        Ok(Self {
            transport,
            contract: ConsumerContract::negotiate(&probe, &catalog)?,
        })
    }
    pub(crate) fn prepare(
        &self,
        capability_id: &str,
        request: Value,
    ) -> Result<PreparedMotionRequest, ConsumerError> {
        self.contract.prepare(capability_id, request)
    }
    pub(crate) fn availability(&self) -> Vec<super::contract::CapabilityAvailability> {
        self.contract.availability()
    }
    pub(crate) fn submit(
        &mut self,
        prepared: PreparedMotionRequest,
        job_id: String,
    ) -> Result<BoundMotionJob, ConsumerError> {
        validate_job_id(&job_id)?;
        let caller_id = bounded_caller_id(self.transport.caller_id()?)?;
        let described = self
            .transport
            .capability_describe(&prepared.capability_id)?;
        self.contract
            .require_described_descriptor(&prepared, described)?;
        let binding_fingerprint = binding_fingerprint(&prepared, &job_id, &caller_id);
        let response = self.transport.submit_connector(GenericConnectorSubmit {
            capability_id: prepared.capability_id.clone(),
            descriptor_revision: prepared.descriptor_revision,
            descriptor_fingerprint: prepared.descriptor_fingerprint.clone(),
            request_schema_id: prepared.request_schema_id.clone(),
            job_id: job_id.clone(),
            request: prepared.request.clone(),
        })?;
        validate_submission(&response, &prepared, &job_id, &binding_fingerprint)?;
        Ok(BoundMotionJob {
            prepared,
            job_id,
            caller_id,
            binding_fingerprint,
        })
    }
    pub(crate) fn get(&mut self, job: &BoundMotionJob) -> Result<JobProjection, ConsumerError> {
        require_control(job, "get")?;
        project_job(job, self.transport.job_get(&job.job_id)?)
    }
    pub(crate) fn list(
        &mut self,
        job: &BoundMotionJob,
    ) -> Result<Vec<JobProjection>, ConsumerError> {
        require_control(job, "list")?;
        self.transport
            .job_list()?
            .into_iter()
            .filter(|value| value.get("jobId").and_then(Value::as_str) == Some(job.job_id.as_str()))
            .map(|value| project_job(job, value))
            .collect()
    }
    pub(crate) fn events(
        &mut self,
        job: &BoundMotionJob,
        after: Option<u64>,
    ) -> Result<Vec<MotionJobEvent>, ConsumerError> {
        require_control(job, "events")?;
        let mut previous = after.unwrap_or(0);
        self.transport
            .job_events(&job.job_id, after)?
            .into_iter()
            .map(|value| {
                let event = project_event(value)?;
                if previous.checked_add(1) != Some(event.sequence) {
                    return Err(ConsumerError::refusal(
                        "Motion job events are not contiguous after the requested sequence",
                    ));
                }
                previous = event.sequence;
                Ok(event)
            })
            .collect()
    }
    pub(crate) fn cancel(
        &mut self,
        job: &BoundMotionJob,
        reason: &str,
    ) -> Result<JobProjection, ConsumerError> {
        require_control(job, "cancel")?;
        if !matches!(
            self.get(job)?,
            JobProjection::Pending { .. } | JobProjection::Running { .. }
        ) {
            return Err(ConsumerError::refusal(
                "Motion cancellation is admitted only for a pending or running job",
            ));
        }
        if reason.is_empty() || reason.len() > 512 {
            return Err(ConsumerError::refusal(
                "Motion cancellation reason is outside bounds",
            ));
        }
        project_job(job, self.transport.job_cancel(&job.job_id, reason)?)
    }
    pub(crate) fn retry(&mut self, job: &BoundMotionJob) -> Result<BoundMotionJob, ConsumerError> {
        require_control(job, "retry")?;
        if !matches!(self.get(job)?, JobProjection::Failed { error, .. } if error.retryable) {
            return Err(ConsumerError::refusal(
                "Motion retry is admitted only for a terminal retryable failure",
            ));
        }
        self.refresh_prepared_binding(&job.prepared)?;
        let caller_id = bounded_caller_id(self.transport.caller_id()?)?;
        if caller_id != job.caller_id {
            return Err(ConsumerError::refusal(
                "Motion retry caller identity drifted from the original bound job",
            ));
        }
        let retry = self
            .transport
            .job_retry(&job.job_id)?
            .as_object()
            .cloned()
            .ok_or_else(|| ConsumerError::refusal("Motion retry response must be an object"))?;
        if retry.len() != 2
            || retry
                .keys()
                .any(|key| !["jobId", "priorJobId"].contains(&key.as_str()))
        {
            return Err(ConsumerError::refusal(
                "Motion retry response has unknown or missing fields",
            ));
        }
        let next_job_id = super::binding::text(&retry, "jobId", 128)?;
        validate_job_id(&next_job_id)?;
        if super::binding::text(&retry, "priorJobId", 128)? != job.job_id
            || next_job_id == job.job_id
        {
            return Err(ConsumerError::refusal(
                "Motion retry did not mint a new job id",
            ));
        }
        Ok(BoundMotionJob {
            binding_fingerprint: binding_fingerprint(&job.prepared, &next_job_id, &caller_id),
            prepared: job.prepared.clone(),
            job_id: next_job_id,
            caller_id,
        })
    }
    pub(crate) fn accept_delivery(
        &mut self,
        job: &BoundMotionJob,
        status: &JobProjection,
    ) -> Result<AcceptedMotionDelivery, ConsumerError> {
        if !matches!(status, JobProjection::Succeeded { job_id, .. } if job_id == &job.job_id) {
            return Err(ConsumerError::refusal(
                "Motion delivery may be accepted only after a succeeded terminal job state",
            ));
        }
        validate_delivery(job, self.transport.connector_delivery(&job.job_id)?)
    }
    #[cfg(test)]
    pub(crate) fn transport(&self) -> &T {
        &self.transport
    }

    fn refresh_prepared_binding(
        &mut self,
        prepared: &PreparedMotionRequest,
    ) -> Result<(), ConsumerError> {
        let probe = self.transport.runtime_probe()?;
        let catalog = self.transport.capability_catalog()?;
        let refreshed = ConsumerContract::negotiate(&probe, &catalog)?;
        let refreshed_prepared =
            refreshed.prepare(&prepared.capability_id, prepared.request.clone())?;
        if refreshed_prepared.catalog_fingerprint != prepared.catalog_fingerprint
            || refreshed_prepared.descriptor_revision != prepared.descriptor_revision
            || refreshed_prepared.descriptor_fingerprint != prepared.descriptor_fingerprint
            || refreshed_prepared.request_schema_id != prepared.request_schema_id
        {
            return Err(ConsumerError::refusal(
                "Motion retry descriptor or catalog drifted; Cut will not upgrade a bound job",
            ));
        }
        let described = self
            .transport
            .capability_describe(&prepared.capability_id)?;
        refreshed.require_described_descriptor(prepared, described)?;
        self.contract = refreshed;
        Ok(())
    }
}
fn require_control(job: &BoundMotionJob, control: &str) -> Result<(), ConsumerError> {
    if job.prepared.controls.contains(control) {
        Ok(())
    } else {
        Err(ConsumerError::refusal(
            "Motion descriptor does not advertise this job control",
        ))
    }
}
