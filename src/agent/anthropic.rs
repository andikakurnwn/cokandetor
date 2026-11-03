//! Claude via the Messages API (raw HTTP + SSE streaming; Rust has no official SDK).

use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::tools::ToolSpec;
use super::{AgentEvent, ApiError, Emitter, Msg, ToolCall, TurnOutput, Usage};
use crate::config::Settings;

const API_VERSION: &str = "2026-10-21";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// Optional request features. Any the endpoint rejects with a 400 are switched
/// off for the rest of the run (useful for proxies and older models).
#[derive(Clone, Copy)]
pub struct Features {
    pub thinking: bool,
    pub effort: bool,
    pub fallback: bool,
    pub eager: bool,
    pub cache: bool,
}

impl Features {
    pub fn from_settings(s: &Settings) -> Self {
        Self {
            thinking: s.anthropic_thinking,
            effort: !s.anthropic_effort.trim().is_empty(),
            fallback: s.anthropic_fallback,
            eager: true,
            cache: true,
        }
    }

    /// Turn off whichever feature a 400 error message complains about.
    fn drop_for(&mut self, msg: &str) -> Option<&'static str> {
        let m = msg.to_lowercase();
        if self.fallback && m.contains("fallback") {
            self.fallback = false;
            return Some("fallbacks");
        }
        if self.eager && m.contains("eager_input_streaming") {
            self.eager = false;
            return Some("eager_input_streaming");
        }
        if self.cache && m.contains("cache_control") {
            self.cache = false;
            return Some("cache_control");
        }
        if self.thinking && m.contains("thinking") {
            self.thinking = false;
            return Some("thinking");
        }
        if self.effort && (m.contains("effort") || m.contains("output_config")) {
            self.effort = false;
            return Some("effort");
        }
        None
    }
}

fn to_messages(history: &[Msg]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for m in history {
        let (role, blocks): (&str, Vec<Value>) = match m {
            Msg::User { text } => ("user", vec![json!({"type": "text", "text": text})]),
            Msg::Assistant {
                text,
                calls,
                blocks,
                provider,
            } => {
                if provider == "anthropic" && !blocks.is_empty() {
                    // Empty text blocks and thinking cut off before its
                    // signature (e.g. at max_tokens) are rejected by the API.
                    let kept = blocks
                        .iter()
                        .filter(|b| match b["type"].as_str() {
                            Some("text") => !b["text"].as_str().unwrap_or_default().is_empty(),
                            Some("thinking") => !b["signature"].as_str().unwrap_or_default().is_empty(),
                            _ => true,
                        })
                        .cloned()
                        .collect();
                    ("assistant", kept)
                } else {
                    let mut b = Vec::new();
                    if !text.is_empty() {
                        b.push(json!({"type": "text", "text": text}));
                    }
                    for c in calls {
                        b.push(json!({"type": "tool_use", "id": c.id, "name": c.name, "input": c.input}));
                    }
                    ("assistant", b)
                }
            }
            Msg::Tools { results } => (
                "user",
                results
                    .iter()
                    .map(|r| {
                        json!({
                            "type": "tool_result",
                            "tool_use_id": r.id,
                            "content": r.content,
                            "is_error": r.is_error,
                        })
                    })
                    .collect(),
            ),
        };
        if blocks.is_empty() {
            continue;
        }
        if let Some(last) = out.last_mut()
            && last["role"] == role
                && let Some(arr) = last["content"].as_array_mut() {
                    arr.extend(blocks);
                    continue;
                }
        out.push(json!({"role": role, "content": blocks}));
    }
    out
}

fn build_body(s: &Settings, f: &Features, system: &str, history: &[Msg], specs: &[ToolSpec]) -> Value {
    let tools: Vec<Value> = specs
        .iter()
        .map(|t| {
            let mut v = json!({"name": t.name, "description": t.description, "input_schema": t.schema});
            if f.eager {
                // Stream file contents as they are generated so the editor can show them live.
                v["eager_input_streaming"] = json!(true);
            }
            v
        })
        .collect();
    let mut system = json!({"type": "text", "text": system});
    if f.cache {
        // Tools + system are the same for every task in a project: give them a
        // read point of their own, whatever happens later in the conversation.
        system["cache_control"] = json!({"type": "ephemeral"});
    }
    let mut body = json!({
        "model": s.anthropic_model.trim(),
        "max_tokens": s.max_tokens.max(1024),
        "stream": true,
        "system": [system],
        "tools": tools,
        "messages": to_messages(history),
    });
    if f.cache {
        // Automatic breakpoint at the end of the growing conversation.
        body["cache_control"] = json!({"type": "ephemeral"});
    }
    if f.thinking {
        body["thinking"] = json!({"type": "adaptive", "display": "summarized"});
    }
    if f.effort {
        body["output_config"] = json!({"effort": s.anthropic_effort.trim()});
    }
    if f.fallback {
        body["fallbacks"] = json!("default");
    }
    body
}

fn append(v: &mut Value, key: &str, s: &str) {
    match v.get_mut(key) {
        Some(Value::String(x)) => x.push_str(s),
        _ => v[key] = Value::String(s.to_string()),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn stream_turn(
    agent: &ureq::Agent,
    s: &Settings,
    features: &mut Features,
    system: &str,
    history: &[Msg],
    specs: &[ToolSpec],
    emit: &Emitter,
    cancel: &AtomicBool,
) -> Result<TurnOutput, ApiError> {
    let url = format!("{}/v1/messages", s.anthropic_base_url.trim().trim_end_matches('/'));
    let key = s.api_key();

    let resp = loop {
        let body = build_body(s, features, system, history, specs);
        let mut req = agent
            .post(&url)
            .set("x-api-key", &key)
            .set("anthropic-version", API_VERSION)
            .set("content-type", "application/json");
        if features.fallback {
            req = req.set("anthropic-beta", FALLBACK_BETA);
        }
        match req.send_string(&body.to_string()) {
            Ok(r) => break r,
            Err(ureq::Error::Status(400, r)) => {
                let text = r.into_string().unwrap_or_default();
                let err = ApiError::from_status(400, &text);
                if err.stale_thinking {
                    // Not an unsupported feature: the caller drops the old thinking.
                    return Err(err);
                }
                match features.drop_for(&err.message) {
                    Some(name) => emit.send(AgentEvent::Status(format!(
                        "Endpoint menolak '{name}', mencoba tanpa fitur itu…"
                    ))),
                    None => return Err(err),
                }
            }
            Err(ureq::Error::Status(code, r)) => {
                let text = r.into_string().unwrap_or_default();
                return Err(ApiError::from_status(code, &text));
            }
            Err(ureq::Error::Transport(t)) => {
                return Err(ApiError::retry(format!("Koneksi gagal: {t}")));
            }
        }
    };

    let reader = BufReader::new(resp.into_reader());
    let mut blocks: Vec<Value> = Vec::new();
    let mut json_bufs: HashMap<usize, String> = HashMap::new();
    let mut invalid: HashMap<usize, bool> = HashMap::new();
    let mut stop_reason = String::new();
    let mut finished = false;
    let mut last_emit = Instant::now();
    let mut usage = Usage {
        calls: 1,
        ..Usage::default()
    };

    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            return Err(ApiError::cancelled());
        }
        let line = line.map_err(|e| ApiError::retry(format!("Stream terputus: {e}")))?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let Ok(ev) = serde_json::from_str::<Value>(data.trim_start()) else {
            continue;
        };
        let idx = ev["index"].as_u64().unwrap_or(0) as usize;
        match ev["type"].as_str().unwrap_or("") {
            "message_start" => usage.merge_anthropic(&ev["message"]["usage"]),
            "content_block_start" => {
                let block = ev["content_block"].clone();
                if blocks.len() <= idx {
                    blocks.resize(idx + 1, Value::Null);
                }
                if block["type"] == "tool_use" {
                    json_bufs.insert(idx, String::new());
                    emit.send(AgentEvent::ToolStart {
                        id: block["id"].as_str().unwrap_or_default().to_string(),
                        name: block["name"].as_str().unwrap_or_default().to_string(),
                    });
                }
                blocks[idx] = block;
            }
            "content_block_delta" => {
                let d = &ev["delta"];
                let Some(block) = blocks.get_mut(idx) else { continue };
                match d["type"].as_str().unwrap_or("") {
                    "text_delta" => {
                        let t = d["text"].as_str().unwrap_or_default();
                        append(block, "text", t);
                        emit.send(AgentEvent::Text(t.to_string()));
                    }
                    "thinking_delta" => {
                        let t = d["thinking"].as_str().unwrap_or_default();
                        append(block, "thinking", t);
                        emit.send(AgentEvent::Thinking(t.to_string()));
                    }
                    "signature_delta" => {
                        block["signature"] = d["signature"].clone();
                    }
                    "input_json_delta" => {
                        let buf = json_bufs.entry(idx).or_default();
                        buf.push_str(d["partial_json"].as_str().unwrap_or_default());
                        if last_emit.elapsed() > Duration::from_millis(30) {
                            last_emit = Instant::now();
                            emit.send(AgentEvent::ToolInput {
                                id: block["id"].as_str().unwrap_or_default().to_string(),
                                name: block["name"].as_str().unwrap_or_default().to_string(),
                                partial: buf.clone(),
                            });
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let Some(block) = blocks.get_mut(idx) else { continue };
                if block["type"] == "tool_use" {
                    let raw = json_bufs.get(&idx).cloned().unwrap_or_default();
                    emit.send(AgentEvent::ToolInput {
                        id: block["id"].as_str().unwrap_or_default().to_string(),
                        name: block["name"].as_str().unwrap_or_default().to_string(),
                        partial: raw.clone(),
                    });
                    // Eager streaming skips server-side validation: parse strictly here.
                    let parsed = if raw.trim().is_empty() {
                        Some(json!({}))
                    } else {
                        serde_json::from_str::<Value>(&raw).ok().filter(Value::is_object)
                    };
                    invalid.insert(idx, parsed.is_none());
                    block["input"] = parsed.unwrap_or_else(|| json!({}));
                }
            }
            "message_delta" => {
                if let Some(sr) = ev["delta"]["stop_reason"].as_str() {
                    stop_reason = sr.to_string();
                }
                usage.merge_anthropic(&ev["usage"]);
            }
            "message_stop" => {
                finished = true;
                break;
            }
            "error" => {
                let kind = ev["error"]["type"].as_str().unwrap_or("error");
                let msg = ev["error"]["message"].as_str().unwrap_or("unknown error");
                let retryable = matches!(kind, "overloaded_error" | "api_error" | "rate_limit_error");
                return Err(ApiError {
                    message: format!("{kind}: {msg}"),
                    retryable,
                    cancelled: false,
                    stale_thinking: false,
                });
            }
            _ => {}
        }
    }
    if !finished {
        return Err(ApiError::retry("Stream berakhir sebelum selesai"));
    }

    let mut text = String::new();
    let mut calls = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        match b["type"].as_str() {
            Some("text") => text.push_str(b["text"].as_str().unwrap_or_default()),
            Some("tool_use") => calls.push(ToolCall {
                id: b["id"].as_str().unwrap_or_default().to_string(),
                name: b["name"].as_str().unwrap_or_default().to_string(),
                input: b["input"].clone(),
                raw: json_bufs.get(&i).cloned().unwrap_or_default(),
                invalid: invalid.get(&i).copied().unwrap_or(false),
            }),
            _ => {}
        }
    }
    blocks.retain(|b| !b.is_null());
    Ok(TurnOutput {
        text,
        calls,
        blocks,
        stop_reason,
        usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_drops_blocks_the_api_rejects() {
        let history = vec![
            Msg::User { text: "hi".into() },
            Msg::Assistant {
                text: String::new(),
                calls: vec![],
                blocks: vec![
                    json!({"type": "thinking", "thinking": "cut off", "signature": ""}),
                    json!({"type": "thinking", "thinking": "", "signature": "SIG"}),
                    json!({"type": "text", "text": ""}),
                    json!({"type": "text", "text": "ok"}),
                ],
                provider: "anthropic".into(),
            },
            Msg::User { text: "again".into() },
        ];
        let msgs = to_messages(&history);
        let blocks = msgs[1]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["signature"], "SIG");
        assert_eq!(blocks[1]["text"], "ok");
    }
}
