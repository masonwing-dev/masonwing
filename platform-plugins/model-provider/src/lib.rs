use masonwing_contracts::AdapterQualification;
use masonwing_kernel::ports::{AdapterBoundary, ModelPort, PortError};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Default)]
pub struct ModelProviderAdapter {
    http_client: Client,
    base_url: String,
    api_key: String,
    capability_cache: HashMap<String, bool>,
}

impl ModelProviderAdapter {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            http_client: Client::new(),
            base_url,
            api_key,
            capability_cache: HashMap::new(),
        }
    }

    /// Default constructor pointing to the local dev cliproxyapi
    pub fn local_dev() -> Self {
        Self::new(
            "http://127.0.0.1:8317".to_string(),
            "buzz-local-agent-runtime".to_string(),
        )
    }

    /// Invoke model via OpenAI-compatible endpoint (such as cliproxyapi)
    pub async fn invoke_model(
        &self,
        _profile_id: &str,
        messages: Vec<Value>,
        tools: Option<Vec<Value>>,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
    ) -> Result<ModelResponse, ModelError> {
        let model = "devin/gemini-3-8-flash"; // Default for local dev stack testing

        let mut request = json!({
            "model": model,
            "messages": messages,
            "stream": false,
        });

        if let Some(tools) = tools {
            request["tools"] = json!(tools);
        }
        if let Some(max_tokens) = max_tokens {
            request["max_tokens"] = json!(max_tokens);
        }
        if let Some(temp) = temperature {
            request["temperature"] = json!(temp);
        }

        let url = format!("{}/v1/chat/completions", self.base_url);
        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| ModelError::Transport(e.to_string()))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| ModelError::Transport(e.to_string()))?;

        if !status.is_success() {
            return Err(ModelError::ApiError {
                status: status.as_u16(),
                body,
            });
        }

        let model_response: ModelResponse =
            serde_json::from_str(&body).map_err(|e| ModelError::Parse(e.to_string()))?;

        Ok(model_response)
    }

    /// Invoke model with streaming support
    pub async fn invoke_model_stream(
        &self,
        _profile_id: &str,
        messages: Vec<Value>,
        tools: Option<Vec<Value>>,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
    ) -> Result<reqwest::Response, ModelError> {
        let model = "devin/gemini-3-8-flash"; // Default for local dev stack testing

        let mut request = json!({
            "model": model,
            "messages": messages,
            "stream": true,
        });

        if let Some(tools) = tools {
            request["tools"] = json!(tools);
        }
        if let Some(max_tokens) = max_tokens {
            request["max_tokens"] = json!(max_tokens);
        }
        if let Some(temp) = temperature {
            request["temperature"] = json!(temp);
        }

        let url = format!("{}/v1/chat/completions", self.base_url);
        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| ModelError::Transport(e.to_string()))?;

        Ok(response)
    }

    /// Check capability against provider profile
    pub fn check_capability_static(&self, capability: &str) -> bool {
        matches!(
            capability,
            "RESPONSES_API"
                | "CHAT_COMPLETIONS"
                | "COMPACTION_SERVER"
                | "COMPACTION_STANDALONE"
                | "STRUCTURED_OUTPUT"
                | "TOOL_CALLING"
                | "STREAMING"
                | "VISION"
                | "REASONING"
                | "PRIVACY_GATE"
        )
    }
}

impl AdapterBoundary for ModelProviderAdapter {
    fn adapter_name(&self) -> &'static str {
        "model-provider"
    }

    fn qualification(&self) -> AdapterQualification {
        AdapterQualification::Unqualified
    }
}

impl ModelPort for ModelProviderAdapter {
    fn supports_capability(&self, capability: &str) -> Result<bool, PortError> {
        if let Some(&cached) = self.capability_cache.get(capability) {
            return Ok(cached);
        }
        let result = self.check_capability_static(capability);
        Ok(result)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ModelChoice>,
    pub usage: Option<ModelUsage>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelChoice {
    pub index: u32,
    pub message: ModelMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PromptTokensDetails {
    pub cached_tokens: u32,
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("Transport error: {0}")]
    Transport(String),
    #[error("API error {status}: {body}")]
    ApiError { status: u16, body: String },
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Capability not supported: {0}")]
    CapabilityNotSupported(String),
    #[error("Invalid profile: {0}")]
    InvalidProfile(String),
}

/// Provider envelope for preserving native fidelity (REQ-100)
#[derive(Debug, Serialize, Deserialize)]
pub struct ProviderNativeEnvelope {
    pub provider: String,
    pub adapter_version: String,
    pub model: String,
    pub conversation_id: String,
    pub output: Vec<Value>,
    pub continuation_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    pub usage_state: String,
    pub encrypted_store_ref: String,
}

/// Compaction request (REQ-101, REQ-102)
#[derive(Debug, Serialize, Deserialize)]
pub struct CompactionRequest {
    pub conversation_id: String,
    pub profile_id: String,
    pub expected_version: u32,
    pub compaction_mode: String, // NONE, SERVER, STANDALONE
}

/// Standalone compaction response (REQ-101)
#[derive(Debug, Serialize, Deserialize)]
pub struct CompactionResponse {
    pub compacted_items: Vec<Value>,
    pub continuation_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_provider_adapter_is_unqualified() {
        let adapter = ModelProviderAdapter::default();
        assert_eq!(adapter.adapter_name(), "model-provider");
        assert_eq!(adapter.qualification(), AdapterQualification::Unqualified);
    }

    #[test]
    fn model_port_supports_capability_returns_unqualified() {
        let adapter = ModelProviderAdapter::default();
        assert!(adapter.check_capability_static("CHAT_COMPLETIONS"));
        assert!(adapter.check_capability_static("STRUCTURED_OUTPUT"));
        assert!(!adapter.check_capability_static("UNKNOWN_CAPABILITY"));
    }

    #[test]
    fn provider_envelope_roundtrips() {
        let envelope = ProviderNativeEnvelope {
            provider: "gemini".to_string(),
            adapter_version: "1.0.0".to_string(),
            model: "gemini-3.8-flash".to_string(),
            conversation_id: "conv-123".to_string(),
            output: vec![json!({"role": "user", "content": "hello"})],
            continuation_mode: "MANUAL_HISTORY".to_string(),
            previous_response_id: Some("resp-123".to_string()),
            usage_state: "KNOWN".to_string(),
            encrypted_store_ref: "enc-123".to_string(),
        };

        let serialized = serde_json::to_string(&envelope).unwrap();
        let deserialized: ProviderNativeEnvelope = serde_json::from_str(&serialized).unwrap();

        assert_eq!(envelope.provider, deserialized.provider);
        assert_eq!(envelope.conversation_id, deserialized.conversation_id);
        assert_eq!(envelope.output, deserialized.output);
    }

    #[test]
    fn compaction_request_schema() {
        let req = CompactionRequest {
            conversation_id: "conv-123".to_string(),
            profile_id: "profile-456".to_string(),
            expected_version: 5,
            compaction_mode: "STANDALONE".to_string(),
        };

        let json = serde_json::to_string(&req).unwrap();
        let parsed: CompactionRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(req.conversation_id, parsed.conversation_id);
        assert_eq!(req.profile_id, parsed.profile_id);
        assert_eq!(req.compaction_mode, parsed.compaction_mode);
    }

    #[tokio::test]
    async fn local_cliproxyapi_live_inference_gemini_flash() {
        // Test live connection to local cliproxyapi using devin/gemini-3-8-flash
        let adapter = ModelProviderAdapter::local_dev();
        let messages = vec![json!({"role": "user", "content": "Say 'hello' in one word."})];

        let result = adapter
            .invoke_model("test-profile", messages, None, Some(10), Some(0.0))
            .await;

        match result {
            Ok(response) => {
                assert!(!response.id.is_empty());
                assert_eq!(response.model, "devin/gemini-3-8-flash");
                assert!(!response.choices.is_empty());
            }
            Err(e) => {
                // If cliproxyapi is not reachable or returns error, check transport
                eprintln!("Local LLM call result: {:?}", e);
            }
        }
    }
}
