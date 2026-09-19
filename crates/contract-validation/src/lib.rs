//! Offline validation against the checked-in, generated wire contracts.
//!
//! The schema is data, not executable authority. External reference retrieval is
//! disabled at the dependency level. Callers never receive submitted values in
//! validation diagnostics, including for nested secret-bearing command fields.

use std::{collections::BTreeMap, sync::OnceLock};

use masonwing_contracts::{Digest, is_platform_operation};
use serde::{
    Deserialize, Serialize,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value, json};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

pub const MAX_COMMAND_BYTES: usize = 1024 * 1024;
const SCHEMA: &str = include_str!("../../../contracts/masonwing/contracts.json");
static CATALOG: OnceLock<Result<SchemaCatalog, ValidationFailure>> = OnceLock::new();

pub struct SchemaCatalog {
    validators: BTreeMap<String, jsonschema::Validator>,
}

/// Offline JSON Schema for a plugin's application payload. Supported references
/// are fragment-only; neither a schema URI nor a payload can initiate I/O.
/// Diagnostics deliberately omit submitted values and arbitrary property names.
pub struct ArtifactSchema {
    validator: jsonschema::Validator,
}

impl ArtifactSchema {
    pub fn compile(bytes: &[u8]) -> Result<Self, ValidationFailure> {
        if bytes.len() > 256 * 1024 {
            return Err(ValidationFailure::new("PLUGIN_SCHEMA_LIMIT", ""));
        }
        let value = strict_json(bytes)?;
        if !value.is_object() && !value.is_boolean() {
            return Err(ValidationFailure::new("PLUGIN_SCHEMA_INVALID", ""));
        }
        let mut remaining = 8192;
        check_schema_profile(&value, 0, &mut remaining)?;
        let validator = jsonschema::draft202012::options()
            .should_validate_formats(true)
            .build(&value)
            .map_err(|_| ValidationFailure::new("PLUGIN_SCHEMA_INVALID", ""))?;
        Ok(Self { validator })
    }

    pub fn validate_bytes(&self, bytes: &[u8], maximum: usize) -> Result<(), ValidationFailure> {
        if bytes.len() > maximum {
            return Err(ValidationFailure::new("PLUGIN_PAYLOAD_LIMIT", ""));
        }
        let value = strict_json(bytes)?;
        if !self.validator.is_valid(&value) {
            return Err(ValidationFailure::new("PLUGIN_PAYLOAD_INVALID", ""));
        }
        Ok(())
    }
}

fn check_schema_profile(
    value: &Value,
    depth: usize,
    remaining: &mut usize,
) -> Result<(), ValidationFailure> {
    if depth > 64 || *remaining == 0 {
        return Err(ValidationFailure::new("PLUGIN_SCHEMA_LIMIT", ""));
    }
    *remaining -= 1;
    match value {
        Value::Object(object) => {
            // Fragment references are sufficient for a versioned schema artifact.
            // External schemas must be materialized into that artifact by its
            // publisher, so the signature/digest covers all validation rules.
            if object
                .get("$ref")
                .is_some_and(|reference| reference.as_str().is_none_or(|r| !r.starts_with('#')))
                || object.contains_key("$dynamicRef")
                || object.contains_key("$recursiveRef")
                || object
                    .get("$id")
                    .is_some_and(|id| id.as_str().is_none_or(|id| !id.starts_with('#')))
            {
                return Err(ValidationFailure::new(
                    "PLUGIN_SCHEMA_REFERENCE_UNSUPPORTED",
                    "",
                ));
            }
            if object
                .get("pattern")
                .and_then(Value::as_str)
                .is_some_and(|pattern| pattern.len() > 512)
                || object
                    .get("patternProperties")
                    .and_then(Value::as_object)
                    .is_some_and(|patterns| patterns.keys().any(|pattern| pattern.len() > 512))
            {
                return Err(ValidationFailure::new("PLUGIN_SCHEMA_LIMIT", ""));
            }
            for child in object.values() {
                check_schema_profile(child, depth + 1, remaining)?;
            }
        }
        Value::Array(array) => {
            for child in array {
                check_schema_profile(child, depth + 1, remaining)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Clone, Debug, Error, PartialEq, Eq, Serialize)]
#[error("{code}")]
pub struct ValidationFailure {
    pub code: &'static str,
    pub field: String,
}

impl ValidationFailure {
    fn new(code: &'static str, field: impl Into<String>) -> Self {
        Self {
            code,
            field: field.into(),
        }
    }
}

impl SchemaCatalog {
    pub fn load() -> Result<Self, ValidationFailure> {
        let mut root: Value = serde_json::from_str(SCHEMA)
            .map_err(|_| ValidationFailure::new("CONTRACT_CATALOG_INVALID", ""))?;
        // A local reference keeps validation independent from remote registries.
        root.as_object_mut()
            .expect("checked-in object")
            .remove("$id");
        let names: Vec<String> = root["$defs"]
            .as_object()
            .ok_or_else(|| ValidationFailure::new("CONTRACT_CATALOG_INVALID", ""))?
            .keys()
            .cloned()
            .collect();
        let mut validators = BTreeMap::new();
        for name in names {
            root["$ref"] = Value::String(format!("#/$defs/{name}"));
            let validator = jsonschema::draft202012::options()
                .should_validate_formats(true)
                .build(&root)
                .map_err(|_| ValidationFailure::new("CONTRACT_CATALOG_INVALID", &name))?;
            validators.insert(name, validator);
        }
        Ok(Self { validators })
    }

    pub fn shared() -> Result<&'static Self, ValidationFailure> {
        CATALOG
            .get_or_init(Self::load)
            .as_ref()
            .map_err(Clone::clone)
    }

    pub fn validate(&self, definition: &str, value: &Value) -> Result<(), ValidationFailure> {
        let validator = self
            .validators
            .get(definition)
            .ok_or_else(|| ValidationFailure::new("CONTRACT_UNSUPPORTED", ""))?;
        validator.validate(value).map_err(|error| {
            // Do not echo paths supplied as unknown keys or arbitrary map keys.
            // Top-level field names come from the fixed command/schema catalog.
            let path = error.instance_path().to_string();
            let safe = path.split('/').nth(1).unwrap_or("");
            let field = if safe.len() <= 64
                && safe.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                safe.to_owned()
            } else {
                String::new()
            };
            ValidationFailure::new("SCHEMA_INVALID", field)
        })
    }

    pub fn command(&self, operation: &str, body: &[u8]) -> Result<Value, ValidationFailure> {
        if !is_platform_operation(operation) {
            return Err(ValidationFailure::new("OPERATION_NOT_FOUND", ""));
        }
        if body.len() > MAX_COMMAND_BYTES {
            return Err(ValidationFailure::new("PAYLOAD_TOO_LARGE", ""));
        }
        let value = strict_json(body)?;
        self.validate(
            &format!("Command_{}", operation.replace(['.', '-'], "_")),
            &value,
        )?;
        Ok(value)
    }
}

/// JCS serialization makes semantically identical JSON key orders and integer
/// representations share the same fingerprint. Wire numbers are bounded to the
/// exact JavaScript integer range by the immutable command schemas.
pub fn canonical_bytes(value: &impl Serialize) -> Result<Vec<u8>, ValidationFailure> {
    serde_json_canonicalizer::to_vec(value)
        .map_err(|_| ValidationFailure::new("CANONICALIZATION_FAILED", ""))
}

pub fn digest_bytes(bytes: &[u8]) -> Digest {
    Digest::sha256(format!("{:x}", Sha256::digest(bytes)))
        .expect("SHA256 encodes a lowercase 256-bit digest")
}

pub fn fingerprint(operation: &str, input: &Value) -> Result<Digest, ValidationFailure> {
    Ok(digest_bytes(&canonical_bytes(
        &json!({"operation": operation, "input": input}),
    )?))
}

/// Reject duplicate object members before schema validation. Deserializing a
/// plain serde_json::Value would silently retain only the last duplicate.
pub fn strict_json(bytes: &[u8]) -> Result<Value, ValidationFailure> {
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValue::deserialize(&mut decoder)
        .map_err(|_| ValidationFailure::new("JSON_INVALID", ""))?;
    decoder
        .end()
        .map_err(|_| ValidationFailure::new("JSON_INVALID", ""))?;
    Ok(value.0)
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: de::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictValue;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("JSON with unique object keys")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Bool(value)))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Number(value.into())))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Number(value.into())))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Number::from_f64(value)
                    .map(|n| StrictValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::String(value.into())))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::String(value)))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element::<StrictValue>()? {
                    items.push(item.0);
                }
                Ok(StrictValue(Value::Array(items)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom("duplicate object key"));
                    }
                    values.insert(key, map.next_value::<StrictValue>()?.0);
                }
                Ok(StrictValue(Value::Object(values)))
            }
        }
        decoder.deserialize_any(StrictVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // MASONWING@1.0.1 REQ-060 / AC-063. No storage is reachable from validation.
    #[test]
    fn command_contract_rejects_unknown_fields_numbers_formats_and_duplicates() {
        let schemas = SchemaCatalog::shared().unwrap();
        assert!(schemas.command("budget.configure", br#"{"currency":"USD","period":"DAY","limit_microunits":100,"expected_version":1}"#).is_ok());
        for body in [
            br#"{"currency":"USD","period":"DAY","limit_microunits":100,"expected_version":1,"admin":true}"#.as_slice(),
            br#"{"currency":"USD","period":"DAY","limit_microunits":9007199254740992,"expected_version":1}"#,
            br#"{"currency":"USD","period":"DAY","limit_microunits":100,"expected_version":0}"#,
            br#"{"currency":"USD","period":"DAY","limit_microunits":100,"expected_version":1,"expected_version":2}"#,
        ] { assert!(schemas.command("budget.configure", body).is_err()); }
        assert!(
            schemas
                .command(
                    "membership.invite",
                    br#"{"email":"invalid","roles":["VIEWER"],"expires_at":"tomorrow"}"#
                )
                .is_err()
        );
        assert_eq!(
            schemas.command("sql.eval", b"{}").unwrap_err().code,
            "OPERATION_NOT_FOUND"
        );
        assert_eq!(
            schemas
                .command("run.start", &vec![b' '; MAX_COMMAND_BYTES + 1])
                .unwrap_err()
                .code,
            "PAYLOAD_TOO_LARGE"
        );
    }

    #[test]
    fn nested_duplicate_and_trailing_json_are_not_silently_normalized() {
        assert!(strict_json(br#"{"nested":{"x":1,"x":2}}"#).is_err());
        assert!(strict_json(br#"{"x":1} {"y":2}"#).is_err());
        assert_eq!(
            strict_json(br#"[null,true,1,-2,1.5,"text"]"#).unwrap(),
            json!([null, true, 1, -2, 1.5, "text"])
        );
    }

    #[test]
    fn fingerprint_ignores_key_order_but_binds_operation_payload_and_version() {
        let first = strict_json(br#"{"b":1.0,"a":2,"expected_version":1}"#).unwrap();
        let reordered = strict_json(br#"{"expected_version":1,"a":2,"b":1}"#).unwrap();
        assert_eq!(fingerprint("op", &first), fingerprint("op", &reordered));
        assert_ne!(fingerprint("op", &first), fingerprint("other", &first));
        assert_ne!(
            fingerprint("op", &first),
            fingerprint("op", &json!({"a":2,"b":1,"expected_version":2}))
        );
    }

    #[test]
    fn validation_does_not_echo_secret_payloads() {
        let error = SchemaCatalog::shared()
            .unwrap()
            .command(
                "run.start",
                br#"{"CANARY_SECRET_TOKEN":"CANARY_SECRET_TOKEN"}"#,
            )
            .unwrap_err();
        assert!(!format!("{error:?}").contains("CANARY_SECRET_TOKEN"));
    }

    #[test]
    fn artifact_schema_validates_payload_instead_of_schema_identity() {
        let schema = ArtifactSchema::compile(br##"{"$defs":{"octet":{"type":"integer","minimum":0,"maximum":255}},"type":"array","items":{"$ref":"#/$defs/octet"},"maxItems":4}"##).unwrap();
        schema.validate_bytes(b"[97,98,99]", 1024).unwrap();
        for input in [
            b"[256]".as_slice(),
            b"[1.5]",
            b"[true]",
            b"[1,2,3,4,5]",
            br#""not an array""#,
        ] {
            assert!(schema.validate_bytes(input, 1024).is_err());
        }
        assert!(schema.validate_bytes(b"[]", 1).is_err());
        let output =
            ArtifactSchema::compile(br#"{"type":"string","pattern":"^sha256:[0-9a-f]{64}$"}"#)
                .unwrap();
        output
            .validate_bytes(&canonical_bytes(&digest_bytes(b"abc")).unwrap(), 1024)
            .unwrap();
        assert!(output.validate_bytes(b"sha256:unquoted", 1024).is_err());
    }

    #[test]
    fn artifact_schema_cannot_retrieve_network_or_files_and_diagnostics_are_sanitized() {
        for schema in [
            br#"{"$ref":"https://example.test/private"}"#.as_slice(),
            br#"{"$ref":"file:///private/secret"}"#,
            br##"{"$id":"https://example.test/private","$ref":"#/x"}"##,
            br##"{"$dynamicRef":"#anything"}"##,
            br#"{"type":"not-a-json-schema-type"}"#,
        ] {
            assert!(ArtifactSchema::compile(schema).is_err());
        }
        let schema =
            ArtifactSchema::compile(br#"{"type":"object","additionalProperties":false}"#).unwrap();
        let error = schema
            .validate_bytes(br#"{"SECRET_VALUE":"SECRET_VALUE"}"#, 1024)
            .unwrap_err();
        assert!(!format!("{error:?}").contains("SECRET_VALUE"));
        assert!(schema.validate_bytes(br#"{"x":1,"x":2}"#, 1024).is_err());
    }
}
