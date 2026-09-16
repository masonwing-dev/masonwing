use std::{
    env,
    net::SocketAddr,
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use masonwing_contracts::{Environment, ScaffoldStatus, ValueError};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::net::TcpListener;

pub const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";
mod generated_routes;
pub use generated_routes::LOCAL_COMMAND_PATHS;

static CORRELATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub service: String,
    pub environment: Environment,
    pub bind_addr: SocketAddr,
}

impl RuntimeConfig {
    pub fn from_env(service: impl Into<String>) -> Result<Self, StartupError> {
        validate_scaffold_activation_settings(
            env::var("MASONWING_EXTERNAL_MUTATIONS").ok().as_deref(),
            env::var("MASONWING_LIVE_BUDGET_MICROUNITS").ok().as_deref(),
        )?;

        let environment =
            Environment::parse(&env::var("MASONWING_ENV").unwrap_or_else(|_| "LOCAL".to_owned()))?;
        let bind_addr = SocketAddr::from_str(
            &env::var("MASONWING_BIND_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned()),
        )?;
        let config = Self {
            service: service.into(),
            environment,
            bind_addr,
        };
        config.validate_scaffold_startup()?;
        Ok(config)
    }

    pub fn validate_scaffold_startup(&self) -> Result<(), StartupError> {
        if matches!(
            self.environment,
            Environment::Production | Environment::StagingLive
        ) {
            return Err(StartupError::UnsafeConfiguration(
                "unqualified scaffold adapters cannot start in a live environment",
            ));
        }
        Ok(())
    }
}

fn validate_scaffold_activation_settings(
    external_mutations: Option<&str>,
    live_budget_microunits: Option<&str>,
) -> Result<(), StartupError> {
    if let Some(value) = external_mutations {
        let disabled = matches!(value.trim().to_ascii_lowercase().as_str(), "false" | "0");
        if !disabled {
            return Err(StartupError::UnsafeConfiguration(
                "scaffold external mutations must remain disabled",
            ));
        }
    }

    if let Some(value) = live_budget_microunits
        && value.trim().parse::<u64>() != Ok(0)
    {
        return Err(StartupError::UnsafeConfiguration(
            "scaffold live budget must remain zero",
        ));
    }

    Ok(())
}

#[derive(Clone, Debug)]
struct AppState {
    status: ScaffoldStatus,
}

pub fn router(service: impl Into<String>, environment: Environment) -> Router {
    let service = service.into();
    let mut status = ScaffoldStatus::local(service);
    status.environment = environment;
    let state = AppState { status };

    let mut app = Router::new()
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/dev/status", get(dev_status));
    for path in LOCAL_COMMAND_PATHS {
        app = app.route(path, post(protected_command));
    }
    app.with_state(state)
}

pub async fn run_service(service: &'static str) -> Result<(), StartupError> {
    let config = RuntimeConfig::from_env(service)?;
    let listener = TcpListener::bind(config.bind_addr).await?;
    axum::serve(listener, router(config.service, config.environment))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn health_live(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(LiveResponse {
            status: "live",
            service: state.status.service,
        }),
    )
}

async fn health_ready(State(state): State<AppState>) -> impl IntoResponse {
    // Scaffold critical adapters are deliberately unqualified. This endpoint
    // represents write readiness, so it remains 503 until real qualification.
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(state.status.readiness),
    )
}

async fn dev_status(State(state): State<AppState>) -> Response {
    if state.status.environment != Environment::Local {
        return StatusCode::NOT_FOUND.into_response();
    }
    (StatusCode::OK, Json(state.status)).into_response()
}

/// Only registered paths reach this gate. Identity is checked before body extraction.
async fn protected_command(headers: HeaderMap) -> Response {
    if !has_credential_reference(&headers) {
        return problem(
            StatusCode::UNAUTHORIZED,
            "AUTHENTICATION_REQUIRED",
            "A verified server-side session or service credential is required",
            ErrorEffectState::NotSent,
            RecoveryAction::Reauthenticate,
            vec![ErrorDetail::new(
                "authorization",
                "MISSING_CREDENTIAL_REFERENCE",
            )],
        );
    }

    // A header or cookie is only a credential reference. Without a qualified
    // identity adapter the scaffold never treats it as authenticated.
    problem(
        StatusCode::SERVICE_UNAVAILABLE,
        "IDENTITY_ADAPTER_UNAVAILABLE",
        "Identity verification adapter is not qualified",
        ErrorEffectState::NotSent,
        RecoveryAction::ContactOperator,
        vec![ErrorDetail::new("identity_adapter", "ADAPTER_UNQUALIFIED")],
    )
}

fn has_credential_reference(headers: &HeaderMap) -> bool {
    headers.contains_key(axum::http::header::AUTHORIZATION)
        || headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value.split(';').any(|cookie| {
                    cookie.trim().split_once('=').is_some_and(|(name, value)| {
                        name == "__Host-masonwing_session" && !value.is_empty()
                    })
                })
            })
}

fn problem(
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    effect_state: ErrorEffectState,
    recovery_action: RecoveryAction,
    details: Vec<ErrorDetail>,
) -> Response {
    (
        status,
        Json(ErrorResponse {
            code: code.to_owned(),
            message: message.to_owned(),
            correlation_id: next_correlation_id(),
            effect_state,
            retryable: false,
            recovery_action,
            details,
        }),
    )
        .into_response()
}

fn next_correlation_id() -> String {
    let sequence = CORRELATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("corr-{nanos:x}-{sequence:x}")
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            let _ = signal.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[derive(Clone, Debug, Serialize)]
struct LiveResponse {
    status: &'static str,
    service: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorEffectState {
    NotSent,
    Confirmed,
    Unknown,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecoveryAction {
    None,
    RetryRead,
    Reauthenticate,
    ReviewConflict,
    Reconcile,
    RequestPermission,
    ContactOperator,
    RefreshSnapshot,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorDetail {
    pub field: String,
    pub reason: String,
}

impl ErrorDetail {
    fn new(field: &'static str, reason: &'static str) -> Self {
        Self {
            field: field.to_owned(),
            reason: reason.to_owned(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
    pub correlation_id: String,
    pub effect_state: ErrorEffectState,
    pub retryable: bool,
    pub recovery_action: RecoveryAction,
    pub details: Vec<ErrorDetail>,
}

#[derive(Debug, Error)]
pub enum StartupError {
    #[error("UNSAFE_CONFIGURATION: {0}")]
    UnsafeConfiguration(&'static str),
    #[error(transparent)]
    Value(#[from] ValueError),
    #[error(transparent)]
    Address(#[from] std::net::AddrParseError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    // MASONWING@1.0.1 REQ-157 / AC-161: unsafe development scaffold is rejected live.
    #[test]
    fn scaffold_refuses_production_startup() {
        let config = RuntimeConfig {
            service: "masonwing-host-api".to_owned(),
            environment: Environment::Production,
            bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080),
        };
        assert!(matches!(
            config.validate_scaffold_startup(),
            Err(StartupError::UnsafeConfiguration(_))
        ));
    }

    #[test]
    fn scaffold_rejects_attempted_external_mutations() {
        assert!(matches!(
            validate_scaffold_activation_settings(Some("true"), Some("0")),
            Err(StartupError::UnsafeConfiguration(_))
        ));
        assert!(matches!(
            validate_scaffold_activation_settings(Some("1"), Some("0")),
            Err(StartupError::UnsafeConfiguration(_))
        ));
        assert!(validate_scaffold_activation_settings(Some("false"), Some("0")).is_ok());
    }

    #[test]
    fn scaffold_rejects_nonzero_or_malformed_live_budget() {
        for value in ["1", "1000", "-1", "invalid"] {
            assert!(matches!(
                validate_scaffold_activation_settings(Some("false"), Some(value)),
                Err(StartupError::UnsafeConfiguration(_))
            ));
        }
        assert!(validate_scaffold_activation_settings(None, None).is_ok());
    }
}
