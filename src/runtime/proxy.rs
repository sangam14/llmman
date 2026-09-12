//! Outbound LLM Secret Injection Proxy (Gondolin-style).
//!
//! Intercepts outbound HTTPS/HTTP API requests to LLM providers (e.g. OpenAI,
//! Anthropic, HuggingFace), injects authorization headers at the network boundary,
//! and records token spend telemetry without exposing secrets to guest VM environments.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenFormat {
    OpenAi,
    Anthropic,
    HuggingFace,
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub name: String,
    pub domain: String,
    pub token_format: TokenFormat,
    pub auth_header_prefix: String,
}

pub struct SecretProxy {
    providers: HashMap<String, ProviderConfig>,
    secrets: HashMap<String, String>,
}

impl SecretProxy {
    pub fn new() -> Self {
        let mut providers = HashMap::new();
        
        providers.insert(
            "api.openai.com".to_string(),
            ProviderConfig {
                name: "openai".to_string(),
                domain: "api.openai.com".to_string(),
                token_format: TokenFormat::OpenAi,
                auth_header_prefix: "Bearer ".to_string(),
            },
        );

        providers.insert(
            "api.anthropic.com".to_string(),
            ProviderConfig {
                name: "anthropic".to_string(),
                domain: "api.anthropic.com".to_string(),
                token_format: TokenFormat::Anthropic,
                auth_header_prefix: "x-api-key: ".to_string(),
            },
        );

        providers.insert(
            "huggingface.co".to_string(),
            ProviderConfig {
                name: "huggingface".to_string(),
                domain: "huggingface.co".to_string(),
                token_format: TokenFormat::HuggingFace,
                auth_header_prefix: "Bearer ".to_string(),
            },
        );

        Self {
            providers,
            secrets: HashMap::new(),
        }
    }

    /// Register a host secret for a given provider name (e.g. "openai" -> "sk-proj-...").
    pub fn set_secret(&mut self, provider_name: &str, secret: &str) {
        self.secrets.insert(provider_name.to_string(), secret.to_string());
    }

    /// Lookup provider by request domain/host header.
    pub fn lookup_provider(&self, host: &str) -> Option<&ProviderConfig> {
        let clean_host = host.split(':').next().unwrap_or(host);
        self.providers.get(clean_host)
    }

    /// Formats the injected authorization header value for a target domain.
    pub fn inject_auth_header(&self, host: &str) -> Option<String> {
        let provider = self.lookup_provider(host)?;
        let secret = self.secrets.get(&provider.name)?;
        Some(format!("{}{}", provider.auth_header_prefix, secret))
    }

    /// Parse token usage metrics from an LLM API JSON response body.
    pub fn extract_token_usage(&self, host: &str, response_body: &[u8]) -> Option<TokenUsage> {
        let provider = self.lookup_provider(host)?;
        match provider.token_format {
            TokenFormat::OpenAi => {
                #[derive(Deserialize)]
                struct OpenAiUsage {
                    prompt_tokens: Option<u64>,
                    completion_tokens: Option<u64>,
                    total_tokens: Option<u64>,
                }
                #[derive(Deserialize)]
                struct OpenAiResp {
                    usage: Option<OpenAiUsage>,
                }

                let parsed: OpenAiResp = serde_json::from_slice(response_body).ok()?;
                let u = parsed.usage?;
                Some(TokenUsage {
                    input_tokens: u.prompt_tokens.unwrap_or(0),
                    output_tokens: u.completion_tokens.unwrap_or(0),
                    total_tokens: u.total_tokens.unwrap_or(0),
                })
            }
            TokenFormat::Anthropic => {
                #[derive(Deserialize)]
                struct AnthropicUsage {
                    input_tokens: Option<u64>,
                    output_tokens: Option<u64>,
                }
                #[derive(Deserialize)]
                struct AnthropicResp {
                    usage: Option<AnthropicUsage>,
                }

                let parsed: AnthropicResp = serde_json::from_slice(response_body).ok()?;
                let u = parsed.usage?;
                let input = u.input_tokens.unwrap_or(0);
                let output = u.output_tokens.unwrap_or(0);
                Some(TokenUsage {
                    input_tokens: input,
                    output_tokens: output,
                    total_tokens: input + output,
                })
            }
            TokenFormat::HuggingFace => None,
        }
    }
}

impl Default for SecretProxy {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_injection() {
        let mut proxy = SecretProxy::new();
        proxy.set_secret("openai", "sk-test-key-12345");

        let header = proxy.inject_auth_header("api.openai.com");
        assert_eq!(header, Some("Bearer sk-test-key-12345".to_string()));
    }

    #[test]
    fn test_openai_token_extraction() {
        let proxy = SecretProxy::new();
        let json_body = br#"{"usage": {"prompt_tokens": 12, "completion_tokens": 8, "total_tokens": 20}}"#;

        let usage = proxy.extract_token_usage("api.openai.com", json_body);
        assert_eq!(
            usage,
            Some(TokenUsage {
                input_tokens: 12,
                output_tokens: 8,
                total_tokens: 20
            })
        );
    }
}
