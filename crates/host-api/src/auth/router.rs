use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::Redirect,
    routing::{get, post},
};
use masonwing_authorization_cedar::CedarAuthorizationAdapter;
use masonwing_identity_oidc::{OidcError, OidcIdentityAdapter};
use serde::Deserialize;
use tower_sessions::Session;

use super::{
    error::AuthError,
    gate::AuthGate,
    session::{CsrfGuard, OidcFlowPurpose, SessionAuthority, SessionView},
};

#[derive(Clone)]
pub struct AuthState {
    pub oidc: Arc<OidcIdentityAdapter>,
    pub sessions: SessionAuthority,
    pub gate: AuthGate,
    pub csrf: CsrfGuard,
}

impl AuthState {
    pub fn new(
        oidc: Arc<OidcIdentityAdapter>,
        sessions: SessionAuthority,
        cedar: CedarAuthorizationAdapter,
        expected_origin: impl Into<String>,
    ) -> Result<Self, AuthError> {
        let csrf = CsrfGuard::new(expected_origin)?;
        let gate = AuthGate::new(sessions.clone(), cedar);
        Ok(Self {
            oidc,
            sessions,
            gate,
            csrf,
        })
    }
}

pub fn router(state: AuthState) -> Router {
    Router::new()
        .route("/auth/login", get(login))
        .route("/auth/callback", get(callback))
        .route("/auth/logout", post(logout))
        .route(
            "/auth/backchannel-logout",
            post(backchannel_logout_not_qualified),
        )
        .route("/session", get(session_view))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct LoginQuery {
    return_to: Option<String>,
    #[serde(default)]
    step_up: bool,
}

async fn login(
    State(state): State<AuthState>,
    Query(query): Query<LoginQuery>,
    session: Session,
) -> Result<Redirect, AuthError> {
    let start = if query.step_up {
        let auth = state.sessions.peek_authenticated(&session).await?;
        let start = state.oidc.begin_step_up(query.return_to.as_deref()).await?;
        state
            .sessions
            .set_oidc_step_up_binding(&session, &start.browser_binding, &auth)
            .await?;
        start
    } else {
        let start = state.oidc.begin_login(query.return_to.as_deref()).await?;
        state
            .sessions
            .set_oidc_login_binding(&session, &start.browser_binding)
            .await?;
        start
    };
    Ok(Redirect::temporary(&start.authorization_url))
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
    #[allow(dead_code)]
    error_description: Option<String>,
}

async fn callback(
    State(state): State<AuthState>,
    Query(query): Query<CallbackQuery>,
    session: Session,
) -> Result<Redirect, AuthError> {
    let callback_state = query.state.as_deref().ok_or(AuthError::InvalidCallback)?;
    if query.code.is_some() == query.error.is_some() {
        return Err(AuthError::InvalidCallback);
    }
    let pending = state.sessions.pending_oidc_flow(&session).await?;

    let completion = match &pending.purpose {
        OidcFlowPurpose::Login => {
            state
                .oidc
                .complete_callback(
                    callback_state,
                    &pending.browser_binding,
                    query.code.as_deref(),
                    query.error.as_deref(),
                )
                .await
        }
        OidcFlowPurpose::StepUp { principal_id, .. } => {
            state
                .oidc
                .complete_step_up_callback(
                    callback_state,
                    &pending.browser_binding,
                    query.code.as_deref(),
                    query.error.as_deref(),
                    *principal_id,
                )
                .await
        }
    };

    let completed = match completion {
        Ok(completed) => completed,
        Err(error @ OidcError::ProviderError(_))
        | Err(error @ OidcError::TokenExchangeFailed)
        | Err(error @ OidcError::InvalidIdToken)
        | Err(error @ OidcError::StepUpInsufficient) => {
            // A valid state transaction was consumed before these outcomes, so
            // the pending browser flow must not be reused for a replay.
            state.sessions.clear_oidc_browser_flow(&session).await?;
            return Err(error.into());
        }
        Err(error) => return Err(error.into()),
    };

    match pending.purpose {
        OidcFlowPurpose::Login => {
            state
                .sessions
                .establish_authenticated_session(&session, &completed.principal)
                .await?;
        }
        OidcFlowPurpose::StepUp {
            principal_id,
            issuer,
            subject,
        } => {
            // The DB lookup already pins issuer/sub to principal_id, and this
            // second server-session check binds the callback to the exact actor
            // that initiated step-up.
            if completed.principal.principal_id != principal_id
                || completed.principal.issuer != issuer
                || completed.principal.subject != subject
            {
                state.sessions.clear_oidc_browser_flow(&session).await?;
                return Err(AuthError::StepUpRequired);
            }
            let evidence = state.oidc.verify_step_up(&completed)?;
            state.sessions.clear_oidc_browser_flow(&session).await?;
            state
                .sessions
                .record_verified_step_up(&session, &evidence)
                .await?;
        }
    }
    Ok(Redirect::temporary(&completed.return_to))
}

async fn logout(
    State(state): State<AuthState>,
    headers: HeaderMap,
    session: Session,
) -> Result<StatusCode, AuthError> {
    // Read without touching first so a forged unsafe request cannot mutate the
    // inactivity timestamp before Origin+CSRF validation.
    let auth = state.sessions.peek_authenticated(&session).await?;
    state.csrf.verify(&headers, &auth)?;
    state.sessions.logout(&session).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn session_view(
    State(state): State<AuthState>,
    session: Session,
) -> Result<Json<SessionView>, AuthError> {
    Ok(Json(state.sessions.session_view(&session).await?))
}

async fn backchannel_logout_not_qualified() -> Result<StatusCode, AuthError> {
    // REQ-038 requires signature/iss/aud/events/sid-or-sub/jti/replay checks.
    // Keep the endpoint fail-closed until all of those are implemented and
    // qualification-tested; never treat an unverified logout token as action.
    Err(AuthError::BackchannelNotQualified)
}
