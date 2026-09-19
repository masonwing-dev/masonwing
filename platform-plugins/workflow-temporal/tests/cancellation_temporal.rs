//! Local Temporal SDK compatibility probe, not product acceptance.
//! No PostgreSQL dispatcher or provider transport is exercised here.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use temporalio_client::{
    Client, ClientOptions, Connection, ConnectionOptions, Url, WorkflowCancelOptions,
    WorkflowGetResultOptions, WorkflowStartOptions,
};
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::{
    ActivityOptions, Runtime, Worker, WorkerOptions, WorkflowCancellationToken, WorkflowContext,
    WorkflowResult,
    activities::{ActivityContext, ActivityError},
};
use temporalio_workflow::RetryPolicy;

#[derive(Clone, Default)]
struct Probe {
    trace: Arc<Mutex<Vec<&'static str>>>,
}

#[activities]
impl Probe {
    #[activity]
    async fn sent_then_wait(
        self: Arc<Self>,
        ctx: ActivityContext,
        _: (),
    ) -> Result<(), ActivityError> {
        // A synthetic observation, not an external mutation or remote receipt.
        self.trace.lock().unwrap().push("sent");
        loop {
            if ctx.is_cancelled() {
                return Err(ActivityError::cancelled());
            }
            ctx.record_heartbeat(()).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[activity]
    async fn reconcile(
        self: Arc<Self>,
        _: ActivityContext,
        _: (),
    ) -> Result<String, ActivityError> {
        self.trace.lock().unwrap().push("reconciled");
        Ok("observed; not undone".into())
    }
}

#[workflow]
#[derive(Default)]
struct CancellationProbe;

#[workflow_methods]
impl CancellationProbe {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, _: ()) -> WorkflowResult<String> {
        let result = ctx
            .execute_activity(
                Probe::sent_then_wait,
                (),
                ActivityOptions::with_start_to_close_timeout(Duration::from_secs(60))
                    // Heartbeats are throttled by the SDK and share a loaded host; 3s
                    // is too tight under contention and was observed timing out.
                    .heartbeat_timeout(Duration::from_secs(30))
                    .retry_policy(RetryPolicy::builder().maximum_attempts(1).build())
                    .build(),
            )
            .await;
        match result {
            // Reconcile on either a bare cancellation or a Failed(...) wrapper
            // containing cancellation — both are reported by the SDK.
            Err(ref error) if error.as_cancelled().is_some() => Ok(ctx
                .execute_activity(
                    Probe::reconcile,
                    (),
                    ActivityOptions::with_start_to_close_timeout(Duration::from_secs(10))
                        .cancellation_token(WorkflowCancellationToken::new())
                        .retry_policy(RetryPolicy::builder().maximum_attempts(1).build())
                        .build(),
                )
                .await?),
            Err(error) => Err(error.into()),
            Ok(()) => Ok("unexpected completion".into()),
        }
    }
}

// MASONWING@1.0.1 REQ-074: SDK cancellation compatibility only.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_allows_detached_reconciliation_without_repeating_sent_activity()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let runtime = Runtime::from_current_tokio(Default::default())?;
    // Only the existing masonwing-dev loopback service; never download/start a server.
    let connection = Connection::connect(
        ConnectionOptions::new(Url::parse("http://127.0.0.1:39854")?)
            .connect_timeout(Duration::from_secs(5))
            .build(),
    )
    .await?;
    let client = Client::new(connection, ClientOptions::new("masonwing-local").build())?;
    let id = format!("cancel-probe-{}", uuid::Uuid::new_v4());
    let probe = Probe::default();
    let options = WorkerOptions::new(id.clone())
        .register_workflow::<CancellationProbe>()?
        .register_activities(probe.clone())
        .build();
    let mut worker = Worker::new(&runtime, client.clone(), options)?;
    let shutdown = worker.shutdown_handle();
    let scenario = async {
        let result = tokio::time::timeout(Duration::from_secs(150), async {
            let handle = client
                .start_workflow(
                    CancellationProbe::run,
                    (),
                    WorkflowStartOptions::new(id.clone(), id.clone())
                        .execution_timeout(Duration::from_secs(120))
                        // Local machine contention can starve workflow tasks beyond the
                        // 10s default; this probe workflow is trivial so 30s stays bounded.
                        .task_timeout(Duration::from_secs(30))
                        .build(),
                )
                .await?;
            // Wait for the actual activity, not an arbitrary delay before cancellation.
            loop {
                if probe.trace.lock().unwrap().contains(&"sent") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            handle.cancel(WorkflowCancelOptions::default()).await?;
            let output = handle
                .get_result(WorkflowGetResultOptions::default())
                .await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(output)
        })
        .await;
        shutdown();
        result
    };
    let (worker_result, scenario_result) = tokio::join!(worker.run(), scenario);
    worker_result?;
    let output = scenario_result??;
    assert_eq!(output, "observed; not undone");
    assert_eq!(*probe.trace.lock().unwrap(), vec!["sent", "reconciled"]);
    Ok(())
}
