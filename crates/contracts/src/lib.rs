//! Public, domain-neutral Masonwing value objects.
//!
//! This crate is intentionally implementation-free. Domain plugins may depend on
//! these values through `masonwing-sdk`; they must not reach into kernel or
//! infrastructure adapter internals.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod wire;

pub const CONTRACT_VERSION: &str = "1.0.0";

fn validate_opaque_identifier(kind: &'static str, value: &str) -> Result<(), ValueError> {
    if value.is_empty() {
        return Err(ValueError::EmptyIdentifier(kind));
    }
    if value.len() > 128 {
        return Err(ValueError::InvalidOpaqueIdentifier(kind));
    }

    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return Err(ValueError::EmptyIdentifier(kind));
    };
    if !first.is_ascii_alphanumeric()
        || !bytes
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(ValueError::InvalidOpaqueIdentifier(kind));
    }
    Ok(())
}

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ValueError> {
                Self::try_from(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = ValueError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                validate_opaque_identifier(stringify!($name), &value)?;
                Ok(Self(value))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

string_id!(TenantId);
string_id!(PrincipalId);
string_id!(ResourceId);
string_id!(GrantId);
string_id!(PluginId);
string_id!(EffectId);
string_id!(ConnectionHandle);
string_id!(ArtifactId);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Action(String);

impl Action {
    pub fn new(value: impl Into<String>) -> Result<Self, ValueError> {
        Self::try_from(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Action {
    type Error = ValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_opaque_identifier("Action", &value)?;
        Ok(Self(value))
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Digest(String);

impl Digest {
    pub fn sha256(hex: impl Into<String>) -> Result<Self, ValueError> {
        let hex = hex.into();
        if hex.len() != 64
            || !hex
                .as_bytes()
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(ValueError::InvalidSha256Digest);
        }
        Ok(Self(format!("sha256:{hex}")))
    }

    pub fn parse(value: impl Into<String>) -> Result<Self, ValueError> {
        Self::try_from(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Digest {
    type Error = ValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(ValueError::InvalidSha256Digest);
        };
        Self::sha256(hex.to_owned())
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EpochMillis(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Environment {
    Local,
    Integration,
    StagingLive,
    Production,
}

impl Environment {
    pub fn parse(value: &str) -> Result<Self, ValueError> {
        match value.trim().to_ascii_uppercase().as_str() {
            "LOCAL" => Ok(Self::Local),
            "INTEGRATION" => Ok(Self::Integration),
            "STAGING_LIVE" => Ok(Self::StagingLive),
            "PRODUCTION" => Ok(Self::Production),
            _ => Err(ValueError::UnsupportedEnvironment(value.to_owned())),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImplementationStatus {
    NotImplemented,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdapterQualification {
    Qualified,
    Unqualified,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessStatus {
    pub writes: bool,
    pub critical_adapters_qualified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScaffoldStatus {
    pub service: String,
    pub environment: Environment,
    pub scaffold: bool,
    pub implementation_status: ImplementationStatus,
    pub external_mutation_enabled: bool,
    pub live_budget_microunits: u64,
    pub readiness: ReadinessStatus,
    pub contract_version: String,
}

impl ScaffoldStatus {
    pub fn local(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            environment: Environment::Local,
            scaffold: true,
            implementation_status: ImplementationStatus::NotImplemented,
            external_mutation_enabled: false,
            live_budget_microunits: 0,
            readiness: ReadinessStatus {
                writes: false,
                critical_adapters_qualified: false,
            },
            contract_version: CONTRACT_VERSION.to_owned(),
        }
    }
}

/// Closed platform command catalog from `MASONWING@1.0.1`.
pub const PLATFORM_OPERATIONS: &[&str] = &[
    "product.compose",
    "plugin.install",
    "plugin.enable",
    "plugin.disable",
    "plugin.upgrade",
    "plugin.revoke",
    "plugin.uninstall",
    "plugin.invoke",
    "identity.configure",
    "membership.invite",
    "membership.accept",
    "membership.change",
    "membership.revoke",
    "policy.evaluate",
    "policy.propose",
    "grant.create",
    "grant.revoke",
    "artifact.begin",
    "artifact.finalize",
    "deletion.request",
    "run.start",
    "run.cancel",
    "approval.decide",
    "effect.propose",
    "effect.dispatch",
    "effect.reconcile",
    "effect.compensate",
    "budget.configure",
    "provider.configure",
    "provider.compact",
    "connection.authorize",
    "connection.revoke",
    "schedule.create",
    "deadletter.replay",
    "ui.register",
    "export.create",
    "notification.read",
    "support.request",
    "kill-switch.set",
    "release.qualify",
    "catalog.submit",
    "catalog.review",
    "conformance.run",
];

pub fn is_platform_operation(operation: &str) -> bool {
    PLATFORM_OPERATIONS.contains(&operation)
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ValueError {
    #[error("{0} cannot be empty")]
    EmptyIdentifier(&'static str),
    #[error(
        "{0} must match OpaqueId: first ASCII alphanumeric, then ASCII alphanumeric or ._:-, max 128 bytes"
    )]
    InvalidOpaqueIdentifier(&'static str),
    #[error("digest must be sha256:<64 lowercase hex>")]
    InvalidSha256Digest,
    #[error("unsupported environment: {0}")]
    UnsupportedEnvironment(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_rejects_uppercase_and_wrong_length() {
        assert_eq!(
            Digest::parse(format!("sha256:{}", "A".repeat(64))).unwrap_err(),
            ValueError::InvalidSha256Digest
        );
        assert_eq!(
            Digest::parse(format!("sha256:{}", "a".repeat(63))).unwrap_err(),
            ValueError::InvalidSha256Digest
        );
    }

    #[test]
    fn operation_catalog_is_closed() {
        assert!(is_platform_operation("effect.dispatch"));
        assert_eq!(PLATFORM_OPERATIONS.len(), 43);
        assert!(!is_platform_operation("sql.eval"));
    }

    #[test]
    fn opaque_ids_and_actions_reject_malformed_json_values() {
        for invalid in ["", "_starts_wrong", "contains space", "contains/slash", "é"] {
            assert!(serde_json::from_str::<TenantId>(&format!("{invalid:?}")).is_err());
            assert!(serde_json::from_str::<Action>(&format!("{invalid:?}")).is_err());
        }
        let too_long = format!("a{}", "b".repeat(128));
        assert!(serde_json::from_str::<ResourceId>(&format!("{too_long:?}")).is_err());

        let valid: TenantId = serde_json::from_str(r#""tenant_a:1""#).unwrap();
        assert_eq!(valid.as_str(), "tenant_a:1");
    }

    #[test]
    fn digest_deserialization_cannot_bypass_parse_validation() {
        let valid = format!("sha256:{}", "a".repeat(64));
        let digest: Digest = serde_json::from_str(&format!("{valid:?}")).unwrap();
        assert_eq!(digest.as_str(), valid);

        for invalid in [
            format!("sha256:{}", "A".repeat(64)),
            format!("sha256:{}", "a".repeat(63)),
            "not-a-digest".to_owned(),
        ] {
            assert!(serde_json::from_str::<Digest>(&format!("{invalid:?}")).is_err());
        }
    }
}
