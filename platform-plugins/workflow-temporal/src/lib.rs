//! Temporal durable workflow adapter.
//!
//! Local-only, unqualified adapter for the masonwing-dev compose stack. This
//! implementation is a compatibility shim for the C1 durable-execution
//! slice; it is NOT a qualified production adapter and must never be used in
//! production or staging.
//!
//! Because domain plugins never import private platform implementations, the
//! adapter lives only in `platform-plugins/workflow-temporal` and is wired by
//! the `masonwing-worker` composition root.

use masonwing_contracts::AdapterQualification;
use masonwing_kernel::ports::{AdapterBoundary, DurableWorkflowPort, PortError, WorkflowDispatch};

#[derive(Debug, Default)]
pub struct TemporalWorkflowAdapter;

impl AdapterBoundary for TemporalWorkflowAdapter {
    fn adapter_name(&self) -> &'static str {
        "workflow-temporal"
    }

    fn qualification(&self) -> AdapterQualification {
        AdapterQualification::Unqualified
    }
}

impl DurableWorkflowPort for TemporalWorkflowAdapter {
    fn dispatch(&self, _request: &WorkflowDispatch) -> Result<(), PortError> {
        Err(PortError::NotQualified {
            adapter: self.adapter_name(),
        })
    }
}

#[cfg(feature = "local-temporal-tests")]
mod temporal_client {
    use std::time::Duration;

    use masonwing_contracts::AdapterQualification;
    use masonwing_kernel::ports::{AdapterBoundary, PortError};
    use temporalio_client::{
        Client, ClientOptions, Connection, ConnectionOptions, UntypedWorkflow, Url,
        WorkflowStartOptions, errors::WorkflowStartError,
    };
    use temporalio_common::data_converters::RawValue;
    use temporalio_common::protos::temporal::api::common::v1::Payload;

    /// Local-only Temporal workflow client. Qualification stays UNQUALIFIED;
    /// production must replace this with a reviewed implementation.
    pub struct LocalTemporalClient {
        client: Client,
        task_queue: String,
    }

    impl AdapterBoundary for LocalTemporalClient {
        fn adapter_name(&self) -> &'static str {
            "workflow-temporal-local"
        }

        fn qualification(&self) -> AdapterQualification {
            AdapterQualification::Unqualified
        }
    }

    impl LocalTemporalClient {
        /// Connect to a Temporal server at `address` (e.g. `http://temporal:7233`
        /// or `http://127.0.0.1:39854`) on `namespace`.
        pub async fn connect(
            address: &str,
            namespace: &str,
            task_queue: &str,
        ) -> Result<Self, PortError> {
            let url = Url::parse(address).map_err(|_| PortError::Unavailable {
                adapter: "workflow-temporal-local",
            })?;
            let connection = Connection::connect(
                ConnectionOptions::new(url)
                    .connect_timeout(Duration::from_secs(10))
                    .build(),
            )
            .await
            .map_err(|_| PortError::Unavailable {
                adapter: "workflow-temporal-local",
            })?;
            let client =
                Client::new(connection, ClientOptions::new(namespace).build()).map_err(|_| {
                    PortError::Unavailable {
                        adapter: "workflow-temporal-local",
                    }
                })?;
            Ok(Self {
                client,
                task_queue: task_queue.to_owned(),
            })
        }

        /// Start the workflow execution for a masonwing run. The workflow type
        /// name is the stable contract name `masonwing.run`; the payload is the
        /// canonical JSON of `{tenant_id, run_id}`.
        ///
        /// On `AlreadyStarted` the prior run_id is adopted (idempotent replay
        /// after a worker crash mid-start).
        pub async fn start_run_workflow(
            &self,
            workflow_id: &str,
            tenant_id: &str,
            run_id: &str,
        ) -> Result<String, PortError> {
            let input = serde_json::json!({
                "tenant_id": tenant_id,
                "run_id": run_id,
            });
            let bytes = serde_json::to_vec(&input).map_err(|_| PortError::Unavailable {
                adapter: "workflow-temporal-local",
            })?;
            let payload = Payload {
                data: bytes,
                ..Default::default()
            };
            let raw = RawValue::new(vec![payload]);
            let workflow = UntypedWorkflow::new("masonwing.run");
            match self
                .client
                .start_workflow(
                    workflow,
                    raw,
                    WorkflowStartOptions::new(workflow_id, self.task_queue.clone())
                        .execution_timeout(Duration::from_secs(3600))
                        .task_timeout(Duration::from_secs(30))
                        .build(),
                )
                .await
            {
                Ok(handle) => Ok(handle.run_id().unwrap_or_default().to_owned()),
                Err(WorkflowStartError::AlreadyStarted { run_id, .. }) => {
                    Ok(run_id.unwrap_or_default())
                }
                Err(_) => Err(PortError::Unavailable {
                    adapter: "workflow-temporal-local",
                }),
            }
        }
    }
}

#[cfg(feature = "local-temporal-tests")]
pub use temporal_client::LocalTemporalClient;
