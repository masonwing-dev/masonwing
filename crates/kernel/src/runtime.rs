//! Application ports and failures; no database, HTTP client or secret handles.

use chrono::{DateTime, Utc};
use masonwing_contracts::{
    Action, Digest, GrantId, PrincipalId, ResourceId, TenantId,
    wire::{
        ArtifactRef, CommandReceipt, HandlerContract, PluginManifest, ReadCollection,
        ResourceProjection,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;
use thiserror::Error;
use tokio::io::AsyncRead;

/// Constructed by the verified identity/authorization boundary, then rechecked
/// against current membership and policy epochs inside the mutation transaction.
#[derive(Clone, Debug)]
pub struct CommandActor {
    pub tenant_id: TenantId,
    pub principal_id: PrincipalId,
    pub issuer: String,
    pub membership_epoch: i64,
    pub permission_epoch: i64,
    pub policy_version: String,
    pub policy_epoch: i64,
}

#[derive(Clone, Debug)]
pub struct AuthorizedCommand {
    pub actor: CommandActor,
    pub operation: String,
    pub idempotency_key: ResourceId,
    pub fingerprint: Digest,
    pub input: Value,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{code}")]
pub struct CommandFailure {
    pub code: &'static str,
    pub kind: FailureKind,
    integrity_reference: Option<Box<ArtifactRef>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    Invalid,
    NotFound,
    Denied,
    Conflict,
    Precondition,
    Unavailable,
    Exhausted,
    Expired,
}

impl CommandFailure {
    pub const fn new(code: &'static str, kind: FailureKind) -> Self {
        Self {
            code,
            kind,
            integrity_reference: None,
        }
    }
    pub const fn invalid(code: &'static str) -> Self {
        Self::new(code, FailureKind::Invalid)
    }
    pub const fn not_found() -> Self {
        Self::new("RESOURCE_NOT_FOUND", FailureKind::NotFound)
    }
    pub const fn denied(code: &'static str) -> Self {
        Self::new(code, FailureKind::Denied)
    }
    pub const fn conflict(code: &'static str) -> Self {
        Self::new(code, FailureKind::Conflict)
    }
    pub const fn precondition(code: &'static str) -> Self {
        Self::new(code, FailureKind::Precondition)
    }
    pub const fn unavailable(code: &'static str) -> Self {
        Self::new(code, FailureKind::Unavailable)
    }
    pub fn artifact_integrity(reference: ArtifactRef) -> Self {
        Self {
            code: "ARTIFACT_INTEGRITY",
            kind: FailureKind::Precondition,
            integrity_reference: Some(Box::new(reference)),
        }
    }
    /// Private containment context, never serialized into a browser error body.
    /// The store must reread and prove corruption before quarantining anything.
    pub fn integrity_reference(&self) -> Option<&ArtifactRef> {
        self.integrity_reference.as_deref()
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageQuery {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub q: Option<String>,
}

impl PageQuery {
    pub fn checked_limit(&self) -> Result<u32, CommandFailure> {
        match self.limit.unwrap_or(50) {
            limit @ 1..=200 => Ok(limit),
            _ => Err(CommandFailure::invalid("PAGE_LIMIT_INVALID")),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct StoredArtifact {
    pub reference: ArtifactRef,
    pub content_type: String,
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
    pub state: String,
}

#[async_trait::async_trait]
pub trait CommandRepository: Send + Sync {
    async fn execute(&self, command: AuthorizedCommand) -> Result<CommandReceipt, CommandFailure>;
    async fn collection(
        &self,
        actor: &CommandActor,
        resource_type: &str,
        page: &PageQuery,
    ) -> Result<ReadCollection, CommandFailure>;
    async fn projection(
        &self,
        actor: &CommandActor,
        resource_type: &str,
        id: &str,
    ) -> Result<ResourceProjection, CommandFailure>;
}

/// Trusted application request created only after the command transaction has
/// checked the current tenant, publisher, plugin, grant, rights and schemas.
/// This synchronous-result slice has no external-effect or secret primitive.
pub struct PurePluginInvocation {
    pub invocation_id: ResourceId,
    pub tenant_id: TenantId,
    pub manifest: PluginManifest,
    pub handler: HandlerContract,
    pub grant_id: GrantId,
    pub granted_capabilities: Vec<Action>,
    pub artifact_bytes: Vec<u8>,
    pub input: Vec<u8>,
}

pub struct PurePluginOutput {
    /// Serialized application instance, checked against the signed output schema
    /// again before the host persists any output or successful receipt.
    pub bytes: Vec<u8>,
}

#[async_trait::async_trait]
pub trait PurePluginRuntime: Send + Sync {
    async fn invoke(
        &self,
        request: PurePluginInvocation,
    ) -> Result<PurePluginOutput, CommandFailure>;
}

/// Artifact object storage is replaceable without exposing raw object-store or
/// database handles to domain code. Keys originate at the host, never plugins.
#[async_trait::async_trait]
pub trait ArtifactObjects: Send + Sync {
    async fn put_immutable(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), CommandFailure>;
    async fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, CommandFailure>;
    async fn put_reader(
        &self,
        _key: &str,
        _reader: &mut (dyn AsyncRead + Send + Unpin),
        _size_bytes: u64,
        _content_type: &str,
    ) -> Result<(), CommandFailure> {
        Err(CommandFailure::unavailable(
            "ARTIFACT_STREAMING_UNAVAILABLE",
        ))
    }
    async fn open_reader(
        &self,
        _key: &str,
        _max_bytes: u64,
    ) -> Result<ArtifactStream, CommandFailure> {
        Err(CommandFailure::unavailable(
            "ARTIFACT_STREAMING_UNAVAILABLE",
        ))
    }
}

pub struct ArtifactStream {
    pub size_bytes: u64,
    pub reader: Pin<Box<dyn AsyncRead + Send>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanVerdict {
    Clean,
    Quarantined { signature: String },
}

/// A transport error or unsupported scanner cannot become a CLEAN verdict.
#[async_trait::async_trait]
pub trait ArtifactScanner: Send + Sync {
    async fn scan(&self, bytes: &[u8]) -> Result<ScanVerdict, CommandFailure>;
    async fn scan_reader(
        &self,
        _reader: &mut (dyn AsyncRead + Send + Unpin),
        _size_bytes: u64,
    ) -> Result<ScanVerdict, CommandFailure> {
        Err(CommandFailure::unavailable("ARTIFACT_SCANNER_UNAVAILABLE"))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailableArtifactScanner;

#[async_trait::async_trait]
impl ArtifactScanner for UnavailableArtifactScanner {
    async fn scan(&self, _bytes: &[u8]) -> Result<ScanVerdict, CommandFailure> {
        Err(CommandFailure::unavailable("ARTIFACT_SCANNER_UNAVAILABLE"))
    }
}

pub fn require_expected_version(actual: i32, expected: i32) -> Result<i32, CommandFailure> {
    if actual != expected {
        return Err(CommandFailure::conflict("STALE_VERSION"));
    }
    actual
        .checked_add(1)
        .ok_or_else(|| CommandFailure::precondition("VERSION_EXHAUSTED"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonwing_contracts::ArtifactId;
    #[test]
    fn optimistic_version_does_not_wrap_or_silently_rebase() {
        assert_eq!(require_expected_version(2, 2).unwrap(), 3);
        assert_eq!(
            require_expected_version(2, 1).unwrap_err().code,
            "STALE_VERSION"
        );
        assert_eq!(
            require_expected_version(i32::MAX, i32::MAX)
                .unwrap_err()
                .code,
            "VERSION_EXHAUSTED"
        );
    }

    #[test]
    fn version_zero_to_one_is_valid_and_monotonic() {
        assert_eq!(require_expected_version(0, 0).unwrap(), 1);
        assert_eq!(
            require_expected_version(1, 0).unwrap_err().code,
            "STALE_VERSION"
        );
    }

    #[test]
    fn command_failure_construction_preserves_kind_and_code() {
        let cf = CommandFailure::denied("TEST_CODE");
        assert_eq!(cf.code, "TEST_CODE");
        assert_eq!(cf.kind, FailureKind::Denied);
        assert!(cf.integrity_reference().is_none());
    }

    #[test]
    fn command_failure_not_found_has_correct_kind() {
        let cf = CommandFailure::not_found();
        assert_eq!(cf.code, "RESOURCE_NOT_FOUND");
        assert_eq!(cf.kind, FailureKind::NotFound);
    }

    #[test]
    fn command_failure_artifact_integrity_carries_reference() {
        let artifact_ref = ArtifactRef {
            artifact_id: ArtifactId::new("artifact_123").unwrap(),
            tenant_id: TenantId::new("tenant_1").unwrap(),
            digest: Digest::sha256(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            )
            .unwrap(),
            schema_version: "1.0.0".into(),
            classification: masonwing_contracts::wire::Classification::Confidential,
        };
        let cf = CommandFailure::artifact_integrity(artifact_ref.clone());
        assert_eq!(cf.code, "ARTIFACT_INTEGRITY");
        assert_eq!(cf.kind, FailureKind::Precondition);
        assert!(cf.integrity_reference().is_some());
    }

    #[test]
    fn command_actor_can_be_constructed_with_all_fields() {
        let actor = CommandActor {
            tenant_id: TenantId::new("tenant_1").unwrap(),
            principal_id: PrincipalId::new("principal_1").unwrap(),
            issuer: "https://issuer.example".into(),
            membership_epoch: 1,
            permission_epoch: 1,
            policy_version: "1.0.0".into(),
            policy_epoch: 1,
        };
        assert_eq!(actor.tenant_id.as_str(), "tenant_1");
        assert_eq!(actor.principal_id.as_str(), "principal_1");
        assert_eq!(actor.membership_epoch, 1);
    }

    #[test]
    fn authorized_command_binds_actor_operation_and_idempotency() {
        let actor = CommandActor {
            tenant_id: TenantId::new("tenant_1").unwrap(),
            principal_id: PrincipalId::new("principal_1").unwrap(),
            issuer: "https://issuer.example".into(),
            membership_epoch: 1,
            permission_epoch: 1,
            policy_version: "1.0.0".into(),
            policy_epoch: 1,
        };
        let idempotency = ResourceId::new("idem_123").unwrap();
        let fingerprint =
            Digest::sha256("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap();
        let cmd = AuthorizedCommand {
            actor: actor.clone(),
            operation: "plugin.install".into(),
            idempotency_key: idempotency.clone(),
            fingerprint,
            input: serde_json::json!({}),
        };
        assert_eq!(cmd.operation, "plugin.install");
        assert_eq!(cmd.idempotency_key, idempotency);
    }

    #[test]
    fn page_query_checked_limit_accepts_valid_range() {
        let query = PageQuery {
            cursor: None,
            limit: Some(100),
            q: None,
        };
        assert_eq!(query.checked_limit().unwrap(), 100);
    }

    #[test]
    fn page_query_checked_limit_rejects_out_of_bounds() {
        let query = PageQuery {
            cursor: None,
            limit: Some(300),
            q: None,
        };
        assert_eq!(
            query.checked_limit().unwrap_err().code,
            "PAGE_LIMIT_INVALID"
        );
    }

    #[test]
    fn page_query_default_limit_is_50() {
        let query = PageQuery {
            cursor: None,
            limit: None,
            q: None,
        };
        assert_eq!(query.checked_limit().unwrap(), 50);
    }

    #[test]
    fn stored_artifact_serializes_all_fields() {
        let artifact_ref = ArtifactRef {
            artifact_id: ArtifactId::new("artifact_123").unwrap(),
            tenant_id: TenantId::new("tenant_1").unwrap(),
            digest: Digest::sha256(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            )
            .unwrap(),
            schema_version: "1.0.0".into(),
            classification: masonwing_contracts::wire::Classification::Confidential,
        };
        let stored = StoredArtifact {
            reference: artifact_ref,
            content_type: "application/json".into(),
            size_bytes: 1024,
            created_at: chrono::Utc::now(),
            state: "ACTIVE".into(),
        };
        let json = serde_json::to_string(&stored).unwrap();
        assert!(json.contains("artifact_123"));
        assert!(json.contains("application/json"));
        assert!(json.contains("ACTIVE"));
    }
}
