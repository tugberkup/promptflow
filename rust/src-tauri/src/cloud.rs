//! Cleanup through a cloud model with the user's own API key: Anthropic, Google or OpenAI.
//! Keys live in the OS keychain (Windows Credential Manager, macOS Keychain), never in settings.json.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

const KEYCHAIN_SERVICE: &str = "PromptFlow";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Provider {
    Anthropic,
    Google,
    OpenAi,
}

impl Provider {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "anthropic" => Some(Self::Anthropic),
            "google" => Some(Self::Google),
            "openai" => Some(Self::OpenAi),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Google => "google",
            Self::OpenAi => "openai",
        }
    }
}

fn entry(provider: Provider) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, provider.id()).map_err(|e| anyhow!("keychain: {e}"))
}

pub fn key(provider: Provider) -> Option<String> {
    entry(provider).ok()?.get_password().ok().filter(|k| !k.is_empty())
}

pub fn set_key(provider: Provider, key: &str) -> Result<()> {
    entry(provider)?.set_password(key).map_err(|e| anyhow!("keychain: {e}"))
}

pub fn delete_key(provider: Provider) {
    if let Ok(e) = entry(provider) {
        let _ = e.delete_credential();
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        // Read error bodies too: they carry the provider's explanation ("invalid x-api-key", ...)
        .http_status_as_error(false)
        .build()
        .into()
}

fn read(response: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value> {
    let mut response = response.context("couldn't reach the provider")?;
    let status = response.status();
    let body: Value = response.body_mut().read_json().unwrap_or(Value::Null);
    if !status.is_success() {
        // All three put the human-readable reason under error.message
        let message = body["error"]["message"].as_str().unwrap_or("").trim().to_string();
        bail!("HTTP {}{}", status.as_u16(), if message.is_empty() { String::new() } else { format!(": {message}") });
    }
    Ok(body)
}

/// Models this key can use for text, best default first.
pub fn models(provider: Provider, key: &str) -> Result<Vec<String>> {
    let agent = agent(Duration::from_secs(15));
    let mut ids: Vec<String> = match provider {
        Provider::Anthropic => {
            let body = read(
                agent
                    .get("https://api.anthropic.com/v1/models?limit=100")
                    .header("x-api-key", key)
                    .header("anthropic-version", "2026-10-21")
                    .call(),
            )?;
            ids(&body["data"], "id")
        }
        Provider::Google => {
            let body = read(
                agent
                    .get("https://generativelanguage.googleapis.com/v1beta/models?pageSize=200")
                    .header("x-goog-api-key", key)
                    .call(),
            )?;
            body["models"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|m| m["supportedGenerationMethods"].as_array().is_some_and(|g| g.iter().any(|g| g == "generateContent")))
                .filter_map(|m| m["name"].as_str().map(|n| n.trim_start_matches("models/").to_string()))
                .filter(|n| n.starts_with("gemini") && !["tts", "image", "embedding", "live", "audio"].iter().any(|x| n.contains(x)))
                .collect()
        }
        Provider::OpenAi => {
            let body = read(agent.get("https://api.openai.com/v1/models").header("Authorization", format!("Bearer {key}")).call())?;
            ids(&body["data"], "id")
                .into_iter()
                .filter(|n| n.starts_with("gpt-"))
                .filter(|n| !["audio", "realtime", "image", "search", "transcribe", "tts"].iter().any(|x| n.contains(x)))
                .collect()
        }
    };
    if provider != Provider::Anthropic {
        // Anthropic lists newest first already; for the others, newest version first
        ids.sort_by(|a, b| b.cmp(a));
    }
    let score = |id: &str| -> u8 {
        match provider {
            Provider::Anthropic if id.contains("haiku") => 0,
            Provider::Google if id.ends_with("flash-lite-latest") => 0,
            Provider::Google if id.contains("flash-lite") && !id.contains("preview") => 1,
            Provider::Google if id.contains("flash") && !id.contains("preview") => 2,
            Provider::OpenAi if id.contains("mini") && !id.contains("preview") => 0,
            _ => 5,
        }
    };
    ids.sort_by_key(|id| score(id)); // stable: newest first within each group
    if ids.is_empty() {
        bail!("this key has no text models");
    }
    Ok(ids)
}

fn ids(list: &Value, field: &str) -> Vec<String> {
    list.as_array().into_iter().flatten().filter_map(|m| m[field].as_str().map(String::from)).collect()
}

/// One request: system prompt, example turns, then `text`. The reply is a JSON object.
pub fn complete_json(
    provider: Provider,
    key: &str,
    model: &str,
    system: &str,
    examples: &[(String, String)],
    text: &str,
) -> Result<Value> {
    let agent = agent(Duration::from_secs(30));
    let reply = match provider {
        Provider::Anthropic => {
            let mut messages = Vec::new();
            for (user, assistant) in examples {
                messages.push(json!({"role": "user", "content": user}));
                messages.push(json!({"role": "assistant", "content": assistant}));
            }
            messages.push(json!({"role": "user", "content": text}));
            let mut body = json!({
                "model": model,
                "max_tokens": 4096,
                "system": [{"type": "text", "text": system, "cache_control": {"type": "ephemeral"}}],
                "messages": messages,
                "output_config": {"format": {"type": "json_schema", "schema": {
                    "type": "object",
                    "properties": {"language": {"type": ["string", "null"]}, "text": {"type": "string"}},
                    "required": ["language", "text"],
                    "additionalProperties": false,
                }}},
            });
            // Haiku can skip thinking, which halves the wait; the bigger models keep it at low effort
            if model.contains("haiku") {
                body["thinking"] = json!({"type": "disabled"});
            } else {
                body["output_config"]["effort"] = json!("low");
            }
            let response = read(
                agent
                    .post("https://api.anthropic.com/v1/messages")
                    .header("x-api-key", key)
                    .header("anthropic-version", "2026-10-21")
                    .send_json(body),
            )?;
            if response["stop_reason"] == "refusal" {
                bail!("the model declined this text");
            }
            response["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<String>()
        }
        Provider::Google => {
            let mut contents = Vec::new();
            for (user, model_reply) in examples {
                contents.push(json!({"role": "user", "parts": [{"text": user}]}));
                contents.push(json!({"role": "model", "parts": [{"text": model_reply}]}));
            }
            contents.push(json!({"role": "user", "parts": [{"text": text}]}));
            let response = read(
                agent
                    .post(&format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"))
                    .header("x-goog-api-key", key)
                    .send_json(json!({
                        "systemInstruction": {"parts": [{"text": system}]},
                        "contents": contents,
                        "generationConfig": {
                            "responseMimeType": "application/json",
                            "responseSchema": {
                                "type": "OBJECT",
                                "properties": {"language": {"type": "STRING", "nullable": true}, "text": {"type": "STRING"}},
                                "required": ["language", "text"],
                            },
                        },
                    })),
            )?;
            response["candidates"][0]["content"]["parts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| p["thought"] != true)
                .filter_map(|p| p["text"].as_str())
                .collect::<String>()
        }
        Provider::OpenAi => {
            let mut messages = vec![json!({"role": "system", "content": system})];
            for (user, assistant) in examples {
                messages.push(json!({"role": "user", "content": user}));
                messages.push(json!({"role": "assistant", "content": assistant}));
            }
            messages.push(json!({"role": "user", "content": text}));
            let response = read(
                agent
                    .post("https://api.openai.com/v1/chat/completions")
                    .header("Authorization", format!("Bearer {key}"))
                    .send_json(json!({"model": model, "messages": messages, "response_format": {"type": "json_object"}})),
            )?;
            response["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string()
        }
    };
    serde_json::from_str(reply.trim()).with_context(|| format!("the reply wasn't JSON: {}", reply.chars().take(120).collect::<String>()))
}
