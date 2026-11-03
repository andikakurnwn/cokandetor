//! GPT and any OpenAI-compatible server (Ollama, LM Studio, vLLM…) via the
//! Chat Completions streaming API.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::tools::ToolSpec;
use super::{AgentEvent, ApiError, Emitter, Msg, ToolCall, TurnOutput, Usage};
use crate::config::{ProviderKind, Settings};

fn to_messages(system: &str, history: &[Msg]) -> Vec<Value> {
    let mut out = vec![json!({"role": "system", "content": system})];
    for m in history {
        match m {
            Msg::User { text } => out.push(json!({"role": "user", "content": text})),
            Msg::Assistant { text, calls, .. } => {
                let mut v = json!({"role": "assistant", "content": text});
                if !calls.is_empty() {
                    v["tool_calls"] = calls
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "type": "function",
                                "function": {"name": c.name, "arguments": c.input.to_string()},
                            })
                        })
                        .collect();
                }
                out.push(v);
            }
            Msg::Tools { results } => {
                for r in results {
                    out.push(json!({"role": "tool", "tool_call_id": r.id, "content": r.content}));
                }
            }
        }
    }
    out
}

#[derive(Default)]
struct PendingCall {
    id: String,
    name: String,
    args: String,
    announced: bool,
}

pub fn stream_turn(
    agent: &ureq::Agent,
    s: &Settings,
    system: &str,
    history: &[Msg],
    specs: &[ToolSpec],
    emit: &Emitter,
    cancel: &AtomicBool,
) -> Result<TurnOutput, ApiError> {
    let (base, model) = match s.provider {
        ProviderKind::Local => (&s.local_base_url, &s.local_model),
        _ => (&s.openai_base_url, &s.openai_model),
    };
    let url = format!("{}/chat/completions", base.trim().trim_end_matches('/'));
    let tools: Vec<Value> = specs
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {"name": t.name, "description": t.description, "parameters": t.schema},
            })
        })
        .collect();
    let mut body = json!({
        "model": model.trim(),
        "stream": true,
        "stream_options": {"include_usage": true},
        "messages": to_messages(system, history),
        "tools": tools,
        "tool_choice": "auto",
    });

    let key = s.api_key();
    let resp = loop {
        let mut req = agent.post(&url).set("content-type", "application/json");
        if !key.is_empty() {
            req = req.set("authorization", &format!("Bearer {key}"));
        }
        match req.send_string(&body.to_string()) {
            Ok(r) => break r,
            Err(ureq::Error::Status(code, r)) => {
                let text = r.into_string().unwrap_or_default();
                // Some compatible servers don't know usage reporting: go without it.
                if code == 400
                    && text.contains("stream_options")
                    && let Some(o) = body.as_object_mut()
                    && o.remove("stream_options").is_some()
                {
                    continue;
                }
                let mut e = ApiError::from_status(code, &text);
                if code == 400 && text.to_lowercase().contains("tool") {
                    e.message.push_str(" — model ini mungkin tidak mendukung tool calling.");
                }
                return Err(e);
            }
            Err(ureq::Error::Transport(t)) => {
                let hint = if s.provider == ProviderKind::Local {
                    " (apakah server lokal seperti Ollama sudah berjalan?)"
                } else {
                    ""
                };
                return Err(ApiError::retry(format!("Koneksi gagal: {t}{hint}")));
            }
        }
    };

    let reader = BufReader::new(resp.into_reader());
    let mut text = String::new();
    let mut pending: Vec<PendingCall> = Vec::new();
    let mut stop_reason = String::new();
    let mut last_emit = Instant::now();
    let mut done = false;
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
        let data = data.trim();
        if data == "[DONE]" {
            done = true;
            break;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if chunk.get("error").is_some_and(|e| !e.is_null()) {
            let msg = chunk["error"]["message"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| chunk["error"].to_string());
            return Err(ApiError::fatal(msg));
        }
        // With include_usage the totals arrive in a last chunk without choices.
        if let Some(u) = Usage::from_openai(&chunk["usage"]) {
            usage = u;
        }
        let choice = &chunk["choices"][0];
        let delta = &choice["delta"];
        if let Some(t) = delta["content"].as_str()
            && !t.is_empty() {
                text.push_str(t);
                emit.send(AgentEvent::Text(t.to_string()));
            }
        for key in ["reasoning_content", "reasoning"] {
            if let Some(t) = delta[key].as_str()
                && !t.is_empty() {
                    emit.send(AgentEvent::Thinking(t.to_string()));
                }
        }
        if let Some(tcs) = delta["tool_calls"].as_array() {
            for (pos, tc) in tcs.iter().enumerate() {
                let i = tc["index"].as_u64().map(|v| v as usize).unwrap_or(pos);
                if pending.len() <= i {
                    pending.resize_with(i + 1, PendingCall::default);
                }
                let p = &mut pending[i];
                if let Some(id) = tc["id"].as_str()
                    && !id.is_empty() {
                        p.id = id.to_string();
                    }
                if let Some(n) = tc["function"]["name"].as_str()
                    && p.name.is_empty() {
                        p.name = n.to_string();
                    }
                if let Some(a) = tc["function"]["arguments"].as_str() {
                    p.args.push_str(a);
                }
                if p.id.is_empty() {
                    p.id = format!("call_{i}_{}", history.len());
                }
                if !p.announced && !p.name.is_empty() {
                    p.announced = true;
                    emit.send(AgentEvent::ToolStart {
                        id: p.id.clone(),
                        name: p.name.clone(),
                    });
                }
                if p.announced && last_emit.elapsed() > Duration::from_millis(30) {
                    last_emit = Instant::now();
                    emit.send(AgentEvent::ToolInput {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        partial: p.args.clone(),
                    });
                }
            }
        }
        if let Some(fr) = choice["finish_reason"].as_str() {
            stop_reason = match fr {
                "tool_calls" | "function_call" => "tool_use",
                "length" => "max_tokens",
                "content_filter" => "refusal",
                _ => "end_turn",
            }
            .to_string();
        }
    }

    if !done && stop_reason.is_empty() {
        return Err(ApiError::retry("Stream berakhir sebelum selesai"));
    }

    let calls = pending
        .into_iter()
        .filter(|p| !p.name.is_empty())
        .map(|p| {
            emit.send(AgentEvent::ToolInput {
                id: p.id.clone(),
                name: p.name.clone(),
                partial: p.args.clone(),
            });
            let parsed = if p.args.trim().is_empty() {
                Some(json!({}))
            } else {
                serde_json::from_str::<Value>(&p.args).ok().filter(Value::is_object)
            };
            ToolCall {
                id: p.id,
                name: p.name,
                invalid: parsed.is_none(),
                input: parsed.unwrap_or_else(|| json!({})),
                raw: p.args,
            }
        })
        .collect();
    Ok(TurnOutput {
        text,
        calls,
        blocks: Vec::new(),
        stop_reason,
        usage,
    })
}
