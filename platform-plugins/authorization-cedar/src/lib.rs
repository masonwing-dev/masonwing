//! Cedar authorization adapter used by the Masonwing host.
//!
//! The adapter intentionally accepts typed, host-created principals/resources
//! rather than a caller-provided Cedar entity bag. This keeps Cedar as the
//! policy evaluator while the host remains the authority for entity facts.

use std::{collections::BTreeMap, str::FromStr};

use cedar_policy::{
    Authorizer, Context, Decision, Entities, EntityId, EntityTypeName, EntityUid, PolicySet,
    Request,
};
use masonwing_contracts::AdapterQualification;
use masonwing_kernel::ports::{
    AdapterBoundary, AuthorizationPort, AuthorizationRequest, PortError,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct CedarPolicySnapshot {
    pub policy_version: String,
    pub policy_epoch: i64,
    policies: PolicySet,
}

impl CedarPolicySnapshot {
    pub fn parse(
        policy_version: impl Into<String>,
        policy_epoch: i64,
        cedar_source: &str,
    ) -> Result<Self, CedarAuthorizationError> {
        let policy_version = policy_version.into();
        if policy_version.is_empty() || policy_epoch <= 0 {
            return Err(CedarAuthorizationError::InvalidPolicyMetadata);
        }

        let policies = PolicySet::from_str(cedar_source)
            .map_err(|_| CedarAuthorizationError::InvalidPolicy)?;
        Ok(Self {
            policy_version,
            policy_epoch,
            policies,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedPrincipal {
    pub principal_id: String,
    pub tenant_id: String,
    /// Canonical compatibility role. Authorization policies should prefer
    /// `roles`, which contains the complete current membership role set.
    pub role: String,
    pub roles: Vec<String>,
    pub membership_epoch: i64,
    pub permission_epoch: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrustedResource {
    pub entity_type: String,
    pub resource_id: String,
    pub tenant_id: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, Value>,
}

impl TrustedResource {
    pub fn new(
        entity_type: impl Into<String>,
        resource_id: impl Into<String>,
        tenant_id: impl Into<String>,
    ) -> Self {
        Self {
            entity_type: entity_type.into(),
            resource_id: resource_id.into(),
            tenant_id: tenant_id.into(),
            attributes: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrustedAuthorizationRequest {
    pub principal: TrustedPrincipal,
    pub action: String,
    pub resource: TrustedResource,
    #[serde(default)]
    pub context: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CedarPermit {
    pub policy_version: String,
    pub policy_epoch: i64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CedarAuthorizationError {
    #[error("AUTHZ_DENIED")]
    Denied,
    #[error("AUTHZ_EVALUATION_ERROR")]
    Evaluation { diagnostic_codes: Vec<String> },
    #[error("AUTHZ_TENANT_CONTEXT_MISMATCH")]
    TenantContextMismatch,
    #[error("AUTHZ_INVALID_POLICY")]
    InvalidPolicy,
    #[error("AUTHZ_INVALID_POLICY_METADATA")]
    InvalidPolicyMetadata,
    #[error("AUTHZ_INVALID_ENTITY")]
    InvalidEntity,
    #[error("AUTHZ_INVALID_REQUEST")]
    InvalidRequest,
}

#[derive(Clone, Debug, Default)]
pub struct CedarAuthorizationAdapter;

impl CedarAuthorizationAdapter {
    /// Evaluate a host-built request against an immutable policy snapshot.
    ///
    /// Cedar already implements default-deny and forbid precedence. Masonwing
    /// adds one stricter invariant: any evaluation diagnostic is a hard deny,
    /// even if Cedar otherwise returns `Allow` because another permit matched.
    pub fn evaluate(
        &self,
        snapshot: &CedarPolicySnapshot,
        input: &TrustedAuthorizationRequest,
    ) -> Result<CedarPermit, CedarAuthorizationError> {
        if input.principal.tenant_id != input.resource.tenant_id {
            return Err(CedarAuthorizationError::TenantContextMismatch);
        }
        if input.principal.membership_epoch <= 0
            || input.principal.permission_epoch <= 0
            || input.principal.roles.is_empty()
            || !input
                .principal
                .roles
                .iter()
                .any(|role| role == &input.principal.role)
        {
            return Err(CedarAuthorizationError::InvalidEntity);
        }

        let principal_uid = entity_uid("User", &input.principal.principal_id)?;
        let action_uid = entity_uid("Action", &input.action)?;
        let resource_uid = entity_uid(&input.resource.entity_type, &input.resource.resource_id)?;

        let entities = build_entities(input)?;
        let context = Context::from_json_value(
            Value::Object(input.context.clone().into_iter().collect::<Map<_, _>>()),
            None,
        )
        .map_err(|_| CedarAuthorizationError::InvalidRequest)?;
        let request = Request::new(principal_uid, action_uid, resource_uid, context, None)
            .map_err(|_| CedarAuthorizationError::InvalidRequest)?;

        let response = Authorizer::new().is_authorized(&request, &snapshot.policies, &entities);
        let diagnostic_codes = response
            .diagnostics()
            .errors()
            .map(|error| diagnostic_code(error.to_string()))
            .collect::<Vec<_>>();
        if !diagnostic_codes.is_empty() {
            return Err(CedarAuthorizationError::Evaluation { diagnostic_codes });
        }

        match response.decision() {
            Decision::Allow => Ok(CedarPermit {
                policy_version: snapshot.policy_version.clone(),
                policy_epoch: snapshot.policy_epoch,
            }),
            Decision::Deny => Err(CedarAuthorizationError::Denied),
        }
    }
}

fn build_entities(
    input: &TrustedAuthorizationRequest,
) -> Result<Entities, CedarAuthorizationError> {
    let mut resource_attrs = input.resource.attributes.clone();
    // Tenant comes from the host-owned resource record and cannot be replaced by
    // a plugin/client attribute with the same name.
    resource_attrs.insert(
        "tenant_id".to_owned(),
        Value::String(input.resource.tenant_id.clone()),
    );

    let entities = json!([
        {
            "uid": {"type": "User", "id": input.principal.principal_id},
            "attrs": {
                "tenant_id": input.principal.tenant_id,
                "role": input.principal.role,
                "roles": input.principal.roles,
                "membership_epoch": input.principal.membership_epoch,
                "permission_epoch": input.principal.permission_epoch
            },
            "parents": []
        },
        {
            "uid": {"type": input.resource.entity_type, "id": input.resource.resource_id},
            "attrs": resource_attrs,
            "parents": []
        }
    ]);

    Entities::from_json_value(entities, None).map_err(|_| CedarAuthorizationError::InvalidEntity)
}

fn entity_uid(entity_type: &str, id: &str) -> Result<EntityUid, CedarAuthorizationError> {
    let entity_type = EntityTypeName::from_str(entity_type)
        .map_err(|_| CedarAuthorizationError::InvalidEntity)?;
    let id = EntityId::from_str(id).map_err(|_| CedarAuthorizationError::InvalidEntity)?;
    Ok(EntityUid::from_type_name_and_id(entity_type, id))
}

fn diagnostic_code(message: String) -> String {
    // Diagnostics are safe audit codes, never the policy source or entity bag.
    // Collapse implementation-specific text into a stable category.
    let _ = message;
    "CEDAR_EVALUATION_DIAGNOSTIC".to_owned()
}

impl AdapterBoundary for CedarAuthorizationAdapter {
    fn adapter_name(&self) -> &'static str {
        "authorization-cedar"
    }

    fn qualification(&self) -> AdapterQualification {
        AdapterQualification::Qualified
    }
}

impl AuthorizationPort for CedarAuthorizationAdapter {
    fn authorize_current(&self, _request: &AuthorizationRequest) -> Result<(), PortError> {
        // The kernel port lacks the immutable current policy snapshot and
        // trusted entity attributes required for a safe Cedar decision. Host
        // call sites therefore use `evaluate`; silently inventing those values
        // here would violate current-authority semantics.
        Err(PortError::Unavailable {
            adapter: self.adapter_name(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(action: &str) -> TrustedAuthorizationRequest {
        TrustedAuthorizationRequest {
            principal: TrustedPrincipal {
                principal_id: "11111111-1111-1111-1111-111111111111".to_owned(),
                tenant_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".to_owned(),
                role: "PUBLISHER".to_owned(),
                roles: vec!["PUBLISHER".to_owned()],
                membership_epoch: 3,
                permission_epoch: 9,
            },
            action: action.to_owned(),
            resource: TrustedResource::new(
                "Document",
                "doc-1",
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
            ),
            context: BTreeMap::new(),
        }
    }

    // MASONWING@1.0.1 REQ-042 / AC-044
    #[test]
    fn no_matching_permit_is_denied() {
        let snapshot = CedarPolicySnapshot::parse("1.0.0", 1, "").unwrap();
        assert_eq!(
            CedarAuthorizationAdapter.evaluate(&snapshot, &request("document.read")),
            Err(CedarAuthorizationError::Denied)
        );
    }

    // MASONWING@1.0.1 REQ-043 / AC-045
    #[test]
    fn forbid_overrides_matching_permit() {
        let policy = r#"
            permit(principal, action == Action::"document.publish", resource);
            forbid(principal, action == Action::"document.publish", resource)
                when { resource.frozen };
        "#;
        let snapshot = CedarPolicySnapshot::parse("1.0.0", 7, policy).unwrap();
        let mut input = request("document.publish");
        input
            .resource
            .attributes
            .insert("frozen".into(), Value::Bool(true));
        assert_eq!(
            CedarAuthorizationAdapter.evaluate(&snapshot, &input),
            Err(CedarAuthorizationError::Denied)
        );
    }

    // MASONWING@1.0.1 REQ-044 / AC-046
    #[test]
    fn any_evaluation_diagnostic_denies_even_with_matching_permit() {
        let policy = r#"
            permit(principal, action == Action::"document.publish", resource);
            forbid(principal, action == Action::"document.publish", resource)
                when { resource.missing_attribute == "blocked" };
        "#;
        let snapshot = CedarPolicySnapshot::parse("1.0.0", 7, policy).unwrap();
        let result = CedarAuthorizationAdapter.evaluate(&snapshot, &request("document.publish"));
        assert!(matches!(
            result,
            Err(CedarAuthorizationError::Evaluation { diagnostic_codes })
                if diagnostic_codes == vec!["CEDAR_EVALUATION_DIAGNOSTIC"]
        ));
    }

    // MASONWING@1.0.1 REQ-047 / AC-049
    #[test]
    fn host_tenant_mismatch_is_rejected_before_cedar() {
        let snapshot =
            CedarPolicySnapshot::parse("1.0.0", 1, "permit(principal, action, resource);").unwrap();
        let mut input = request("document.read");
        input.resource.tenant_id = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".into();
        assert_eq!(
            CedarAuthorizationAdapter.evaluate(&snapshot, &input),
            Err(CedarAuthorizationError::TenantContextMismatch)
        );
    }
}
