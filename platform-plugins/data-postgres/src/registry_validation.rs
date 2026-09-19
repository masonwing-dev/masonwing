//! Pure installation checks. No database, migration, provider or guest code is
//! reachable until these checks and the persisted publisher signature pass.

use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::{Signature, VerifyingKey};
use masonwing_contract_validation::{SchemaCatalog, canonical_bytes, digest_bytes};
use masonwing_contracts::{
    Digest, PluginId, TenantId,
    wire::{PluginManifest, WorkflowDefinition},
};
use masonwing_kernel::{DependencyGraph, RegistryError, runtime::CommandFailure};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Host extension carried by the baseline's opaque plugin_lock_ref. The lock
/// names exact artifacts; resolving it never means choosing the latest version.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginLock {
    pub schema_version: String,
    pub product_id: PluginId,
    pub plugins: Vec<LockedPlugin>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedPlugin {
    pub plugin_id: PluginId,
    pub artifact_digest: Digest,
    pub contract_version: String,
}

pub fn manifest_digest(manifest: &PluginManifest) -> Result<Digest, CommandFailure> {
    let mut content = serde_json::to_value(manifest).map_err(|_| invalid())?;
    content
        .as_object_mut()
        .ok_or_else(invalid)?
        .remove("signature_ref");
    Ok(digest_bytes(
        &canonical_bytes(&content).map_err(|_| invalid())?,
    ))
}

/// Ed25519 signs a JCS envelope, with a fixed purpose and both digests bound.
/// signature_ref is a lookup handle, not part of the immutable signed content.
pub fn manifest_signing_message(manifest: &PluginManifest) -> Result<Vec<u8>, CommandFailure> {
    canonical_bytes(&json!({
        "purpose": "masonwing.plugin-manifest.v1",
        "publisher_id": manifest.publisher_id,
        "artifact_digest": manifest.artifact_digest,
        "manifest_digest": manifest_digest(manifest)?,
    }))
    .map_err(|_| invalid())
}

pub(crate) fn verify_signature(
    manifest: &PluginManifest,
    key: &[u8],
    signature: &[u8],
) -> Result<(), CommandFailure> {
    let key: &[u8; 32] = key.try_into().map_err(|_| signature_invalid())?;
    let key = VerifyingKey::from_bytes(key).map_err(|_| signature_invalid())?;
    let signature = Signature::from_slice(signature).map_err(|_| signature_invalid())?;
    key.verify_strict(&manifest_signing_message(manifest)?, &signature)
        .map_err(|_| signature_invalid())
}

pub(crate) fn validate_manifest(
    manifest: &PluginManifest,
    tenant: &TenantId,
    known: &BTreeSet<String>,
) -> Result<(), CommandFailure> {
    let value = serde_json::to_value(manifest).map_err(|_| invalid())?;
    SchemaCatalog::shared()
        .map_err(|_| CommandFailure::unavailable("CONTRACT_CATALOG_INVALID"))?
        .validate("PluginManifest", &value)
        .map_err(|_| invalid())?;
    if manifest.contract_version != "1.0.0" {
        return Err(CommandFailure::precondition("CONTRACT_UNSUPPORTED"));
    }
    let requested: BTreeSet<_> = manifest
        .requested_capabilities
        .iter()
        .map(|a| a.as_str())
        .collect();
    if requested.len() != manifest.requested_capabilities.len() {
        return Err(invalid());
    }
    if requested.iter().any(|a| !known.contains(*a)) {
        return Err(CommandFailure::precondition("CONFIG_UNSUPPORTED"));
    }
    let mut handlers = BTreeSet::new();
    for handler in &manifest.handlers {
        if !handlers.insert(&handler.id)
            || handler
                .required_capabilities
                .iter()
                .any(|a| !requested.contains(a.as_str()))
        {
            return Err(CommandFailure::precondition("EXTENSION_SCOPE_DENIED"));
        }
        if handler.input_schema_ref.tenant_id != *tenant
            || handler.output_schema_ref.tenant_id != *tenant
        {
            return Err(CommandFailure::not_found());
        }
    }
    let mut tools = BTreeSet::new();
    for tool in &manifest.tools {
        if !tools.insert(&tool.id) || !requested.contains(tool.action.as_str()) {
            return Err(CommandFailure::precondition("EXTENSION_SCOPE_DENIED"));
        }
        if tool.input_schema_ref.tenant_id != *tenant || tool.output_schema_ref.tenant_id != *tenant
        {
            return Err(CommandFailure::not_found());
        }
    }
    if manifest
        .workflows
        .iter()
        .chain(&manifest.data_contracts)
        .any(|a| a.tenant_id != *tenant)
    {
        return Err(CommandFailure::not_found());
    }
    let mut contributions = BTreeSet::new();
    let mut locations = BTreeSet::new();
    let prefix = format!("/plugins/{}/", manifest.id);
    for ui in &manifest.ui {
        if !contributions.insert(&ui.id)
            || !locations.insert((&ui.path, &ui.slot))
            || !ui.path.starts_with(&prefix)
            || ui.path.len() > 1024
            || ui
                .path
                .bytes()
                .any(|b| b <= 32 || matches!(b, b'%' | b'\\' | b'?' | b'#'))
            || ui.path.split('/').any(|part| matches!(part, "." | ".."))
            || ui.path.contains("//")
            || !known.contains(ui.required_action.as_str())
            || ui.ui_contract_version != "1.0.0"
        {
            return Err(CommandFailure::precondition("EXTENSION_SCOPE_DENIED"));
        }
    }
    Ok(())
}

/// Selected versions are supplied by the transaction or an exact product lock.
/// Checking the whole graph also prevents upgrades breaking reverse dependants.
pub(crate) fn resolve_manifests(
    manifests: &BTreeMap<PluginId, PluginManifest>,
) -> Result<Vec<PluginId>, CommandFailure> {
    if manifests.len() > 512 {
        return Err(CommandFailure::precondition("REGISTRY_CAPACITY"));
    }
    let mut graph = DependencyGraph::default();
    for (id, manifest) in manifests {
        let mut edges = Vec::new();
        let mut seen = BTreeSet::new();
        for dependency in &manifest.dependencies {
            if !seen.insert(&dependency.plugin_id) {
                return Err(invalid());
            }
            let range = VersionReq::parse(&dependency.contract_range).map_err(|_| invalid())?;
            let Some(target) = manifests.get(&dependency.plugin_id) else {
                if dependency.optional {
                    continue;
                }
                return Err(CommandFailure::precondition("DEPENDENCY_UNSATISFIED"));
            };
            let version = Version::parse(&target.contract_version).map_err(|_| invalid())?;
            if !range.matches(&version) {
                return Err(CommandFailure::precondition("DEPENDENCY_UNSATISFIED"));
            }
            edges.push(dependency.plugin_id.clone());
        }
        graph.insert(id.clone(), edges);
    }
    let mut ordered = Vec::new();
    let mut seen = BTreeSet::new();
    for id in manifests.keys() {
        for resolved in graph.resolve(id).map_err(|e| match e {
            RegistryError::DependencyCycle { .. } => {
                CommandFailure::precondition("DEPENDENCY_CYCLE")
            }
            RegistryError::DependencyUnsatisfied { .. } => {
                CommandFailure::precondition("DEPENDENCY_UNSATISFIED")
            }
        })? {
            if seen.insert(resolved.clone()) {
                ordered.push(resolved);
            }
        }
    }
    Ok(ordered)
}

pub(crate) fn validate_lock(lock: &PluginLock, product: &str) -> Result<(), CommandFailure> {
    if lock.schema_version != "1.0.0"
        || lock.product_id.as_str() != product
        || lock.plugins.is_empty()
        || lock.plugins.len() > 100
    {
        return Err(CommandFailure::invalid("PLUGIN_LOCK_INVALID"));
    }
    let mut ids = BTreeSet::new();
    if lock
        .plugins
        .iter()
        .any(|p| !ids.insert(&p.plugin_id) || p.contract_version != "1.0.0")
    {
        return Err(CommandFailure::invalid("PLUGIN_LOCK_INVALID"));
    }
    Ok(())
}

pub(crate) fn validate_workflow(
    definition: &WorkflowDefinition,
    manifest: &PluginManifest,
) -> Result<(), CommandFailure> {
    let mut value = serde_json::to_value(definition).map_err(|_| workflow_invalid())?;
    SchemaCatalog::shared()
        .map_err(|_| CommandFailure::unavailable("CONTRACT_CATALOG_INVALID"))?
        .validate("WorkflowDefinition", &value)
        .map_err(|_| workflow_invalid())?;
    value
        .as_object_mut()
        .ok_or_else(workflow_invalid)?
        .remove("digest");
    if digest_bytes(&canonical_bytes(&value).map_err(|_| workflow_invalid())?) != definition.digest
    {
        return Err(workflow_invalid());
    }
    let nodes: BTreeMap<_, _> = definition
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n))
        .collect();
    if nodes.len() != definition.nodes.len() || !nodes.contains_key(definition.entry_node.as_str())
    {
        return Err(workflow_invalid());
    }
    if definition
        .required_actions
        .iter()
        .any(|a| !manifest.requested_capabilities.contains(a))
    {
        return Err(workflow_invalid());
    }
    for node in &definition.nodes {
        if let Some(handler) = &node.handler_id {
            let handler = manifest
                .handlers
                .iter()
                .find(|h| &h.id == handler)
                .ok_or_else(workflow_invalid)?;
            if node.input_schema_ref != handler.input_schema_ref
                || node.output_schema_ref != handler.output_schema_ref
                || node.timeout_seconds > handler.max_execution_seconds
                || handler
                    .required_capabilities
                    .iter()
                    .any(|a| !definition.required_actions.contains(a))
            {
                return Err(workflow_invalid());
            }
        } else if matches!(node.kind.as_str(), "DETERMINISTIC" | "ACTIVITY") {
            return Err(workflow_invalid());
        }
    }
    let mut edges = BTreeSet::new();
    // Removing bounded-loop nodes must leave an acyclic graph. This allows
    // explicit loops while rejecting cycles that escape their iteration guard.
    let mut unbounded = DependencyGraph::default();
    for node in &definition.nodes {
        if node.kind == "BOUNDED_LOOP" {
            continue;
        }
        let next = definition
            .edges
            .iter()
            .filter(|e| e.from == node.id)
            .filter_map(|e| nodes.get(e.to.as_str()))
            .filter(|n| n.kind != "BOUNDED_LOOP")
            .map(|n| PluginId::new(n.id.clone()).map_err(|_| workflow_invalid()))
            .collect::<Result<Vec<_>, _>>()?;
        unbounded.insert(
            PluginId::new(node.id.clone()).map_err(|_| workflow_invalid())?,
            next,
        );
    }
    for edge in &definition.edges {
        if !nodes.contains_key(edge.from.as_str())
            || !nodes.contains_key(edge.to.as_str())
            || !edges.insert((&edge.from, &edge.condition))
        {
            return Err(workflow_invalid());
        }
    }
    for node in &definition.nodes {
        if node.kind != "BOUNDED_LOOP"
            && unbounded
                .resolve(&PluginId::new(node.id.clone()).map_err(|_| workflow_invalid())?)
                .is_err()
        {
            return Err(workflow_invalid());
        }
    }
    Ok(())
}

pub(crate) fn invalid() -> CommandFailure {
    CommandFailure::invalid("MANIFEST_INVALID")
}
fn signature_invalid() -> CommandFailure {
    CommandFailure::precondition("SIGNATURE_INVALID")
}
fn workflow_invalid() -> CommandFailure {
    CommandFailure::precondition("WORKFLOW_INVALID")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::Value;

    fn manifest(id: &str) -> PluginManifest {
        serde_json::from_value(json!({"id":id,"version":"1.0.0","contract_version":"1.0.0",
            "artifact_digest":digest_bytes(b"package"),"publisher_id":"publisher","execution_class":"WASM_COMPONENT",
            "requested_capabilities":[],"dependencies":[],"ui":[],"migrations":[],"license_expression":"MIT",
            "sbom_digest":digest_bytes(b"sbom"),"signature_ref":"signature","handlers":[],"workflows":[],"data_contracts":[],"tools":[]})).unwrap()
    }

    #[test]
    fn signed_content_binds_code_and_permissions_but_not_signature_lookup_handle() {
        let mut m = manifest("a");
        let key = SigningKey::from_bytes(&[17; 32]); // Public synthetic test key only.
        let signature = key.sign(&manifest_signing_message(&m).unwrap()).to_bytes();
        verify_signature(&m, key.verifying_key().as_bytes(), &signature).unwrap();
        m.signature_ref = "rotated-handle".into();
        verify_signature(&m, key.verifying_key().as_bytes(), &signature).unwrap();
        m.artifact_digest = digest_bytes(b"replaced");
        assert_eq!(
            verify_signature(&m, key.verifying_key().as_bytes(), &signature)
                .unwrap_err()
                .code,
            "SIGNATURE_INVALID"
        );
    }

    #[test]
    fn dependencies_reject_cycles_missing_versions_and_reverse_breakage() {
        let mut a = manifest("a");
        let mut b = manifest("b");
        a.dependencies = serde_json::from_value(
            json!([{"plugin_id":"b","contract_range":"^1.0","optional":false}]),
        )
        .unwrap();
        assert_eq!(
            resolve_manifests(&BTreeMap::from([(a.id.clone(), a.clone())]))
                .unwrap_err()
                .code,
            "DEPENDENCY_UNSATISFIED"
        );
        b.dependencies = serde_json::from_value(
            json!([{"plugin_id":"a","contract_range":"^1.0","optional":false}]),
        )
        .unwrap();
        assert_eq!(
            resolve_manifests(&BTreeMap::from([
                (a.id.clone(), a.clone()),
                (b.id.clone(), b.clone())
            ]))
            .unwrap_err()
            .code,
            "DEPENDENCY_CYCLE"
        );
        b.dependencies.clear();
        assert_eq!(
            resolve_manifests(&BTreeMap::from([
                (a.id.clone(), a.clone()),
                (b.id.clone(), b.clone())
            ]))
            .unwrap(),
            vec![b.id.clone(), a.id.clone()]
        );
        b.contract_version = "2.0.0".into();
        assert_eq!(
            resolve_manifests(&BTreeMap::from([(a.id.clone(), a), (b.id.clone(), b)]))
                .unwrap_err()
                .code,
            "DEPENDENCY_UNSATISFIED"
        );
    }

    #[test]
    fn unknown_capability_and_foreign_ui_namespace_do_not_pass_installation() {
        let mut m = manifest("a");
        let tenant = TenantId::new("tenant").unwrap();
        let known = BTreeSet::from(["resource.read".to_owned()]);
        validate_manifest(&m, &tenant, &known).unwrap();
        m.requested_capabilities =
            vec![masonwing_contracts::Action::new("private.execute").unwrap()];
        assert_eq!(
            validate_manifest(&m, &tenant, &known).unwrap_err().code,
            "CONFIG_UNSUPPORTED"
        );
        m.requested_capabilities.clear();
        m.ui = serde_json::from_value(json!([{"id":"nav","slot":"NAVIGATION","path":"/plugins/b/private","ui_contract_version":"1.0.0","trust":"DECLARATIVE","required_action":"resource.read"}])).unwrap();
        assert_eq!(
            validate_manifest(&m, &tenant, &known).unwrap_err().code,
            "EXTENSION_SCOPE_DENIED"
        );
        m.ui[0].path = "/plugins/a/home".into();
        validate_manifest(&m, &tenant, &known).unwrap();
    }

    #[test]
    fn lock_is_closed_and_requires_unique_exact_versions() {
        let p = json!({"plugin_id":"a","artifact_digest":digest_bytes(b"package"),"contract_version":"1.0.0"});
        let value = json!({"schema_version":"1.0.0","product_id":"product","plugins":[p.clone()]});
        let lock: PluginLock = serde_json::from_value(value.clone()).unwrap();
        validate_lock(&lock, "product").unwrap();
        assert!(validate_lock(&lock, "other-product").is_err());
        let mut duplicate: PluginLock = serde_json::from_value(value.clone()).unwrap();
        duplicate.plugins.push(serde_json::from_value(p).unwrap());
        assert!(validate_lock(&duplicate, "product").is_err());
        let mut extra = value;
        extra["take_latest"] = Value::Bool(true);
        assert!(serde_json::from_value::<PluginLock>(extra).is_err());
    }

    // MASONWING@1.0.1 REQ-076 / AC-080: installing a workflow whose nodes name
    // handlers that do not exist, or whose graph has a cycle no BOUNDED_LOOP
    // bounds, must be refused as WORKFLOW_INVALID before any run exists.
    #[test]
    fn invalid_workflow_rejected_before_execution() {
        let artifact_ref = json!({
            "artifact_id": "a", "tenant_id": "t",
            "digest": digest_bytes(b"schema"),
            "schema_version": "1.0.0", "classification": "INTERNAL"
        });
        let handler = json!({
            "id": "h1",
            "input_schema_ref": artifact_ref,
            "output_schema_ref": artifact_ref,
            "effects": "DETERMINISTIC",
            "required_capabilities": [],
            "max_execution_seconds": 60
        });
        let mut m = manifest("a");
        m.handlers = serde_json::from_value(json!([handler])).unwrap();
        let node = |id: &str, kind: &str, handler_id: Option<&str>| {
            json!({
                "id": id, "kind": kind, "handler_id": handler_id,
                "input_schema_ref": artifact_ref, "output_schema_ref": artifact_ref,
                "max_attempts": 1, "timeout_seconds": 30, "max_iterations": 1,
                "on_failure": "FAIL"
            })
        };

        // A definition naming an unknown handler_id is refused at install.
        let def_unknown =
            build_workflow(vec![node("n1", "ACTIVITY", Some("missing"))], vec![], "n1");
        assert_eq!(
            validate_workflow(&def_unknown, &m).unwrap_err().code,
            "WORKFLOW_INVALID"
        );

        // An unbounded cycle with no BOUNDED_LOOP node bounding it is refused.
        let def_cycle = build_workflow(
            vec![
                node("a", "ACTIVITY", Some("h1")),
                node("b", "ACTIVITY", Some("h1")),
            ],
            vec![
                json!({"from":"a","to":"b","condition":"SUCCESS"}),
                json!({"from":"b","to":"a","condition":"SUCCESS"}),
            ],
            "a",
        );
        assert_eq!(
            validate_workflow(&def_cycle, &m).unwrap_err().code,
            "WORKFLOW_INVALID"
        );

        // A valid acyclic definition still passes.
        let def_ok = build_workflow(
            vec![
                node("a", "ACTIVITY", Some("h1")),
                node("b", "ACTIVITY", Some("h1")),
            ],
            vec![json!({"from":"a","to":"b","condition":"SUCCESS"})],
            "a",
        );
        validate_workflow(&def_ok, &m).unwrap();
    }

    fn build_workflow(
        nodes: Vec<Value>,
        edges: Vec<Value>,
        entry_node: &str,
    ) -> WorkflowDefinition {
        let artifact_ref = json!({
            "artifact_id": "a", "tenant_id": "t",
            "digest": digest_bytes(b"schema"),
            "schema_version": "1.0.0", "classification": "INTERNAL"
        });
        let mut value = json!({
            "id": "wf", "version": "1.0.0",
            "input_schema_ref": artifact_ref,
            "output_schema_ref": artifact_ref,
            "entry_node": entry_node,
            "nodes": nodes, "edges": edges,
            "max_total_steps": 10, "max_model_turns": 5,
            "required_actions": [], "digest": "sha256:0"
        });
        // The validator re-computes the digest over the definition with the
        // digest field removed, so build it the same way.
        let mut content = value.clone();
        content.as_object_mut().unwrap().remove("digest");
        value["digest"] = json!(digest_bytes(&canonical_bytes(&content).unwrap()));
        serde_json::from_value(value).unwrap()
    }
}
