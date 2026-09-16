use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use masonwing_contracts::Environment;
use masonwing_host_api::{
    ErrorEffectState, ErrorResponse, LOCAL_COMMAND_PATHS, RecoveryAction, router,
};
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn liveness_is_200_while_write_readiness_truthfully_remains_503() {
    let app = router("masonwing-host-api", Environment::Local);
    let live = app
        .clone()
        .oneshot(Request::get("/health/live").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let ready = app
        .oneshot(Request::get("/health/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(live.status(), StatusCode::OK);
    assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn local_dev_status_matches_scaffold_contract() {
    let response = router("masonwing-worker", Environment::Local)
        .oneshot(Request::get("/dev/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(value["service"], "masonwing-worker");
    assert_eq!(value["environment"], "LOCAL");
    assert_eq!(value["implementation_status"], "NOT_IMPLEMENTED");
    assert_eq!(value["external_mutation_enabled"], false);
    assert_eq!(value["live_budget_microunits"], 0);
    assert_eq!(value["readiness"]["writes"], false);
    assert_eq!(value["readiness"]["critical_adapters_qualified"], false);
    assert_eq!(value["contract_version"], "1.0.0");
}

#[tokio::test]
async fn dev_status_is_not_exposed_outside_local() {
    let response = router("masonwing-host-api", Environment::Integration)
        .oneshot(Request::get("/dev/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn protected_command_denies_missing_auth_before_parsing_malformed_body() {
    let response = router("masonwing-host-api", Environment::Local)
        .oneshot(
            Request::post("/v1/tenants/tenant_b/commands/effect.dispatch")
                .header("content-type", "application/json")
                .body(Body::from("{ definitely-not-json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let error: ErrorResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(error.code, "AUTHENTICATION_REQUIRED");
    assert_eq!(error.effect_state, ErrorEffectState::NotSent);
    assert_eq!(error.recovery_action, RecoveryAction::Reauthenticate);
    assert!(!error.details.is_empty());
}

#[tokio::test]
async fn unverified_credential_reference_never_produces_fake_success_receipt() {
    let response = router("masonwing-host-api", Environment::Local)
        .oneshot(
            Request::post("/v1/tenants/tenant_a/commands/run.start")
                .header("authorization", "Bearer synthetic-unverified")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let error: ErrorResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(error.code, "IDENTITY_ADAPTER_UNAVAILABLE");
    assert_eq!(error.effect_state, ErrorEffectState::NotSent);
    assert_eq!(error.recovery_action, RecoveryAction::ContactOperator);
    assert!(!error.details.is_empty());
}

#[tokio::test]
async fn all_spec_commands_deny_before_body_parse_with_distinct_correlation_ids() {
    assert_eq!(LOCAL_COMMAND_PATHS.len(), 43);
    let app = router("masonwing-host-api", Environment::Local);
    let mut correlations = std::collections::BTreeSet::new();
    for path in LOCAL_COMMAND_PATHS {
        let response = app
            .clone()
            .oneshot(
                Request::post(path.replace("{tenant_id}", "tenant_a"))
                    .header("content-type", "application/json")
                    .body(Body::from("{broken json"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
        let body: ErrorResponse =
            serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(body.effect_state, ErrorEffectState::NotSent);
        assert_eq!(body.recovery_action, RecoveryAction::Reauthenticate);
        assert!(correlations.insert(body.correlation_id));
        assert!(!body.retryable);
    }
}

#[tokio::test]
async fn unknown_path_and_misnamed_cookie_do_not_gain_command_authority() {
    let app = router("masonwing-host-api", Environment::Local);
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/tenants/tenant_a/commands/sql.eval")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = app
        .oneshot(
            Request::post("/v1/tenants/tenant_a/commands/run.start")
                .header("cookie", "another__Host-masonwing_session=not-a-session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
