//! Browser BFF and the tenant command/read boundary. Authentication and CSRF
//! checks precede body reads. Domain mutations remain inside the application
//! repository, which rechecks current authority at transaction commit admission.

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{
        Path, Query, Request, State,
        rejection::{PathRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use bytes::Bytes;
use futures_util::TryStreamExt;
use masonwing_artifacts_adapter::{ArtifactStoreAdapter, ClamAvScanner, S3Config};
use masonwing_authorization_cedar::{CedarAuthorizationAdapter, TrustedResource};
use masonwing_contract_validation::{MAX_COMMAND_BYTES, SchemaCatalog, fingerprint};
use masonwing_contracts::{Environment, PrincipalId, ResourceId, TenantId, wire::ReceiptState};
use masonwing_data_postgres::{
    InviteTokenKey, MembershipAcceptBootstrapCommand, PostgresStore, VerifiedInviteAcceptIdentity,
};
use masonwing_identity_oidc::{OidcIdentityAdapter, OidcProviderConfig};
use masonwing_kernel::runtime::{
    AuthorizedCommand, CommandActor, CommandFailure, CommandRepository, FailureKind, PageQuery,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use std::{
    collections::BTreeMap,
    env, io,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::io::AsyncReadExt;
use tokio_util::io::StreamReader;
use tower_sessions::Session;
use uuid::Uuid;

use crate::{
    ErrorDetail, ErrorEffectState, ErrorResponse, RecoveryAction, StartupError,
    auth::{
        self, AuthError, AuthState, AuthorizationInput, SessionAuthority, SessionCookieProfile,
        VerifiedActor,
    },
    next_correlation_id,
};

const MAX_ARTIFACT_BYTES: u64 = 1024 * 1024 * 1024;
const BASELINE_TYPES: &[&str] = &[
    "Run",
    "Membership",
    "Delegation",
    "EffectIntent",
    "Connection",
    "CostReservation",
    "PluginManifest",
    "UploadSession",
    "ConnectorAuthorization",
];

#[derive(Clone)]
pub struct RuntimeState {
    pub store: Arc<PostgresStore>,
    pub auth: AuthState,
    pub invite_key: InviteTokenKey,
    pub environment: Environment,
}

#[derive(Deserialize)]
struct OperationEntry {
    operation: String,
    step_up: bool,
}
#[derive(Deserialize)]
struct OperationCatalog {
    operations: Vec<OperationEntry>,
}
static OPERATIONS: OnceLock<BTreeMap<String, bool>> = OnceLock::new();

fn operations() -> &'static BTreeMap<String, bool> {
    OPERATIONS.get_or_init(|| {
        let catalog: OperationCatalog =
            serde_json::from_str(include_str!("../../../contracts/masonwing/operations.json"))
                .expect("generated operation catalog is checked during build tests");
        catalog
            .operations
            .into_iter()
            .map(|op| (op.operation, op.step_up))
            .collect()
    })
}

pub async fn from_env(environment: Environment) -> Result<Router, StartupError> {
    let database_url = required("DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(24)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await
        .map_err(|_| StartupError::Dependency("DATABASE_UNAVAILABLE"))?;
    let local = environment == Environment::Local;
    let mut oidc = OidcProviderConfig::keycloak(
        required("OIDC_ISSUER")?,
        required("OIDC_CLIENT_ID")?,
        env::var("OIDC_CLIENT_SECRET").ok(),
        required("OIDC_REDIRECT_URI")?,
    );
    oidc.allow_loopback_http = local;
    oidc.internal_base_url = env::var("OIDC_INTERNAL_BASE_URL").ok();
    if let Ok(values) = env::var("OIDC_STEP_UP_ACR_VALUES") {
        let values: Vec<_> = values
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .collect();
        if values.is_empty() {
            return Err(StartupError::UnsafeConfiguration(
                "step-up ACR allowlist is empty",
            ));
        }
        oidc.step_up_acr_values = values;
    }
    let identity = OidcIdentityAdapter::discover(pool.clone(), oidc)
        .await
        .map_err(|_| StartupError::Dependency("OIDC_DISCOVERY_UNAVAILABLE"))?;
    let sessions = SessionAuthority::new(pool.clone());
    let auth = AuthState::new(
        Arc::new(identity),
        sessions.clone(),
        CedarAuthorizationAdapter,
        required("MASONWING_PUBLIC_ORIGIN")?,
    )
    .map_err(|_| StartupError::Dependency("AUTH_CONFIGURATION_INVALID"))?;
    let object_store = ArtifactStoreAdapter::from_s3(
        S3Config::from_env()
            .map_err(|_| StartupError::Dependency("ARTIFACT_CONFIGURATION_INVALID"))?,
    )
    .map_err(|_| StartupError::Dependency("ARTIFACT_STORE_UNAVAILABLE"))?;
    let scanner = ClamAvScanner::new(required("MASONWING_SCANNER_ADDRESS")?)
        .map_err(|_| StartupError::Dependency("SCANNER_CONFIGURATION_INVALID"))?;
    let key_bytes = URL_SAFE_NO_PAD
        .decode(required("MASONWING_INVITE_KEY")?)
        .map_err(|_| StartupError::Dependency("INVITE_KEY_INVALID"))?;
    let invite_key = InviteTokenKey::from_bytes(key_bytes)
        .map_err(|_| StartupError::Dependency("INVITE_KEY_INVALID"))?;
    let spool = PathBuf::from(required("MASONWING_SPOOL_DIRECTORY")?);
    let metadata = tokio::fs::metadata(&spool)
        .await
        .map_err(|_| StartupError::Dependency("SPOOL_DIRECTORY_UNAVAILABLE"))?;
    if !metadata.is_dir() {
        return Err(StartupError::Dependency("SPOOL_DIRECTORY_UNAVAILABLE"));
    }
    let plugin_runtime = crate::plugin_runtime::HostPluginRuntime::new(local)
        .map_err(|_| StartupError::Dependency("PLUGIN_RUNTIME_UNAVAILABLE"))?;
    let native_packages = plugin_runtime.native_packages();
    let store = PostgresStore::from_pool(pool, Arc::new(object_store))
        .await
        .map_err(|_| StartupError::Dependency("DATA_ADAPTER_UNAVAILABLE"))?
        .with_upload_services(Arc::new(scanner), spool)
        .with_invite_key(invite_key.clone())
        .with_native_packages(native_packages)
        .with_plugin_runtime(Arc::new(plugin_runtime));
    // Force contract initialization at startup, before accepting connections.
    SchemaCatalog::shared().map_err(|_| StartupError::Dependency("CONTRACT_CATALOG_INVALID"))?;
    let _ = operations();
    let session_layer = sessions.session_layer(if local {
        SessionCookieProfile::LoopbackDev
    } else {
        SessionCookieProfile::Secure
    });
    let state = RuntimeState {
        store: Arc::new(store),
        auth,
        invite_key,
        environment,
    };
    Ok(router(state)
        .layer(session_layer)
        .layer(middleware::from_fn(response_headers)))
}

fn required(name: &'static str) -> Result<String, StartupError> {
    env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or(StartupError::Dependency(name))
}

pub fn router(state: RuntimeState) -> Router {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(readiness))
        .route("/dev/status", get(status))
        .route(
            "/v1/tenants/{tenant_id}/commands/{operation}",
            post(command),
        )
        .route(
            "/v1/tenants/{tenant_id}/resources/{resource_type}",
            get(collection),
        )
        .route(
            "/v1/tenants/{tenant_id}/resources/{resource_type}/{resource_id}",
            get(projection),
        )
        .route(
            "/v1/tenants/{tenant_id}/views/{resource_type}",
            get(extension_collection),
        )
        .route(
            "/v1/tenants/{tenant_id}/views/{resource_type}/{resource_id}",
            get(extension_projection),
        )
        .route(
            "/v1/tenants/{tenant_id}/artifacts/{artifact_id}",
            get(artifact_metadata),
        )
        .route(
            "/v1/tenants/{tenant_id}/artifacts/{artifact_id}/content",
            get(artifact_content),
        )
        .route("/v1/tenants/{tenant_id}/uploads/{upload_id}", put(upload))
        .route("/v1/tenants/{tenant_id}/runs", get(runs))
        .route("/v1/tenants/{tenant_id}/runs/{run_id}", get(run))
        .route(
            "/v1/tenants/{tenant_id}/invites/{invite_id}/secret",
            post(invite_secret),
        )
        .fallback(not_found)
        .with_state(state.clone())
        .merge(auth::router(state.auth))
}

async fn live() -> Json<Value> {
    Json(json!({"status":"live","service":"masonwing-host-api"}))
}

async fn readiness() -> Response {
    // Partial local command execution is not qualification for the complete
    // durable workflow/effect/production dependency set.
    (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"ready":false,"reason":"QUALIFICATION_INCOMPLETE","external_mutations":false,"live_budget_microunits":0}))).into_response()
}

async fn status(State(state): State<RuntimeState>) -> Response {
    if state.environment != Environment::Local {
        return not_found().await;
    }
    Json(json!({"service":"masonwing-host-api","environment":"LOCAL","readiness":{"ready":false,"status":"QUALIFICATION_INCOMPLETE"},"external_mutations_enabled":false,"live_budget_microunits":0,"operation_catalog_count":operations().len(),"data_store":"POSTGRESQL","artifact_store":"S3","identity":"OIDC","authorization":"CEDAR"})).into_response()
}

#[derive(Deserialize)]
struct CommandPath {
    tenant_id: String,
    operation: String,
}

async fn command(
    State(state): State<RuntimeState>,
    path: Result<Path<CommandPath>, PathRejection>,
    session: Session,
    request: Request,
) -> Response {
    command_inner(state, path, session, request)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

async fn command_inner(
    state: RuntimeState,
    path: Result<Path<CommandPath>, PathRejection>,
    session: Session,
    request: Request,
) -> Result<Response, ApiError> {
    let Path(path) = path.map_err(|_| CommandFailure::not_found())?;
    let step_up = *operations()
        .get(&path.operation)
        .ok_or_else(CommandFailure::not_found)?;
    let authenticated = state.auth.sessions.peek_authenticated(&session).await?;
    state.auth.csrf.verify(request.headers(), &authenticated)?;
    reject_tenant_override(request.headers())?;
    let tenant = uuid(&path.tenant_id)?;
    let content_type = single_header(request.headers(), header::CONTENT_TYPE.as_str())?;
    if !content_type
        .split(';')
        .next()
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
    {
        return Err(CommandFailure::invalid("CONTENT_TYPE_INVALID").into());
    }
    let idempotency_key =
        ResourceId::new(single_header(request.headers(), "idempotency-key")?.to_owned())
            .map_err(|_| CommandFailure::invalid("IDEMPOTENCY_KEY_INVALID"))?;
    let (actor, bytes) = body_after_admission(request, async {
        if path.operation == "membership.accept" {
            // The authenticated invite holder deliberately has no membership
            // yet. Its tenant binding is proved by the signed, single-use invite
            // inside the bootstrap transaction after the bounded body read.
            // The route tenant still has to exist and be ACTIVE: absent or
            // foreign tenants are masked as 404 before the body is parsed.
            state
                .auth
                .sessions
                .tenant_admission(tenant)
                .await
                .map_err(mask_tenant)?;
            Ok(None)
        } else {
            let actor = state
                .auth
                .gate
                .verify_for_tenant(&session, tenant)
                .await
                .map_err(mask_tenant)?;
            if step_up
                && actor
                    .step_up_expires_at
                    .is_none_or(|expires| expires <= chrono::Utc::now())
            {
                return Err(AuthError::StepUpRequired.into());
            }
            Ok(Some(command_actor(actor)?))
        }
    })
    .await?;
    let input = SchemaCatalog::shared()
        .map_err(|_| CommandFailure::unavailable("CONTRACT_CATALOG_INVALID"))?
        .command(&path.operation, &bytes)
        .map_err(|error| CommandFailure::invalid(error.code))?;
    let fingerprint = fingerprint(&path.operation, &input)
        .map_err(|_| CommandFailure::invalid("FINGERPRINT_INVALID"))?;
    let receipt = if path.operation == "membership.accept" {
        state.auth.sessions.touch_authenticated(&session).await?;
        state
            .store
            .execute_membership_accept_bootstrap(
                MembershipAcceptBootstrapCommand {
                    expected_tenant_id: tenant,
                    identity: VerifiedInviteAcceptIdentity {
                        principal_id: authenticated.principal_id,
                        issuer: authenticated.issuer,
                        subject: authenticated.subject,
                    },
                    idempotency_key,
                    fingerprint,
                    input,
                },
                &state.invite_key,
            )
            .await?
    } else {
        // The store resolves the actual target row and evaluates the action
        // using its current host-built facts, in the same epoch-locked transaction.
        state
            .store
            .execute(AuthorizedCommand {
                actor: actor
                    .ok_or_else(|| CommandFailure::unavailable("COMMAND_ADMISSION_INVALID"))?,
                operation: path.operation,
                idempotency_key,
                fingerprint,
                input,
            })
            .await?
    };
    let status = if receipt.state == ReceiptState::Accepted {
        StatusCode::ACCEPTED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(receipt)).into_response())
}

async fn body_after_admission(
    request: Request,
    admission: impl std::future::Future<Output = Result<Option<CommandActor>, ApiError>>,
) -> Result<(Option<CommandActor>, Vec<u8>), ApiError> {
    let actor = admission.await?;
    let bytes = tokio::time::timeout(
        Duration::from_secs(10),
        to_bytes(request.into_body(), MAX_COMMAND_BYTES),
    )
    .await
    .map_err(|_| CommandFailure::invalid("REQUEST_BODY_TIMEOUT"))?
    .map_err(|_| CommandFailure::invalid("PAYLOAD_TOO_LARGE"))?;
    Ok((actor, bytes.to_vec()))
}

#[derive(Deserialize)]
struct CollectionPath {
    tenant_id: String,
    resource_type: String,
}
#[derive(Deserialize)]
struct ProjectionPath {
    tenant_id: String,
    resource_type: String,
    resource_id: String,
}
#[derive(Deserialize)]
struct ArtifactPath {
    tenant_id: String,
    artifact_id: String,
}
#[derive(Deserialize)]
struct UploadPath {
    tenant_id: String,
    upload_id: String,
}
#[derive(Deserialize)]
struct TenantPath {
    tenant_id: String,
}
#[derive(Deserialize)]
struct RunPath {
    tenant_id: String,
    run_id: String,
}
#[derive(Deserialize)]
struct InvitePath {
    tenant_id: String,
    invite_id: String,
}

async fn collection(
    State(state): State<RuntimeState>,
    path: Result<Path<CollectionPath>, PathRejection>,
    query: Result<Query<PageQuery>, QueryRejection>,
    session: Session,
) -> Response {
    read_collection(state, path, query, session, false)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}
async fn extension_collection(
    State(state): State<RuntimeState>,
    path: Result<Path<CollectionPath>, PathRejection>,
    query: Result<Query<PageQuery>, QueryRejection>,
    session: Session,
) -> Response {
    read_collection(state, path, query, session, true)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}
async fn read_collection(
    state: RuntimeState,
    path: Result<Path<CollectionPath>, PathRejection>,
    query: Result<Query<PageQuery>, QueryRejection>,
    session: Session,
    extension: bool,
) -> Result<Response, ApiError> {
    let Path(path) = path.map_err(|_| CommandFailure::not_found())?;
    if !extension && !BASELINE_TYPES.contains(&path.resource_type.as_str()) {
        return Err(CommandFailure::not_found().into());
    }
    let actor = read_actor(&state, &session, &path.tenant_id).await?;
    let Query(query) = query.map_err(|_| CommandFailure::invalid("QUERY_INVALID"))?;
    let result = state
        .store
        .collection(&actor, &path.resource_type, &query)
        .await?;
    Ok(Json(result).into_response())
}
async fn projection(
    State(state): State<RuntimeState>,
    path: Result<Path<ProjectionPath>, PathRejection>,
    session: Session,
) -> Response {
    read_projection(state, path, session, false)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}
async fn extension_projection(
    State(state): State<RuntimeState>,
    path: Result<Path<ProjectionPath>, PathRejection>,
    session: Session,
) -> Response {
    read_projection(state, path, session, true)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}
async fn read_projection(
    state: RuntimeState,
    path: Result<Path<ProjectionPath>, PathRejection>,
    session: Session,
    extension: bool,
) -> Result<Response, ApiError> {
    let Path(path) = path.map_err(|_| CommandFailure::not_found())?;
    if !extension && !BASELINE_TYPES.contains(&path.resource_type.as_str()) {
        return Err(CommandFailure::not_found().into());
    }
    let actor = read_actor(&state, &session, &path.tenant_id).await?;
    Ok(Json(
        state
            .store
            .projection(&actor, &path.resource_type, &path.resource_id)
            .await?,
    )
    .into_response())
}
async fn artifact_metadata(
    State(state): State<RuntimeState>,
    path: Result<Path<ArtifactPath>, PathRejection>,
    session: Session,
) -> Response {
    async {
        let Path(path) = path.map_err(|_| CommandFailure::not_found())?;
        let actor = read_actor(&state, &session, &path.tenant_id).await?;
        Ok::<_, ApiError>(
            Json(
                state
                    .store
                    .artifact_metadata(&actor, &path.artifact_id)
                    .await?
                    .reference,
            )
            .into_response(),
        )
    }
    .await
    .unwrap_or_else(IntoResponse::into_response)
}

async fn artifact_content(
    State(state): State<RuntimeState>,
    path: Result<Path<ArtifactPath>, PathRejection>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    async {
        let Path(path)=path.map_err(|_|CommandFailure::not_found())?;
        let actor=read_actor(&state,&session,&path.tenant_id).await?;
        if headers.contains_key(header::RANGE) {return Err(CommandFailure::invalid("RANGE_UNSUPPORTED").into());}
        let mut download=state.store.verified_artifact_download(&actor,&path.artifact_id).await?;
        let content_type=HeaderValue::from_str(&download.metadata.content_type).map_err(|_|CommandFailure::unavailable("ARTIFACT_METADATA_INVALID"))?;
        let size=download.metadata.size_bytes;
        let etag=HeaderValue::from_str(&format!("\"{}\"",download.metadata.reference.digest)).map_err(|_|CommandFailure::unavailable("ARTIFACT_METADATA_INVALID"))?;
        let body=async_stream::try_stream! {
            let _transfer_permit=download.permit;
            let mut checked=tokio::time::Instant::now();
            loop {
                if checked.elapsed()>=Duration::from_secs(1) {
                    state.auth.sessions.peek_authenticated(&session).await.map_err(|_|io::Error::other("AUTHORITY_CHANGED"))?;
                    state.store.artifact_metadata(&actor,&path.artifact_id).await.map_err(|_|io::Error::other("AUTHORITY_CHANGED"))?;
                    checked=tokio::time::Instant::now();
                }
                let mut buffer=vec![0;64*1024];
                let count=download.file.read(&mut buffer).await.map_err(|_|io::Error::other("ARTIFACT_READ_FAILED"))?;
                if count==0 {break;}
                buffer.truncate(count);
                yield Bytes::from(buffer);
            }
        };
        let body:std::pin::Pin<Box<dyn futures_util::Stream<Item=Result<Bytes,io::Error>>+Send>>=Box::pin(body);
        let mut response=Body::from_stream(body).into_response();
        response.headers_mut().insert(header::CONTENT_TYPE,content_type);
        response.headers_mut().insert(header::CONTENT_LENGTH,HeaderValue::from_str(&size.to_string()).map_err(|_|CommandFailure::unavailable("ARTIFACT_METADATA_INVALID"))?);
        response.headers_mut().insert(header::CONTENT_DISPOSITION,HeaderValue::from_static("attachment"));
        response.headers_mut().insert(header::ETAG,etag);
        Ok::<_,ApiError>(response)
    }.await.unwrap_or_else(IntoResponse::into_response)
}

async fn upload(
    State(state): State<RuntimeState>,
    path: Result<Path<UploadPath>, PathRejection>,
    session: Session,
    request: Request,
) -> Response {
    async {
        let Path(path) = path.map_err(|_| CommandFailure::not_found())?;
        let auth = state.auth.sessions.peek_authenticated(&session).await?;
        state.auth.csrf.verify(request.headers(), &auth)?;
        reject_tenant_override(request.headers())?;
        let tenant = uuid(&path.tenant_id)?;
        let permission = state
            .auth
            .gate
            .authorize_current(
                &session,
                tenant,
                AuthorizationInput {
                    action: "artifact.begin".into(),
                    resource: TrustedResource::new(
                        "Tenant",
                        tenant.to_string(),
                        tenant.to_string(),
                    ),
                    requires_step_up: false,
                },
            )
            .await
            .map_err(mask_tenant)?;
        let length = optional_length(request.headers())?;
        if length.is_some_and(|size| size > MAX_ARTIFACT_BYTES) {
            return Err(CommandFailure::invalid("PAYLOAD_TOO_LARGE").into());
        }
        let stream = request
            .into_body()
            .into_data_stream()
            .map_err(|_| io::Error::other("UPLOAD_BODY_INVALID"));
        let mut reader = StreamReader::new(stream);
        state
            .store
            .put_upload(
                &command_actor(permission.actor)?,
                &path.upload_id,
                &mut reader,
                length,
            )
            .await?;
        Ok::<_, ApiError>(StatusCode::NO_CONTENT.into_response())
    }
    .await
    .unwrap_or_else(IntoResponse::into_response)
}

async fn runs(
    State(state): State<RuntimeState>,
    path: Result<Path<TenantPath>, PathRejection>,
    query: Result<Query<PageQuery>, QueryRejection>,
    session: Session,
) -> Response {
    async {
        let Path(path) = path.map_err(|_| CommandFailure::not_found())?;
        let actor = read_actor(&state, &session, &path.tenant_id).await?;
        let Query(query) = query.map_err(|_| CommandFailure::invalid("QUERY_INVALID"))?;
        Ok::<_, ApiError>(
            Json(state.store.collection(&actor, "Run", &query).await?).into_response(),
        )
    }
    .await
    .unwrap_or_else(IntoResponse::into_response)
}
async fn run(
    State(state): State<RuntimeState>,
    path: Result<Path<RunPath>, PathRejection>,
    session: Session,
) -> Response {
    async {
        let Path(path) = path.map_err(|_| CommandFailure::not_found())?;
        let actor = read_actor(&state, &session, &path.tenant_id).await?;
        let projection = state.store.projection(&actor, "Run", &path.run_id).await?;
        let (_, bytes) = state
            .store
            .artifact_content(
                &actor,
                projection.artifact_ref.artifact_id.as_str(),
                MAX_COMMAND_BYTES,
            )
            .await?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| CommandFailure::unavailable("RUN_INTEGRITY"))?;
        SchemaCatalog::shared()
            .map_err(|_| CommandFailure::unavailable("CONTRACT_CATALOG_INVALID"))?
            .validate("Run", &value)
            .map_err(|_| CommandFailure::unavailable("RUN_INTEGRITY"))?;
        Ok::<_, ApiError>(Json(value).into_response())
    }
    .await
    .unwrap_or_else(IntoResponse::into_response)
}

async fn invite_secret(
    State(state): State<RuntimeState>,
    path: Result<Path<InvitePath>, PathRejection>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    async {
        let Path(path) = path.map_err(|_| CommandFailure::not_found())?;
        let authenticated = state.auth.sessions.peek_authenticated(&session).await?;
        state.auth.csrf.verify(&headers, &authenticated)?;
        reject_tenant_override(&headers)?;
        let tenant = uuid(&path.tenant_id)?;
        let permission = state
            .auth
            .gate
            .authorize_current(
                &session,
                tenant,
                AuthorizationInput {
                    action: "membership.invite".into(),
                    resource: TrustedResource::new(
                        "Tenant",
                        tenant.to_string(),
                        tenant.to_string(),
                    ),
                    requires_step_up: true,
                },
            )
            .await
            .map_err(mask_tenant)?;
        let secret = state
            .store
            .retrieve_membership_invite_secret(
                &command_actor(permission.actor)?,
                uuid(&path.invite_id)?,
                &state.invite_key,
            )
            .await?;
        Ok::<_, ApiError>(Json(json!({"invite_token":secret.expose()})).into_response())
    }
    .await
    .unwrap_or_else(IntoResponse::into_response)
}

async fn read_actor(
    state: &RuntimeState,
    session: &Session,
    tenant: &str,
) -> Result<CommandActor, ApiError> {
    let actor = state
        .auth
        .gate
        .verify_for_tenant(session, uuid(tenant)?)
        .await
        .map_err(mask_tenant)?;
    command_actor(actor).map_err(Into::into)
}

fn command_actor(actor: VerifiedActor) -> Result<CommandActor, CommandFailure> {
    Ok(CommandActor {
        tenant_id: TenantId::new(actor.tenant_id.to_string())
            .map_err(|_| CommandFailure::not_found())?,
        principal_id: PrincipalId::new(actor.principal_id.to_string())
            .map_err(|_| CommandFailure::not_found())?,
        issuer: actor.issuer,
        membership_epoch: actor.membership_epoch,
        permission_epoch: actor.permission_epoch,
        policy_version: actor.policy_version,
        policy_epoch: actor.policy_epoch,
    })
}
fn uuid(value: &str) -> Result<Uuid, CommandFailure> {
    Uuid::parse_str(value).map_err(|_| CommandFailure::not_found())
}
fn mask_tenant(error: AuthError) -> AuthError {
    if matches!(
        error,
        AuthError::MembershipDenied | AuthError::TenantContextMismatch
    ) {
        AuthError::NotFound
    } else {
        error
    }
}
fn reject_tenant_override(headers: &HeaderMap) -> Result<(), CommandFailure> {
    if headers.contains_key("x-tenant-id") || headers.contains_key("x-masonwing-tenant") {
        return Err(CommandFailure::invalid("TENANT_OVERRIDE_REJECTED"));
    }
    Ok(())
}
fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, CommandFailure> {
    let mut values = headers.get_all(name).iter();
    let value = values
        .next()
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| CommandFailure::invalid("REQUIRED_HEADER_MISSING"))?;
    if values.next().is_some() || value.is_empty() {
        return Err(CommandFailure::invalid("AMBIGUOUS_HEADER"));
    }
    Ok(value)
}
fn optional_length(headers: &HeaderMap) -> Result<Option<u64>, CommandFailure> {
    if !headers.contains_key(header::CONTENT_LENGTH) {
        return Ok(None);
    }
    single_header(headers, header::CONTENT_LENGTH.as_str())?
        .parse::<u64>()
        .map(Some)
        .map_err(|_| CommandFailure::invalid("CONTENT_LENGTH_INVALID"))
}

pub enum ApiError {
    Auth(AuthError),
    Command(CommandFailure),
}
impl From<AuthError> for ApiError {
    fn from(value: AuthError) -> Self {
        Self::Auth(value)
    }
}
impl From<CommandFailure> for ApiError {
    fn from(value: CommandFailure) -> Self {
        Self::Command(value)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let Self::Command(error) = self else {
            let Self::Auth(error) = self else {
                unreachable!()
            };
            return error.into_response();
        };
        let (status, recovery) = match error.kind {
            FailureKind::Invalid => (
                if error.code == "PAYLOAD_TOO_LARGE" {
                    StatusCode::PAYLOAD_TOO_LARGE
                } else {
                    StatusCode::BAD_REQUEST
                },
                RecoveryAction::None,
            ),
            FailureKind::NotFound => (StatusCode::NOT_FOUND, RecoveryAction::None),
            FailureKind::Denied => (StatusCode::FORBIDDEN, RecoveryAction::RequestPermission),
            FailureKind::Conflict => (StatusCode::CONFLICT, RecoveryAction::ReviewConflict),
            FailureKind::Precondition => (StatusCode::UNPROCESSABLE_ENTITY, RecoveryAction::None),
            FailureKind::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                RecoveryAction::ContactOperator,
            ),
            FailureKind::Exhausted => (StatusCode::TOO_MANY_REQUESTS, RecoveryAction::None),
            FailureKind::Expired => (StatusCode::GONE, RecoveryAction::RefreshSnapshot),
        };
        (
            status,
            Json(ErrorResponse {
                code: error.code.into(),
                message: "The request could not be completed.".into(),
                correlation_id: next_correlation_id(),
                effect_state: ErrorEffectState::NotSent,
                retryable: false,
                recovery_action: recovery,
                details: Vec::<ErrorDetail>::new(),
            }),
        )
            .into_response()
    }
}

async fn not_found() -> Response {
    ApiError::Command(CommandFailure::not_found()).into_response()
}

async fn response_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'; sandbox"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immutable_operation_catalog_is_complete_and_step_up_is_not_defaulted_off() {
        assert_eq!(
            operations().len(),
            masonwing_contracts::PLATFORM_OPERATIONS.len()
        );
        assert_eq!(operations().get("plugin.install"), Some(&true));
        assert_eq!(operations().get("artifact.begin"), Some(&false));
        assert!(operations().get("sql.eval").is_none());
    }
    #[test]
    fn ambiguous_auth_sensitive_headers_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.append("idempotency-key", HeaderValue::from_static("a"));
        headers.append("idempotency-key", HeaderValue::from_static("b"));
        assert_eq!(
            single_header(&headers, "idempotency-key").unwrap_err().code,
            "AMBIGUOUS_HEADER"
        );
        headers.insert("x-tenant-id", HeaderValue::from_static("other"));
        assert!(reject_tenant_override(&headers).is_err());
    }

    #[tokio::test]
    async fn rejected_tenant_or_step_up_admission_never_polls_untrusted_body() {
        use std::{
            sync::atomic::{AtomicUsize, Ordering},
            task::Poll,
        };
        for failure in [
            CommandFailure::not_found(),
            CommandFailure::denied("STEP_UP_REQUIRED"),
        ] {
            let polls = Arc::new(AtomicUsize::new(0));
            let observed = polls.clone();
            let stream = futures_util::stream::poll_fn(move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
                Poll::Ready(Some(Ok::<_, std::io::Error>(Bytes::from_static(
                    b"invalid secret-bearing JSON",
                ))))
            });
            let result = body_after_admission(Request::new(Body::from_stream(stream)), async {
                Err(failure.into())
            })
            .await;
            assert!(result.is_err());
            assert_eq!(polls.load(Ordering::SeqCst), 0);
        }
    }
}
