//! OIDC identity adapter for the Masonwing BFF.
//!
//! This module keeps the OAuth/OIDC protocol in the backend: Authorization Code
//! with PKCE, server-side one-use login transactions, issuer+subject identity and
//! verified ID-token processing. Browser session/cookie policy lives in the host
//! auth module, while this adapter owns the provider protocol and identity rows.

use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use masonwing_contracts::AdapterQualification;
use masonwing_kernel::ports::{AdapterBoundary, IdentityPort, PortError, VerifiedIdentity};
use openidconnect::core::{
    CoreAuthPrompt, CoreAuthenticationFlow, CoreClient, CoreJsonWebKeySet, CoreJwsSigningAlgorithm,
    CoreProviderMetadata,
};
use openidconnect::reqwest;
use openidconnect::{
    AccessTokenHash, AuthenticationContextClass, AuthorizationCode, ClientId, ClientSecret,
    CsrfToken, IssuerUrl, JsonWebKeySetUrl, Nonce, OAuth2TokenResponse, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenUrl,
};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use thiserror::Error;
use uuid::Uuid;

const DISCOVERY_MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub struct OidcProviderConfig {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    /// Explicit local-development escape hatch. Production configuration should
    /// keep this false; HTTP is then rejected before provider discovery.
    pub allow_loopback_http: bool,
    /// Optional server-side transport origin for the explicit loopback profile.
    /// Example: public/logical issuer `http://localhost:39853`, backend transport
    /// `http://keycloak:8080`. The token `iss` remains the logical issuer.
    pub internal_base_url: Option<String>,
    pub login_transaction_ttl_seconds: i64,
    pub allowed_signing_algorithms: Vec<CoreJwsSigningAlgorithm>,
    /// Step-up requests always send `prompt=login`, `max_age`, and at least one
    /// requested ACR. The returned token must also contain one configured strong
    /// AMR. A password-only login therefore never satisfies step-up.
    pub step_up_max_age_seconds: u64,
    pub step_up_clock_skew_seconds: i64,
    pub step_up_acr_values: Vec<String>,
    pub step_up_strong_amr_values: Vec<String>,
}

impl OidcProviderConfig {
    pub fn keycloak(
        issuer: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: Option<String>,
        redirect_uri: impl Into<String>,
    ) -> Self {
        Self {
            issuer: issuer.into(),
            client_id: client_id.into(),
            client_secret,
            redirect_uri: redirect_uri.into(),
            allow_loopback_http: false,
            internal_base_url: None,
            login_transaction_ttl_seconds: 300,
            allowed_signing_algorithms: vec![CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256],
            step_up_max_age_seconds: 0,
            step_up_clock_skew_seconds: 60,
            // Operators must map this to a provider authentication flow that
            // actually performs MFA. The synthetic local password-only realm
            // intentionally does not satisfy this value.
            step_up_acr_values: vec!["urn:masonwing:loa:2".to_owned()],
            step_up_strong_amr_values: vec![
                "otp".to_owned(),
                "totp".to_owned(),
                "mfa".to_owned(),
                "webauthn".to_owned(),
                "hwk".to_owned(),
            ],
        }
    }
}

#[derive(Clone)]
pub struct OidcIdentityAdapter {
    pool: PgPool,
    provider: CoreProviderMetadata,
    client_id: ClientId,
    client_secret: Option<ClientSecret>,
    redirect_uri: RedirectUrl,
    http_client: reqwest::Client,
    issuer: IssuerUrl,
    transaction_ttl_seconds: i64,
    allowed_signing_algorithms: Vec<CoreJwsSigningAlgorithm>,
    step_up_max_age_seconds: u64,
    step_up_clock_skew_seconds: i64,
    step_up_acr_values: Vec<String>,
    step_up_strong_amr_values: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginStart {
    pub authorization_url: String,
    pub state: String,
    /// Store this value only in the server-side browser session. It binds the
    /// callback to the browser that initiated the transaction.
    pub browser_binding: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedOidcPrincipal {
    pub principal_id: Uuid,
    pub issuer: String,
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
}

#[derive(Clone, Debug, FromRow)]
struct ConsumedLoginTransaction {
    nonce: String,
    pkce_verifier: String,
    return_to: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletedLogin {
    pub principal: VerifiedOidcPrincipal,
    pub return_to: String,
    auth_time: Option<DateTime<Utc>>,
    auth_context_ref: Option<String>,
    auth_method_refs: Vec<String>,
}

/// Evidence constructed only after the OIDC adapter has verified a fresh ID
/// token, requested ACR and a configured strong AMR. Fields are intentionally
/// private so browser/plugin input cannot construct a step-up proof directly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedStepUpEvidence {
    principal_id: Uuid,
    issuer: String,
    subject: String,
    verified_at: DateTime<Utc>,
    auth_time: DateTime<Utc>,
    acr: String,
    amr: Vec<String>,
}

impl VerifiedStepUpEvidence {
    pub fn principal_id(&self) -> Uuid {
        self.principal_id
    }

    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn verified_at(&self) -> DateTime<Utc> {
        self.verified_at
    }

    pub fn auth_time(&self) -> DateTime<Utc> {
        self.auth_time
    }

    pub fn acr(&self) -> &str {
        &self.acr
    }

    pub fn amr(&self) -> &[String] {
        &self.amr
    }
}

#[derive(Debug, Error)]
pub enum OidcError {
    #[error("OIDC_CONFIG_INVALID")]
    ConfigInvalid,
    #[error("ISSUER_TRUST_FAILED")]
    IssuerTrustFailed,
    #[error("OIDC_DISCOVERY_UNAVAILABLE")]
    DiscoveryUnavailable,
    #[error("RETURN_TARGET_DENIED")]
    ReturnTargetDenied,
    #[error("AUTH_INVALID_CALLBACK")]
    InvalidCallback,
    #[error("AUTH_PROVIDER_ERROR:{0}")]
    ProviderError(String),
    #[error("OIDC_TOKEN_EXCHANGE_FAILED")]
    TokenExchangeFailed,
    #[error("OIDC_ID_TOKEN_INVALID")]
    InvalidIdToken,
    #[error("STEP_UP_REQUIRED")]
    StepUpInsufficient,
    #[error("OIDC_DATABASE_UNAVAILABLE")]
    DatabaseUnavailable,
}

impl OidcIdentityAdapter {
    pub async fn discover(pool: PgPool, config: OidcProviderConfig) -> Result<Self, OidcError> {
        if config.login_transaction_ttl_seconds <= 0
            || config.allowed_signing_algorithms.is_empty()
            || config.step_up_clock_skew_seconds < 0
            || config.step_up_clock_skew_seconds > 300
            || config.step_up_max_age_seconds > 300
            || config.step_up_acr_values.is_empty()
            || config
                .step_up_acr_values
                .iter()
                .any(|value| value.is_empty())
            || config.step_up_strong_amr_values.is_empty()
            || config
                .step_up_strong_amr_values
                .iter()
                .any(|value| value.is_empty() || matches!(value.as_str(), "pwd" | "password"))
        {
            return Err(OidcError::ConfigInvalid);
        }

        let issuer = IssuerUrl::new(config.issuer.clone()).map_err(|_| OidcError::ConfigInvalid)?;
        let redirect_uri =
            RedirectUrl::new(config.redirect_uri).map_err(|_| OidcError::ConfigInvalid)?;
        validate_issuer_transport(issuer.url(), config.allow_loopback_http)?;
        let internal_base = parse_internal_base(
            config.internal_base_url.as_deref(),
            issuer.url(),
            config.allow_loopback_http,
        )?;

        let http_client = reqwest::ClientBuilder::new()
            // OIDC discovery/JWKS/token redirects are intentionally disabled:
            // accepting them would turn provider metadata into an SSRF pivot.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| OidcError::ConfigInvalid)?;

        let discovery_url = issuer
            .join(".well-known/openid-configuration")
            .map_err(|_| OidcError::ConfigInvalid)?;
        validate_same_origin(issuer.url(), &discovery_url, config.allow_loopback_http)?;

        let discovery_transport = transport_url(&discovery_url, internal_base.as_ref())?;
        let response = http_client
            .get(discovery_transport)
            .send()
            .await
            .map_err(|_| OidcError::DiscoveryUnavailable)?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|len| len > DISCOVERY_MAX_BYTES as u64)
        {
            return Err(OidcError::DiscoveryUnavailable);
        }
        let body = response
            .bytes()
            .await
            .map_err(|_| OidcError::DiscoveryUnavailable)?;
        if body.len() > DISCOVERY_MAX_BYTES {
            return Err(OidcError::DiscoveryUnavailable);
        }

        let mut provider: CoreProviderMetadata =
            serde_json::from_slice(&body).map_err(|_| OidcError::DiscoveryUnavailable)?;
        if provider.issuer() != &issuer {
            return Err(OidcError::IssuerTrustFailed);
        }

        validate_same_origin(
            issuer.url(),
            provider.authorization_endpoint().url(),
            config.allow_loopback_http,
        )?;
        let token_endpoint = provider
            .token_endpoint()
            .ok_or(OidcError::IssuerTrustFailed)?;
        validate_same_origin(
            issuer.url(),
            token_endpoint.url(),
            config.allow_loopback_http,
        )?;
        validate_same_origin(
            issuer.url(),
            provider.jwks_uri().url(),
            config.allow_loopback_http,
        )?;

        if !provider
            .id_token_signing_alg_values_supported()
            .iter()
            .any(|alg| config.allowed_signing_algorithms.contains(alg))
        {
            return Err(OidcError::IssuerTrustFailed);
        }

        // Only fetch JWKS after every endpoint has passed the trust boundary.
        let jwks_transport = transport_url(provider.jwks_uri().url(), internal_base.as_ref())?;
        let jwks_transport = JsonWebKeySetUrl::new(jwks_transport.to_string())
            .map_err(|_| OidcError::IssuerTrustFailed)?;
        let jwks = CoreJsonWebKeySet::fetch_async(&jwks_transport, &http_client)
            .await
            .map_err(|_| OidcError::DiscoveryUnavailable)?;
        provider = provider.set_jwks(jwks);
        if let Some(internal_base) = internal_base.as_ref() {
            let logical_token = provider
                .token_endpoint()
                .ok_or(OidcError::IssuerTrustFailed)?;
            let token_transport = transport_url(logical_token.url(), Some(internal_base))?;
            provider = provider.set_token_endpoint(Some(
                TokenUrl::new(token_transport.to_string())
                    .map_err(|_| OidcError::IssuerTrustFailed)?,
            ));
        }

        Ok(Self {
            pool,
            provider,
            client_id: ClientId::new(config.client_id),
            client_secret: config.client_secret.map(ClientSecret::new),
            redirect_uri,
            http_client,
            issuer,
            transaction_ttl_seconds: config.login_transaction_ttl_seconds,
            allowed_signing_algorithms: config.allowed_signing_algorithms,
            step_up_max_age_seconds: config.step_up_max_age_seconds,
            step_up_clock_skew_seconds: config.step_up_clock_skew_seconds,
            step_up_acr_values: config.step_up_acr_values,
            step_up_strong_amr_values: config.step_up_strong_amr_values,
        })
    }

    pub fn issuer(&self) -> &str {
        self.issuer.as_str()
    }

    pub async fn begin_login(&self, return_to: Option<&str>) -> Result<LoginStart, OidcError> {
        self.begin_authorization(return_to, false).await
    }

    pub async fn begin_step_up(&self, return_to: Option<&str>) -> Result<LoginStart, OidcError> {
        self.begin_authorization(return_to, true).await
    }

    async fn begin_authorization(
        &self,
        return_to: Option<&str>,
        step_up: bool,
    ) -> Result<LoginStart, OidcError> {
        let return_to = validate_return_to(return_to.unwrap_or("/"))?.to_owned();
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let state = random_protocol_secret();
        let nonce = random_protocol_secret();
        let browser_binding = random_protocol_secret();

        let client = CoreClient::from_provider_metadata(
            self.provider.clone(),
            self.client_id.clone(),
            self.client_secret.clone(),
        )
        .set_redirect_uri(self.redirect_uri.clone());
        let state_for_url = state.clone();
        let nonce_for_url = nonce.clone();
        let mut authorization_request = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                move || CsrfToken::new(state_for_url),
                move || Nonce::new(nonce_for_url),
            )
            .add_scope(Scope::new("profile".to_owned()))
            .add_scope(Scope::new("email".to_owned()))
            .set_pkce_challenge(pkce_challenge);
        if step_up {
            authorization_request = authorization_request
                .add_prompt(CoreAuthPrompt::Login)
                .set_max_age(StdDuration::from_secs(self.step_up_max_age_seconds));
            for acr in &self.step_up_acr_values {
                authorization_request = authorization_request
                    .add_auth_context_value(AuthenticationContextClass::new(acr.clone()));
            }
        }
        let (authorization_url, _, _) = authorization_request.url();

        let expires_at = Utc::now() + Duration::seconds(self.transaction_ttl_seconds);
        sqlx::query(
            r#"
            INSERT INTO oidc_auth_transactions
                (state, browser_binding, issuer, nonce, pkce_verifier, return_to, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(&state)
        .bind(&browser_binding)
        .bind(self.issuer.as_str())
        .bind(&nonce)
        .bind(pkce_verifier.secret())
        .bind(&return_to)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(|_| OidcError::DatabaseUnavailable)?;

        Ok(LoginStart {
            authorization_url: authorization_url.to_string(),
            state,
            browser_binding,
        })
    }

    /// Consume state exactly once before any provider token exchange. Concurrent
    /// callback replays race on one DELETE ... RETURNING row; only one can win.
    pub async fn complete_callback(
        &self,
        state: &str,
        browser_binding: &str,
        code: Option<&str>,
        provider_error: Option<&str>,
    ) -> Result<CompletedLogin, OidcError> {
        self.complete_callback_inner(state, browser_binding, code, provider_error, None)
            .await
    }

    /// Step-up callback variant. The returned issuer/sub must already resolve to
    /// the exact principal that initiated step-up; an alternate IdP account is
    /// denied without creating a new Masonwing identity row.
    pub async fn complete_step_up_callback(
        &self,
        state: &str,
        browser_binding: &str,
        code: Option<&str>,
        provider_error: Option<&str>,
        expected_principal_id: Uuid,
    ) -> Result<CompletedLogin, OidcError> {
        self.complete_callback_inner(
            state,
            browser_binding,
            code,
            provider_error,
            Some(expected_principal_id),
        )
        .await
    }

    async fn complete_callback_inner(
        &self,
        state: &str,
        browser_binding: &str,
        code: Option<&str>,
        provider_error: Option<&str>,
        expected_principal_id: Option<Uuid>,
    ) -> Result<CompletedLogin, OidcError> {
        if state.is_empty() || (code.is_some() == provider_error.is_some()) {
            return Err(OidcError::InvalidCallback);
        }

        let transaction = sqlx::query_as::<_, ConsumedLoginTransaction>(
            r#"
            DELETE FROM oidc_auth_transactions
            WHERE state = $1
              AND browser_binding = $2
              AND issuer = $3
              AND expires_at > now()
            RETURNING nonce, pkce_verifier, return_to
            "#,
        )
        .bind(state)
        .bind(browser_binding)
        .bind(self.issuer.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| OidcError::DatabaseUnavailable)?
        .ok_or(OidcError::InvalidCallback)?;

        if let Some(error) = provider_error {
            return Err(OidcError::ProviderError(sanitize_provider_error(error)));
        }

        let client = CoreClient::from_provider_metadata(
            self.provider.clone(),
            self.client_id.clone(),
            self.client_secret.clone(),
        )
        .set_redirect_uri(self.redirect_uri.clone());
        let token_response = client
            .exchange_code(AuthorizationCode::new(code.unwrap_or_default().to_owned()))
            .map_err(|_| OidcError::TokenExchangeFailed)?
            .set_pkce_verifier(PkceCodeVerifier::new(transaction.pkce_verifier))
            .request_async(&self.http_client)
            .await
            .map_err(|_| OidcError::TokenExchangeFailed)?;

        let id_token = token_response
            .extra_fields()
            .id_token()
            .ok_or(OidcError::InvalidIdToken)?;
        let verifier = client
            .id_token_verifier()
            .set_allowed_algs(self.allowed_signing_algorithms.clone());
        let nonce = Nonce::new(transaction.nonce);
        let claims = id_token
            .claims(&verifier, &nonce)
            .map_err(|_| OidcError::InvalidIdToken)?;

        if let Some(expected_hash) = claims.access_token_hash() {
            let signing_alg = id_token
                .signing_alg()
                .map_err(|_| OidcError::InvalidIdToken)?;
            let signing_key = id_token
                .signing_key(&verifier)
                .map_err(|_| OidcError::InvalidIdToken)?;
            let actual_hash = AccessTokenHash::from_token(
                token_response.access_token(),
                signing_alg,
                signing_key,
            )
            .map_err(|_| OidcError::InvalidIdToken)?;
            if &actual_hash != expected_hash {
                return Err(OidcError::InvalidIdToken);
            }
        }

        let subject = claims.subject().as_str().to_owned();
        let email = claims.email().map(|value| value.as_str().to_owned());
        let email_verified = claims.email_verified().unwrap_or(false);
        let auth_time = claims.auth_time();
        let auth_context_ref = claims
            .auth_context_ref()
            .map(|value| value.as_ref().to_owned());
        let auth_method_refs = claims
            .auth_method_refs()
            .map(|values| {
                values
                    .iter()
                    .map(|value| value.as_str().to_owned())
                    .collect()
            })
            .unwrap_or_default();
        let principal_id = match expected_principal_id {
            Some(expected) => {
                self.resolve_existing_step_up_identity(
                    expected,
                    &subject,
                    email.as_deref(),
                    email_verified,
                )
                .await?
            }
            None => {
                self.upsert_verified_identity(&subject, email.as_deref(), email_verified)
                    .await?
            }
        };

        Ok(CompletedLogin {
            principal: VerifiedOidcPrincipal {
                principal_id,
                issuer: self.issuer.as_str().to_owned(),
                subject,
                email,
                email_verified,
            },
            return_to: transaction.return_to,
            auth_time,
            auth_context_ref,
            auth_method_refs,
        })
    }

    /// Convert a completed OIDC code flow into a step-up proof only when the IdP
    /// token demonstrates fresh authentication at the requested ACR and at least
    /// one configured strong AMR. `pwd`/password alone is never sufficient.
    pub fn verify_step_up(
        &self,
        completed: &CompletedLogin,
    ) -> Result<VerifiedStepUpEvidence, OidcError> {
        self.verify_step_up_at(completed, Utc::now())
    }

    fn verify_step_up_at(
        &self,
        completed: &CompletedLogin,
        now: DateTime<Utc>,
    ) -> Result<VerifiedStepUpEvidence, OidcError> {
        verify_step_up_claims(
            completed,
            now,
            self.step_up_max_age_seconds,
            self.step_up_clock_skew_seconds,
            &self.step_up_acr_values,
            &self.step_up_strong_amr_values,
        )
    }

    pub async fn cleanup_expired_transactions(&self) -> Result<u64, OidcError> {
        let result = sqlx::query("DELETE FROM oidc_auth_transactions WHERE expires_at <= now()")
            .execute(&self.pool)
            .await
            .map_err(|_| OidcError::DatabaseUnavailable)?;
        Ok(result.rows_affected())
    }

    async fn upsert_verified_identity(
        &self,
        subject: &str,
        email: Option<&str>,
        email_verified: bool,
    ) -> Result<Uuid, OidcError> {
        let proposed_principal = Uuid::new_v4();
        sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO oidc_identities
                (principal_id, issuer, subject, email, email_verified, last_login_at)
            VALUES ($1, $2, $3, $4, $5, now())
            ON CONFLICT (issuer, subject) DO UPDATE
            SET email = EXCLUDED.email,
                email_verified = EXCLUDED.email_verified,
                last_login_at = now()
            RETURNING principal_id
            "#,
        )
        .bind(proposed_principal)
        .bind(self.issuer.as_str())
        .bind(subject)
        .bind(email)
        .bind(email_verified)
        .fetch_one(&self.pool)
        .await
        .map_err(|_| OidcError::DatabaseUnavailable)
    }

    async fn resolve_existing_step_up_identity(
        &self,
        expected_principal_id: Uuid,
        subject: &str,
        email: Option<&str>,
        email_verified: bool,
    ) -> Result<Uuid, OidcError> {
        sqlx::query_scalar::<_, Uuid>(
            r#"
            UPDATE oidc_identities
            SET email = $4,
                email_verified = $5,
                last_login_at = now()
            WHERE principal_id = $1
              AND issuer = $2
              AND subject = $3
            RETURNING principal_id
            "#,
        )
        .bind(expected_principal_id)
        .bind(self.issuer.as_str())
        .bind(subject)
        .bind(email)
        .bind(email_verified)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| OidcError::DatabaseUnavailable)?
        .ok_or(OidcError::StepUpInsufficient)
    }
}

fn verify_step_up_claims(
    completed: &CompletedLogin,
    now: DateTime<Utc>,
    max_age_seconds: u64,
    clock_skew_seconds: i64,
    allowed_acr_values: &[String],
    strong_amr_values: &[String],
) -> Result<VerifiedStepUpEvidence, OidcError> {
    let auth_time = completed.auth_time.ok_or(OidcError::StepUpInsufficient)?;
    let skew = Duration::seconds(clock_skew_seconds);
    if auth_time > now + skew || now - auth_time > Duration::seconds(max_age_seconds as i64) + skew
    {
        return Err(OidcError::StepUpInsufficient);
    }

    let acr = completed
        .auth_context_ref
        .as_deref()
        .filter(|actual| allowed_acr_values.iter().any(|allowed| allowed == actual))
        .ok_or(OidcError::StepUpInsufficient)?;
    let has_strong_amr = completed.auth_method_refs.iter().any(|actual| {
        strong_amr_values
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(actual))
    });
    if !has_strong_amr {
        return Err(OidcError::StepUpInsufficient);
    }

    Ok(VerifiedStepUpEvidence {
        principal_id: completed.principal.principal_id,
        issuer: completed.principal.issuer.clone(),
        subject: completed.principal.subject.clone(),
        verified_at: now,
        auth_time,
        acr: acr.to_owned(),
        amr: completed.auth_method_refs.clone(),
    })
}

/// Reject open redirects before a login transaction exists.
pub fn validate_return_to(value: &str) -> Result<&str, OidcError> {
    if value.is_empty()
        || value.len() > 1024
        || !value.starts_with('/')
        || value.starts_with("//")
        || value.contains('\\')
        || value.contains('\r')
        || value.contains('\n')
        || value.chars().any(char::is_control)
    {
        return Err(OidcError::ReturnTargetDenied);
    }

    let lower = value.to_ascii_lowercase();
    if [
        "%2f", "%5c", "%0d", "%0a", "%252f", "%255c", "%250d", "%250a",
    ]
    .iter()
    .any(|encoded| lower.contains(encoded))
    {
        return Err(OidcError::ReturnTargetDenied);
    }
    Ok(value)
}

fn validate_issuer_transport(
    url: &reqwest::Url,
    allow_loopback_http: bool,
) -> Result<(), OidcError> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(OidcError::IssuerTrustFailed);
    }
    match url.scheme() {
        "https" => Ok(()),
        "http" if allow_loopback_http && is_loopback_host(url.host_str()) => Ok(()),
        _ => Err(OidcError::IssuerTrustFailed),
    }
}

fn parse_internal_base(
    value: Option<&str>,
    logical_issuer: &reqwest::Url,
    allow_loopback_http: bool,
) -> Result<Option<reqwest::Url>, OidcError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if !allow_loopback_http
        || logical_issuer.scheme() != "http"
        || !is_loopback_host(logical_issuer.host_str())
    {
        return Err(OidcError::IssuerTrustFailed);
    }
    let base = reqwest::Url::parse(value).map_err(|_| OidcError::ConfigInvalid)?;
    if !matches!(base.scheme(), "http" | "https")
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
        || !matches!(base.path(), "" | "/")
    {
        return Err(OidcError::IssuerTrustFailed);
    }
    Ok(Some(base))
}

fn transport_url(
    logical: &reqwest::Url,
    internal_base: Option<&reqwest::Url>,
) -> Result<reqwest::Url, OidcError> {
    let Some(base) = internal_base else {
        return Ok(logical.clone());
    };
    let mut rewritten = logical.clone();
    rewritten
        .set_scheme(base.scheme())
        .map_err(|_| OidcError::IssuerTrustFailed)?;
    rewritten
        .set_host(base.host_str())
        .map_err(|_| OidcError::IssuerTrustFailed)?;
    rewritten
        .set_port(base.port())
        .map_err(|_| OidcError::IssuerTrustFailed)?;
    Ok(rewritten)
}

fn validate_same_origin(
    issuer: &reqwest::Url,
    endpoint: &reqwest::Url,
    allow_loopback_http: bool,
) -> Result<(), OidcError> {
    validate_issuer_transport(endpoint, allow_loopback_http)?;
    if endpoint.scheme() != issuer.scheme()
        || endpoint.host_str() != issuer.host_str()
        || endpoint.port_or_known_default() != issuer.port_or_known_default()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
    {
        return Err(OidcError::IssuerTrustFailed);
    }
    Ok(())
}

fn is_loopback_host(host: Option<&str>) -> bool {
    matches!(host, Some("localhost" | "127.0.0.1" | "::1" | "[::1]"))
}

fn random_protocol_secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn sanitize_provider_error(error: &str) -> String {
    let clean = error
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        .take(64)
        .collect::<String>();
    if clean.is_empty() {
        "provider_error".to_owned()
    } else {
        clean
    }
}

impl AdapterBoundary for OidcIdentityAdapter {
    fn adapter_name(&self) -> &'static str {
        "identity-oidc"
    }

    fn qualification(&self) -> AdapterQualification {
        AdapterQualification::Qualified
    }
}

impl IdentityPort for OidcIdentityAdapter {
    fn verify_session_reference(
        &self,
        _session_reference: &str,
    ) -> Result<VerifiedIdentity, PortError> {
        // Browser sessions are asynchronous SQLx-backed host state. The legacy
        // synchronous kernel port cannot safely resolve them without blocking;
        // host call sites use SessionAuthority instead.
        Err(PortError::Unavailable {
            adapter: self.adapter_name(),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OidcTransactionAudit {
    pub issuer: String,
    pub expires_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completed_with_auth(now: DateTime<Utc>, acr: Option<&str>, amr: &[&str]) -> CompletedLogin {
        CompletedLogin {
            principal: VerifiedOidcPrincipal {
                principal_id: Uuid::new_v4(),
                issuer: "https://id.example.test/realms/masonwing".into(),
                subject: "subject-1".into(),
                email: Some("user@example.test".into()),
                email_verified: true,
            },
            return_to: "/settings/security".into(),
            auth_time: Some(now),
            auth_context_ref: acr.map(str::to_owned),
            auth_method_refs: amr.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    // MASONWING@1.0.1 REQ-029 / AC-030
    #[test]
    fn return_to_accepts_only_local_paths() {
        assert_eq!(
            validate_return_to("/settings?tab=auth").unwrap(),
            "/settings?tab=auth"
        );
        for denied in [
            "https://evil.test/",
            "//evil.test/",
            "/%2Fevil.test/",
            "/%252Fevil.test/",
            "/\\evil.test/",
            "/ok%0d%0aLocation:evil",
            "",
        ] {
            assert!(matches!(
                validate_return_to(denied),
                Err(OidcError::ReturnTargetDenied)
            ));
        }
    }

    // MASONWING@1.0.1 REQ-031 / AC-032
    #[test]
    fn provider_endpoint_must_share_trusted_issuer_origin() {
        let issuer = reqwest::Url::parse("https://id.example.test/realms/masonwing").unwrap();
        let good = reqwest::Url::parse("https://id.example.test/realms/masonwing/certs").unwrap();
        let evil = reqwest::Url::parse("https://169.254.169.254/latest/meta-data").unwrap();
        assert!(validate_same_origin(&issuer, &good, false).is_ok());
        assert!(matches!(
            validate_same_origin(&issuer, &evil, false),
            Err(OidcError::IssuerTrustFailed)
        ));
    }

    #[test]
    fn insecure_issuer_requires_explicit_loopback_profile() {
        let loopback = reqwest::Url::parse("http://127.0.0.1:39855/realms/masonwing").unwrap();
        assert!(matches!(
            validate_issuer_transport(&loopback, false),
            Err(OidcError::IssuerTrustFailed)
        ));
        assert!(validate_issuer_transport(&loopback, true).is_ok());

        let remote_http = reqwest::Url::parse("http://id.example.test/realms/masonwing").unwrap();
        assert!(matches!(
            validate_issuer_transport(&remote_http, true),
            Err(OidcError::IssuerTrustFailed)
        ));
    }

    #[test]
    fn loopback_internal_transport_rewrites_origin_only() {
        let issuer = reqwest::Url::parse("http://localhost:39853/realms/masonwing").unwrap();
        let base = parse_internal_base(Some("http://keycloak:8080"), &issuer, true)
            .unwrap()
            .unwrap();
        let logical = reqwest::Url::parse(
            "http://localhost:39853/realms/masonwing/protocol/openid-connect/token",
        )
        .unwrap();
        let rewritten = transport_url(&logical, Some(&base)).unwrap();
        assert_eq!(
            rewritten.as_str(),
            "http://keycloak:8080/realms/masonwing/protocol/openid-connect/token"
        );
        assert!(matches!(
            parse_internal_base(Some("http://169.254.169.254"), &issuer, false),
            Err(OidcError::IssuerTrustFailed)
        ));
    }

    #[test]
    fn provider_error_is_sanitized_before_surface() {
        assert_eq!(sanitize_provider_error("access_denied"), "access_denied");
        assert_eq!(
            sanitize_provider_error("denied\r\nSet-Cookie: secret=1"),
            "deniedSet-Cookiesecret1"
        );
    }

    // MASONWING@1.0.1 REQ-041 / AC-043
    #[test]
    fn password_only_authentication_never_satisfies_step_up() {
        let now = Utc::now();
        let completed = completed_with_auth(now, Some("urn:masonwing:loa:2"), &["pwd"]);
        let result = verify_step_up_claims(
            &completed,
            now,
            0,
            60,
            &["urn:masonwing:loa:2".into()],
            &["otp".into(), "webauthn".into()],
        );
        assert!(matches!(result, Err(OidcError::StepUpInsufficient)));
    }

    #[test]
    fn fresh_matching_acr_and_strong_amr_produce_step_up_evidence() {
        let now = Utc::now();
        let completed = completed_with_auth(now, Some("urn:masonwing:loa:2"), &["pwd", "otp"]);
        let evidence = verify_step_up_claims(
            &completed,
            now,
            0,
            60,
            &["urn:masonwing:loa:2".into()],
            &["otp".into(), "webauthn".into()],
        )
        .unwrap();
        assert_eq!(evidence.principal_id(), completed.principal.principal_id);
        assert_eq!(evidence.verified_at(), now);
        assert_eq!(evidence.acr(), "urn:masonwing:loa:2");
        assert_eq!(evidence.amr(), &["pwd".to_owned(), "otp".to_owned()]);
    }

    #[test]
    fn stale_or_wrong_acr_step_up_is_denied() {
        let now = Utc::now();
        let stale = completed_with_auth(
            now - Duration::minutes(3),
            Some("urn:masonwing:loa:2"),
            &["otp"],
        );
        assert!(matches!(
            verify_step_up_claims(
                &stale,
                now,
                0,
                60,
                &["urn:masonwing:loa:2".into()],
                &["otp".into()],
            ),
            Err(OidcError::StepUpInsufficient)
        ));

        let wrong_acr = completed_with_auth(now, Some("0"), &["otp"]);
        assert!(matches!(
            verify_step_up_claims(
                &wrong_acr,
                now,
                0,
                60,
                &["urn:masonwing:loa:2".into()],
                &["otp".into()],
            ),
            Err(OidcError::StepUpInsufficient)
        ));
    }
}
