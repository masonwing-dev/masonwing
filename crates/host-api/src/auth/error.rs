use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use uuid::Uuid;

use masonwing_authorization_cedar::CedarAuthorizationError;
use masonwing_identity_oidc::OidcError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthError {
    Unauthenticated,
    SessionExpired,
    InvalidCallback,
    ReturnTargetDenied,
    ProviderDenied,
    CsrfRejected,
    MembershipDenied,
    TenantContextMismatch,
    NotFound,
    StepUpRequired,
    AuthorizationDenied,
    AuthorizationEvaluation,
    PolicyUnavailable,
    BackchannelNotQualified,
    DependencyUnavailable,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: String,
    message: String,
    correlation_id: String,
    effect_state: &'static str,
    retryable: bool,
    recovery_action: &'static str,
    details: Vec<ErrorDetail>,
}

#[derive(Debug, Serialize)]
struct ErrorDetail {
    field: String,
    reason: String,
}

impl AuthError {
    fn response_parts(&self) -> (StatusCode, &'static str, &'static str, bool, &'static str) {
        match self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "AUTH_REQUIRED",
                "Authentication is required.",
                false,
                "REAUTHENTICATE",
            ),
            Self::SessionExpired => (
                StatusCode::UNAUTHORIZED,
                "SESSION_EXPIRED",
                "The session has expired.",
                false,
                "REAUTHENTICATE",
            ),
            Self::InvalidCallback => (
                StatusCode::UNAUTHORIZED,
                "AUTH_INVALID_CALLBACK",
                "The authentication callback is invalid.",
                false,
                "REAUTHENTICATE",
            ),
            Self::ReturnTargetDenied => (
                StatusCode::BAD_REQUEST,
                "RETURN_TARGET_DENIED",
                "The return target is not allowed.",
                false,
                "NONE",
            ),
            Self::ProviderDenied => (
                StatusCode::UNAUTHORIZED,
                "AUTH_PROVIDER_ERROR",
                "The identity provider did not complete authentication.",
                false,
                "REAUTHENTICATE",
            ),
            Self::CsrfRejected => (
                StatusCode::FORBIDDEN,
                "CSRF_REJECTED",
                "The request origin or CSRF token is invalid.",
                false,
                "NONE",
            ),
            Self::MembershipDenied => (
                StatusCode::FORBIDDEN,
                "MEMBERSHIP_NOT_ACTIVE",
                "An active tenant membership is required.",
                false,
                "REQUEST_PERMISSION",
            ),
            Self::TenantContextMismatch => (
                StatusCode::FORBIDDEN,
                "TENANT_CONTEXT_MISMATCH",
                "The tenant context does not match the authenticated request.",
                false,
                "REFRESH_SNAPSHOT",
            ),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "RESOURCE_NOT_FOUND",
                "The requested resource was not found.",
                false,
                "NONE",
            ),
            Self::StepUpRequired => (
                StatusCode::FORBIDDEN,
                "STEP_UP_REQUIRED",
                "Recent verified step-up authentication is required.",
                false,
                "REAUTHENTICATE",
            ),
            Self::AuthorizationDenied => (
                StatusCode::FORBIDDEN,
                "AUTHZ_DENIED",
                "The action is not permitted.",
                false,
                "REQUEST_PERMISSION",
            ),
            Self::AuthorizationEvaluation => (
                StatusCode::SERVICE_UNAVAILABLE,
                "AUTHZ_EVALUATION_ERROR",
                "Authorization could not be evaluated safely.",
                false,
                "CONTACT_OPERATOR",
            ),
            Self::PolicyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "AUTHZ_POLICY_UNAVAILABLE",
                "The current authorization policy is unavailable.",
                true,
                "RETRY_READ",
            ),
            Self::BackchannelNotQualified => (
                StatusCode::SERVICE_UNAVAILABLE,
                "AUTH_BACKCHANNEL_NOT_QUALIFIED",
                "Back-channel logout is not qualified in this runtime.",
                false,
                "CONTACT_OPERATOR",
            ),
            Self::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "AUTH_DEPENDENCY_UNAVAILABLE",
                "An authentication dependency is unavailable.",
                true,
                "RETRY_READ",
            ),
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> axum::response::Response {
        let (status, code, message, retryable, recovery_action) = self.response_parts();
        let body = ErrorBody {
            code: code.to_owned(),
            message: message.to_owned(),
            correlation_id: format!("auth:{}", Uuid::new_v4()),
            effect_state: "NOT_SENT",
            retryable,
            recovery_action,
            details: Vec::new(),
        };
        (status, Json(body)).into_response()
    }
}

impl From<OidcError> for AuthError {
    fn from(value: OidcError) -> Self {
        match value {
            OidcError::ReturnTargetDenied => Self::ReturnTargetDenied,
            OidcError::InvalidCallback => Self::InvalidCallback,
            OidcError::ProviderError(_) => Self::ProviderDenied,
            OidcError::DatabaseUnavailable | OidcError::DiscoveryUnavailable => {
                Self::DependencyUnavailable
            }
            OidcError::TokenExchangeFailed | OidcError::InvalidIdToken => Self::InvalidCallback,
            OidcError::StepUpInsufficient => Self::StepUpRequired,
            OidcError::ConfigInvalid | OidcError::IssuerTrustFailed => Self::DependencyUnavailable,
        }
    }
}

impl From<CedarAuthorizationError> for AuthError {
    fn from(value: CedarAuthorizationError) -> Self {
        match value {
            CedarAuthorizationError::Denied => Self::AuthorizationDenied,
            CedarAuthorizationError::Evaluation { .. } => Self::AuthorizationEvaluation,
            CedarAuthorizationError::TenantContextMismatch => Self::TenantContextMismatch,
            CedarAuthorizationError::InvalidPolicy
            | CedarAuthorizationError::InvalidPolicyMetadata => Self::PolicyUnavailable,
            CedarAuthorizationError::InvalidEntity | CedarAuthorizationError::InvalidRequest => {
                Self::AuthorizationEvaluation
            }
        }
    }
}
