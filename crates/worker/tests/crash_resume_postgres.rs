//! MASONWING@1.0.1 REQ-070 / AC-074: crash-resume over real PostgreSQL.
//!
//! A worker dies after completing step one of a three-node workflow. The
//! resumed worker reads `run_checkpoints`, rebuilds the deterministic
//! interpreter, and finishes the traversal: step one is never re-dispatched,
//! each remaining step runs exactly once under its own logical id, the run
//! completes, and a stale worker cannot re-drive the finished run.
//!
//! Run: cargo test -p masonwing-worker --features local-postgres-tests --test crash_resume_postgres

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use masonwing_contract_validation::digest_bytes;
use masonwing_contracts::{
    ArtifactId, Digest, TenantId,
    wire::{ArtifactRef, Classification, WorkflowDefinition, WorkflowEdge, WorkflowNode},
};
use masonwing_data_postgres::{CheckpointWrite, PostgresStore};
use masonwing_kernel::runtime::{ArtifactObjects, CommandFailure};
use masonwing_worker::run_interpreter::StepDecision;
use masonwing_worker::run_resume::interpreter_from_checkpoints;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Artifact store that never touches bytes; the checkpoint path only writes
/// references, and refusing all access keeps that invariant under fault.
struct NoObjects;

#[async_trait]
impl ArtifactObjects for NoObjects {
    async fn put_immutable(
        &self,
        _key: &str,
        _bytes: Vec<u8>,
        _content_type: &str,
    ) -> Result<(), CommandFailure> {
        Err(CommandFailure::unavailable("ARTIFACT_STORE_UNAVAILABLE"))
    }
    async fn get_bounded(&self, _key: &str, _max_bytes: usize) -> Result<Vec<u8>, CommandFailure> {
        Err(CommandFailure::unavailable("ARTIFACT_STORE_UNAVAILABLE"))
    }
}

/// A three-node linear workflow: a → b → c on SUCCESS edges.
fn definition(tenant_id: &TenantId) -> WorkflowDefinition {
    let reference = || ArtifactRef {
        tenant_id: tenant_id.clone(),
        artifact_id: ArtifactId::new(Uuid::new_v4().to_string()).unwrap(),
        digest: Digest::sha256("0".repeat(64)).unwrap(),
        schema_version: "1".to_string(),
        classification: Classification::Internal,
    };
    let node = |id: &str| WorkflowNode {
        id: id.to_string(),
        kind: "ACTIVITY".to_string(),
        handler_id: Some(format!("handler-{id}")),
        input_schema_ref: reference(),
        output_schema_ref: reference(),
        max_attempts: 1,
        timeout_seconds: 30,
        max_iterations: 1,
        on_failure: "FAIL_RUN".to_string(),
    };
    WorkflowDefinition {
        id: "test.crash-resume".to_string(),
        version: "1.0.0".to_string(),
        input_schema_ref: reference(),
        output_schema_ref: reference(),
        entry_node: "a".to_string(),
        nodes: vec![node("a"), node("b"), node("c")],
        edges: vec![
            WorkflowEdge {
                from: "a".to_string(),
                to: "b".to_string(),
                condition: "SUCCESS".to_string(),
            },
            WorkflowEdge {
                from: "b".to_string(),
                to: "c".to_string(),
                condition: "SUCCESS".to_string(),
            },
        ],
        max_total_steps: 8,
        max_model_turns: 4,
        required_actions: vec![],
        digest: Digest::sha256("0".repeat(64)).unwrap(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crash_resume_dispatches_each_remaining_step_exactly_once() -> TestResult {
    let operator = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .connect(
            "postgresql://masonwing_migrator:masonwing-local-migrator@127.0.0.1:39852/masonwing",
        )
        .await?;
    let runtime = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(5))
        .connect("postgresql://masonwing_app:masonwing-local-app@127.0.0.1:39852/masonwing")
        .await?;
    let store = PostgresStore::from_pool(runtime, Arc::new(NoObjects)).await?;

    let tenant = Uuid::new_v4();
    let principal = Uuid::new_v4();
    let membership = Uuid::new_v4();
    {
        let mut tx = operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(tenant.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO tenants(id,name) VALUES($1,'crash-resume fixture')")
            .bind(tenant)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO memberships(tenant_id,principal_id,membership_id,role,status,membership_epoch,permission_epoch) VALUES($1,$2,$3,'OWNER','ACTIVE',1,1)")
            .bind(tenant)
            .bind(principal)
            .bind(membership)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO membership_roles(tenant_id,membership_id,role) VALUES($1,$2,'OWNER')",
        )
        .bind(tenant)
        .bind(membership)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO authorization_policies(tenant_id,policy_version,policy_epoch,cedar_source,is_current) VALUES($1,'1.0.0',1,'permit(principal, action, resource);',true)")
            .bind(tenant)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }

    let tenant_id = TenantId::new(tenant.to_string())?;
    let definition = definition(&tenant_id);
    let run_id = Uuid::new_v4();
    let grant_id = Uuid::new_v4();
    let input_ref = ArtifactRef {
        tenant_id: tenant_id.clone(),
        artifact_id: ArtifactId::new(Uuid::new_v4().to_string())?,
        digest: digest_bytes(b"{}"),
        schema_version: "1".to_string(),
        classification: Classification::Internal,
    };
    {
        let mut tx = operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(tenant.to_string())
            .execute(&mut *tx)
            .await?;
        // The run row is seeded RUNNING: admission proved itself elsewhere; this
        // test exercises only the crash/resume half of the lifecycle.
        sqlx::query("INSERT INTO delegations(tenant_id,id,principal_id,principal_type,issuer,creator_id,actions,resources,expires_at,state) VALUES($1,$2,$3,'USER','https://crash-resume.test',$4,ARRAY['run.start'],'[]'::jsonb,clock_timestamp()+interval '1 hour','ACTIVE')")
            .bind(tenant)
            .bind(grant_id)
            .bind(principal.to_string())
            .bind(principal)
            .execute(&mut *tx)
            .await?;
        // The runs table pins (plugin_id, artifact_digest) against
        // plugin_versions, so the digest must be a real sha256 and the
        // signature chain must exist to satisfy the foreign key.
        let plugin_digest = format!("sha256:{}", "a".repeat(64));
        sqlx::query("INSERT INTO publisher_keys(tenant_id,publisher_id,key_id,public_key) VALUES($1,'test.publisher','test.key',decode('0000000000000000000000000000000000000000000000000000000000000000','hex'))")
            .bind(tenant)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO artifact_signatures(tenant_id,id,publisher_id,key_id,artifact_digest,manifest_digest,signature) VALUES($1,'test.sig','test.publisher','test.key',$2,$3,decode(repeat('00',64),'hex'))")
            .bind(tenant)
            .bind(&plugin_digest)
            .bind(&plugin_digest)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO plugin_versions(tenant_id,plugin_id,artifact_digest,manifest,contract_version,plugin_version,signature_id) VALUES($1,'test.crash-resume',$2,'{}'::jsonb,'1.0.0','1.0.0','test.sig')")
            .bind(tenant)
            .bind(&plugin_digest)
            .execute(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("plugin_versions insert: {e}"));
        sqlx::query("INSERT INTO runs(tenant_id,id,principal_id,workflow_id,workflow_version,plugin_id,plugin_digest,grant_id,grant_fence,input_ref,state,temporal_workflow_id,dispatch_state) VALUES($1,$2,$3,$4,'1.0.0','test.crash-resume',$5,$6,1,$7,'RUNNING',$8,'STARTED')")
            .bind(tenant)
            .bind(run_id)
            .bind(principal)
            .bind(&definition.id)
            .bind(&plugin_digest)
            .bind(grant_id)
            .bind(serde_json::to_value(&input_ref)?)
            .bind(run_id.to_string())
            .execute(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("runs insert: {e}"));
        tx.commit().await?;
    }

    // ── First worker: completes step one, then dies mid-step-two. ─────────
    // The worker executes what the interpreter names and records it durably;
    // the crash takes it before step two's outcome is committed.
    let first_worker = {
        let store = store.clone();
        let definition = definition.clone();
        let run_id = run_id;
        let tenant = tenant;
        async move {
            let history = store.read_run_checkpoints(tenant, run_id).await?;
            let interpreter = interpreter_from_checkpoints(&definition, &history);
            let decision = interpreter.next_step();
            assert_eq!(
                decision,
                StepDecision::Execute {
                    logical_step_id: "0001:a".to_string(),
                    node_id: "a".to_string(),
                },
                "fresh worker must start at step one"
            );
            // Execute the step (synthetic work) and record it durably.
            let StepDecision::Execute {
                logical_step_id, ..
            } = decision
            else {
                unreachable!()
            };
            let fence = masonwing_worker::run_resume::checkpoint_fence(&history).unwrap_or(1);
            assert!(
                store
                    .record_run_checkpoint(
                        tenant,
                        run_id,
                        &CheckpointWrite {
                            logical_step_id,
                            state: "SUCCEEDED".to_string(),
                            failure_code: None,
                            fence,
                            output_ref: None,
                        },
                    )
                    .await?
            );
            // Crash: the worker dies before it can even ask for the next step.
            Ok::<_, CommandFailure>(())
        }
    };
    first_worker.await?;

    // ── Second worker: resumes from the durable history, finishes the run. ──
    {
        let history = store.read_run_checkpoints(tenant, run_id).await?;
        assert_eq!(history.len(), 1, "only step one reached the durable log");
        assert_eq!(history[0].state, "SUCCEEDED");

        // The interpreter must not re-issue step one; it picks up at step two.
        let interpreter = interpreter_from_checkpoints(&definition, &history);
        let decision = interpreter.next_step();
        assert_eq!(
            decision,
            StepDecision::Execute {
                logical_step_id: "0002:b".to_string(),
                node_id: "b".to_string(),
            },
            "resume must continue at step two, not replay step one"
        );
        let StepDecision::Execute {
            logical_step_id, ..
        } = decision
        else {
            unreachable!()
        };
        let fence = history[0].fence;
        assert!(
            store
                .record_run_checkpoint(
                    tenant,
                    run_id,
                    &CheckpointWrite {
                        logical_step_id,
                        state: "SUCCEEDED".to_string(),
                        failure_code: None,
                        fence,
                        output_ref: None,
                    },
                )
                .await?,
            "step two must be recorded exactly once"
        );

        // A replayed record for the same logical step is a durable no-op.
        let history = store.read_run_checkpoints(tenant, run_id).await?;
        assert!(
            !store
                .record_run_checkpoint(
                    tenant,
                    run_id,
                    &CheckpointWrite {
                        logical_step_id: "0002:b".to_string(),
                        state: "SUCCEEDED".to_string(),
                        failure_code: None,
                        fence,
                        output_ref: None,
                    },
                )
                .await?,
            "re-issuing the same logical step must be a durable no-op"
        );
        assert_eq!(history.len(), 2);

        // Step three is the last dispatch; after it the traversal completes.
        let interpreter = interpreter_from_checkpoints(&definition, &history);
        let decision = interpreter.next_step();
        assert_eq!(
            decision,
            StepDecision::Execute {
                logical_step_id: "0003:c".to_string(),
                node_id: "c".to_string(),
            }
        );
        let StepDecision::Execute {
            logical_step_id, ..
        } = decision
        else {
            unreachable!()
        };
        assert!(
            store
                .record_run_checkpoint(
                    tenant,
                    run_id,
                    &CheckpointWrite {
                        logical_step_id,
                        state: "SUCCEEDED".to_string(),
                        failure_code: None,
                        fence,
                        output_ref: None,
                    },
                )
                .await?
        );

        let history = store.read_run_checkpoints(tenant, run_id).await?;
        assert_eq!(history.len(), 3);
        assert!(history.iter().all(|c| c.state == "SUCCEEDED"));
        let interpreter = interpreter_from_checkpoints(&definition, &history);
        assert_eq!(interpreter.next_step(), StepDecision::Completed);
    }

    // The run completes through the fenced transition; a stale worker cannot
    // re-drive it afterwards.
    store.mark_run_completed(tenant, run_id, 1).await?;
    let state: String = {
        let mut tx = operator.begin().await?;
        sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
            .bind(tenant.to_string())
            .execute(&mut *tx)
            .await?;
        let state: String =
            sqlx::query_scalar("SELECT state FROM runs WHERE tenant_id=$1 AND id=$2")
                .bind(tenant)
                .bind(run_id)
                .fetch_one(&mut *tx)
                .await?;
        tx.rollback().await?;
        state
    };
    assert_eq!(state, "SUCCEEDED");

    // A late duplicate of step one — the stale write the crash could not have
    // committed — is refused, and nothing rewrites a terminal run.
    let stale = store
        .record_run_checkpoint(
            tenant,
            run_id,
            &CheckpointWrite {
                logical_step_id: "0001:a".to_string(),
                state: "SUCCEEDED".to_string(),
                failure_code: None,
                fence: 1,
                output_ref: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(stale.code, "ILLEGAL_TRANSITION");
    let completed = store
        .mark_run_completed(tenant, run_id, 1)
        .await
        .unwrap_err();
    assert_eq!(completed.code, "ILLEGAL_TRANSITION");

    // Cleanup: remove only this tenant's rows. `tenants` has no tenant_id
    // column, so it is deleted by primary key last.
    let mut tx = operator.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id',$1,true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await?;
    for table in [
        "run_checkpoints",
        "runs",
        "delegations",
        "plugin_versions",
        "artifact_signatures",
        "publisher_keys",
        "membership_roles",
        "memberships",
        "authorization_policies",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE tenant_id=$1"))
            .bind(tenant)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DELETE FROM tenants WHERE id=$1")
        .bind(tenant)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
