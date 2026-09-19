//! Operator-side materialization of compiled SDK fixtures into tenant-local
//! artifact handles. The domain fixtures never receive a database/storage client.

use std::collections::BTreeMap;

use masonwing_contract_validation::{canonical_bytes, digest_bytes, strict_json};
use masonwing_contracts::{
    ArtifactId,
    wire::{ArtifactRef, WorkflowDefinition},
};
use masonwing_kernel::runtime::CommandFailure;
use masonwing_sdk::PackagedPluginFixture;
use uuid::Uuid;

pub fn materialize_fixture(
    mut fixture: PackagedPluginFixture,
) -> Result<PackagedPluginFixture, CommandFailure> {
    fixture.validate().map_err(|_| invalid())?;
    let mut references = BTreeMap::<ArtifactId, (ArtifactRef, ArtifactRef)>::new();
    for artifact in &fixture.supporting_artifacts {
        let mut local = artifact.reference.clone();
        local.artifact_id =
            ArtifactId::new(Uuid::new_v4().to_string()).expect("UUID artifact handle");
        if references
            .insert(
                artifact.reference.artifact_id.clone(),
                (artifact.reference.clone(), local),
            )
            .is_some()
        {
            return Err(invalid());
        }
    }
    let workflow_ids: Vec<_> = fixture
        .workflows
        .iter()
        .map(|reference| reference.artifact_id.clone())
        .collect();
    for artifact in &mut fixture.supporting_artifacts {
        if workflow_ids.contains(&artifact.reference.artifact_id) {
            let mut definition: WorkflowDefinition =
                serde_json::from_value(strict_json(&artifact.bytes).map_err(|_| invalid())?)
                    .map_err(|_| invalid())?;
            remap(&mut definition.input_schema_ref, &references)?;
            remap(&mut definition.output_schema_ref, &references)?;
            for node in &mut definition.nodes {
                remap(&mut node.input_schema_ref, &references)?;
                remap(&mut node.output_schema_ref, &references)?;
            }
            let mut value = serde_json::to_value(&definition).map_err(|_| invalid())?;
            value.as_object_mut().ok_or_else(invalid)?.remove("digest");
            definition.digest = digest_bytes(&canonical_bytes(&value).map_err(|_| invalid())?);
            artifact.bytes = canonical_bytes(&definition).map_err(|_| invalid())?;
            references
                .get_mut(&artifact.reference.artifact_id)
                .ok_or_else(invalid)?
                .1
                .digest = digest_bytes(&artifact.bytes);
        }
    }
    for handler in &mut fixture.handlers {
        remap(&mut handler.input_schema_ref, &references)?;
        remap(&mut handler.output_schema_ref, &references)?;
    }
    for reference in &mut fixture.workflows {
        remap(reference, &references)?;
    }
    for artifact in &mut fixture.supporting_artifacts {
        remap(&mut artifact.reference, &references)?;
    }
    fixture.validate().map_err(|_| invalid())?;
    Ok(fixture)
}

fn remap(
    reference: &mut ArtifactRef,
    replacements: &BTreeMap<ArtifactId, (ArtifactRef, ArtifactRef)>,
) -> Result<(), CommandFailure> {
    let (original, local) = replacements
        .get(&reference.artifact_id)
        .ok_or_else(invalid)?;
    if reference != original {
        return Err(invalid());
    }
    *reference = local.clone();
    Ok(())
}

fn invalid() -> CommandFailure {
    CommandFailure::precondition("FIXTURE_IMPORT_INVALID")
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonwing_contract_validation::{ArtifactSchema, SchemaCatalog};
    use masonwing_contracts::TenantId;

    #[test]
    fn imported_fixtures_use_real_handles_and_rehash_exact_workflow_content() {
        let tenant = TenantId::new(Uuid::new_v4().to_string()).unwrap();
        for (original, _) in crate::plugin_runtime::local_fixture_plugins(&tenant) {
            let fixture = materialize_fixture(original.clone()).unwrap();
            assert_eq!(fixture.artifact_digest, original.artifact_digest);
            assert_eq!(
                fixture.handlers[0].input_schema_ref.digest,
                original.handlers[0].input_schema_ref.digest
            );
            assert_ne!(
                fixture.handlers[0].input_schema_ref.artifact_id,
                original.handlers[0].input_schema_ref.artifact_id
            );
            assert_ne!(fixture.workflows[0].digest, original.workflows[0].digest);
            for artifact in &fixture.supporting_artifacts {
                Uuid::parse_str(artifact.reference.artifact_id.as_str()).unwrap();
                assert_eq!(artifact.reference.tenant_id, tenant);
                assert_eq!(digest_bytes(&artifact.bytes), artifact.reference.digest);
                if fixture.workflows.contains(&artifact.reference) {
                    let mut value = strict_json(&artifact.bytes).unwrap();
                    SchemaCatalog::shared()
                        .unwrap()
                        .validate("WorkflowDefinition", &value)
                        .unwrap();
                    let digest = value.as_object_mut().unwrap().remove("digest").unwrap();
                    assert_eq!(
                        serde_json::to_value(digest_bytes(&canonical_bytes(&value).unwrap()))
                            .unwrap(),
                        digest
                    );
                } else {
                    ArtifactSchema::compile(&artifact.bytes).unwrap();
                }
            }
        }
    }

    #[test]
    fn missing_or_ambiguous_schema_payload_cannot_be_imported() {
        let tenant = TenantId::new("tenant").unwrap();
        let mut fixture = checksum_fixture::fixture_bundle(&tenant);
        fixture.supporting_artifacts.remove(0);
        assert!(materialize_fixture(fixture).is_err());
        let mut fixture = checksum_fixture::fixture_bundle(&tenant);
        fixture
            .supporting_artifacts
            .push(fixture.supporting_artifacts[0].clone());
        assert!(materialize_fixture(fixture).is_err());
    }
}
