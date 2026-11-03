//! The coding agent: a background thread that streams a model turn, executes
//! the requested tools against the project folder and loops until the model
//! is done. Everything the UI needs arrives as [`AgentEvent`]s.

pub mod anthropic;
pub mod openai;
pub mod tools;
pub mod usage;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use crate::config::{ProviderKind, Settings};
pub use tools::FileChange;
pub use usage::Usage;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
    /// The raw argument JSON as streamed.
    pub raw: String,
    /// The streamed arguments were not valid JSON.
    pub invalid: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolResult {
    pub id: String,
    pub name: String,
    pub content: String,
    pub is_error: bool,
}

/// Provider-neutral conversation history.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum Msg {
    User {
        text: String,
    },
    Assistant {
        text: String,
        calls: Vec<ToolCall>,
        /// Raw Anthropic content blocks (thinking blocks must be replayed unchanged).
        #[serde(default)]
        blocks: Vec<Value>,
        #[serde(default)]
        provider: String,
    },
    Tools {
        results: Vec<ToolResult>,
    },
}

pub enum AgentEvent {
    Status(String),
    TurnStart,
    /// The current turn failed and will be retried: drop what it streamed.
    TurnReset,
    Text(String),
    Thinking(String),
    ToolStart { id: String, name: String },
    ToolInput { id: String, name: String, partial: String },
    ToolEnd {
        id: String,
        name: String,
        input: Value,
        ok: bool,
        summary: String,
        change: Option<FileChange>,
    },
    Error(String),
    /// Tokens one model call consumed, and its rough cost in USD (`None` = unknown price).
    Usage { usage: Usage, cost: Option<f64> },
    /// The user pressed Stop.
    Stopped,
    Done { history: Vec<Msg> },
}

#[derive(Clone)]
pub struct Emitter {
    tx: Sender<AgentEvent>,
    ctx: egui::Context,
}

impl Emitter {
    pub fn send(&self, ev: AgentEvent) {
        let _ = self.tx.send(ev);
        self.ctx.request_repaint();
    }
}

pub struct TurnOutput {
    pub text: String,
    pub calls: Vec<ToolCall>,
    pub blocks: Vec<Value>,
    pub stop_reason: String,
    pub usage: Usage,
}

#[derive(Debug)]
pub struct ApiError {
    pub message: String,
    pub retryable: bool,
    pub cancelled: bool,
    /// Replayed thinking blocks no longer match the conversation (preserved thinking).
    pub stale_thinking: bool,
}

impl ApiError {
    pub fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
            cancelled: false,
            stale_thinking: false,
        }
    }

    pub fn retry(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
            cancelled: false,
            stale_thinking: false,
        }
    }

    pub fn cancelled() -> Self {
        Self {
            message: "Dihentikan oleh pengguna.".into(),
            retryable: false,
            cancelled: true,
            stale_thinking: false,
        }
    }

    pub fn from_status(code: u16, body: &str) -> Self {
        let msg = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|v| {
                v["error"]["message"]
                    .as_str()
                    .or_else(|| v["message"].as_str())
                    .or_else(|| v["error"].as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| body.chars().take(400).collect());
        let hint = match code {
            401 => " (API key salah atau belum diisi?)",
            403 => " (API key tidak punya akses ke model ini?)",
            404 => " (nama model atau base URL salah?)",
            429 => " (rate limit)",
            _ => "",
        };
        Self {
            message: format!("HTTP {code}: {msg}{hint}"),
            retryable: code == 429 || code == 408 || code >= 500,
            cancelled: false,
            stale_thinking: code == 400 && msg.contains("bound to a different conversation"),
        }
    }
}

pub fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(Duration::from_secs(600))
        .build()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// A normal task from the user.
    Task,
    /// Onboarding: explore the project and write `.ckdtr/skill.md`.
    Skill,
}

pub struct AgentRequest {
    pub settings: Settings,
    pub root: PathBuf,
    pub project_name: String,
    pub overview: String,
    pub history: Vec<Msg>,
    pub prompt: String,
    pub mode: Mode,
    /// Contents of `.ckdtr/skill.md`, if the project has one.
    pub skill: Option<String>,
    /// (file name, text) of CLAUDE.md / AGENTS.md found in the project.
    pub instructions: Vec<(String, String)>,
}

pub struct AgentHandle {
    pub rx: Receiver<AgentEvent>,
    pub cancel: Arc<AtomicBool>,
}

impl AgentHandle {
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub fn spawn(req: AgentRequest, ctx: egui::Context) -> AgentHandle {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let emit = Emitter { tx, ctx };
    let c = cancel.clone();
    std::thread::spawn(move || {
        let mut history = req.history.clone();
        run(&req, &mut history, &emit, &c);
        if c.load(Ordering::Relaxed) {
            emit.send(AgentEvent::Stopped);
        }
        emit.send(AgentEvent::Done { history });
    });
    AgentHandle { rx, cancel }
}

/// The system prompt holds only what stays fixed for a conversation: no file
/// counts or other live state (that goes in the task message), so the prompt
/// cache keeps hitting from one task to the next and replayed thinking stays valid.
fn system_prompt(req: &AgentRequest) -> String {
    let commands = if req.settings.allow_commands {
        "- You can run Windows shell commands with run_command (for example to build or run tests)."
    } else {
        "- Shell commands are disabled; you only have the file tools."
    };
    let mut s = format!(
        "You are Cokandetor Agent, a coding agent built into the Cokandetor desktop code editor.\n\
         You are working on the project \"{name}\" at {root}. All tool paths are relative to the project root; use forward slashes.\n\n\
         The user watches your file edits stream live into their editor, so:\n\
         - Read the relevant files before changing them.\n\
         - Prefer edit_file for targeted changes to existing files; use write_file for new files or full rewrites.\n\
         - Keep each write reasonably sized; split very large files across several steps.\n\
         {commands}\n\n\
         When the task is done, reply with a short summary of what you changed. Reply in the language the user writes in.\n\
         A task message may start with a <project_state> block: a snapshot of the project's files taken when the task was sent.\n",
        name = req.project_name,
        root = req.root.display(),
    );
    // The skill is the agent's map of the project: with it, a task can start
    // working straight away instead of exploring with tool calls first.
    if let (Mode::Task, Some(skill)) = (req.mode, &req.skill) {
        s.push_str(&format!(
            "\nProject skill ({}), written when this project joined the editor. Treat it as your map of the \
             project and read files only for the details a task needs:\n<skill>\n{skill}\n</skill>\n",
            crate::skill::REL
        ));
    }
    for (name, text) in &req.instructions {
        s.push_str(&format!("\nProject instructions from {name}:\n<instructions>\n{text}\n</instructions>\n"));
    }
    if !req.settings.custom_instructions.trim().is_empty() {
        s.push_str("\nAdditional instructions from the user:\n");
        s.push_str(req.settings.custom_instructions.trim());
        s.push('\n');
    }
    if req.mode == Mode::Skill {
        let t = crate::clock::now(req.settings.tz_offset, None);
        let today = format!("{:04}-{:02}-{:02}", t.year, t.month, t.day);
        s.push('\n');
        s.push_str(&crate::skill::writing_instructions(&req.project_name, &today));
    }
    s
}

/// Fingerprint of what a conversation's prefix is built from (system prompt and
/// tools). When it changes, the earlier turns can't be reused: their cache is
/// gone and the API rejects their replayed thinking.
pub fn context_key(req: &AgentRequest) -> u64 {
    let specs = tools::specs(req.settings.allow_commands);
    let mut key = system_prompt(req);
    for t in &specs {
        key.push_str(&format!("\n{} {} {}", t.name, t.description, t.schema));
    }
    crate::config::fnv(&key)
}

const STATE_OPEN: &str = "<project_state>\n";
const STATE_CLOSE: &str = "\n</project_state>";

/// The user's prompt, preceded by a snapshot of the project when it differs
/// from the last one this conversation saw.
fn task_message(prompt: &str, overview: &str, history: &[Msg]) -> String {
    let overview = overview.trim_end();
    let last = history.iter().rev().find_map(|m| match m {
        Msg::User { text } => text
            .strip_prefix(STATE_OPEN)
            .and_then(|t| t.split_once(STATE_CLOSE))
            .map(|(state, _)| state),
        _ => None,
    });
    if overview.is_empty() || last == Some(overview) {
        return prompt.to_string();
    }
    format!("{STATE_OPEN}{overview}{STATE_CLOSE}\n\n{prompt}")
}

/// Removes replayed thinking from earlier turns.
fn drop_thinking(history: &mut [Msg]) {
    for m in history {
        if let Msg::Assistant { blocks, .. } = m {
            blocks.retain(|b| !matches!(b["type"].as_str(), Some("thinking" | "redacted_thinking")));
        }
    }
}

fn run(req: &AgentRequest, history: &mut Vec<Msg>, emit: &Emitter, cancel: &AtomicBool) {
    let s = &req.settings;
    if s.needs_key() && s.api_key().is_empty() {
        emit.send(AgentEvent::Error(format!(
            "API key {} belum diisi. Buka ⚙ Pengaturan (atau set env var {}).",
            s.provider.short(),
            if s.provider == ProviderKind::Anthropic { "ANTHROPIC_API_KEY" } else { "OPENAI_API_KEY" }
        )));
        return;
    }

    let text = task_message(&req.prompt, &req.overview, history);
    history.push(Msg::User { text });
    let system = system_prompt(req);
    let mut specs = tools::specs(s.allow_commands);
    let mut max_steps = s.max_steps.max(1);
    if req.mode == Mode::Skill {
        // Writing a skill only needs to read the project and write one file.
        specs.retain(|t| matches!(t.name, "list_files" | "read_file" | "search_files" | "write_file"));
        max_steps = max_steps.min(30);
    }
    let agent = http_agent();
    let mut features = anthropic::Features::from_settings(s);
    let mut dropped_thinking = false;

    for step in 0..max_steps {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        emit.send(AgentEvent::Status(format!("Berpikir… (langkah {})", step + 1)));

        let mut attempt = 0;
        let turn = loop {
            emit.send(AgentEvent::TurnStart);
            let res = match s.provider {
                ProviderKind::Anthropic => {
                    anthropic::stream_turn(&agent, s, &mut features, &system, history, &specs, emit, cancel)
                }
                ProviderKind::OpenAI | ProviderKind::Local => {
                    openai::stream_turn(&agent, s, &system, history, &specs, emit, cancel)
                }
            };
            match res {
                Ok(t) => break t,
                Err(e) if e.cancelled => return,
                Err(e) if e.stale_thinking && !dropped_thinking => {
                    // The conversation was built with another prompt: the API can no
                    // longer verify its earlier reasoning, so continue without it.
                    dropped_thinking = true;
                    drop_thinking(history);
                    emit.send(AgentEvent::TurnReset);
                    emit.send(AgentEvent::Status(
                        "Penalaran lama tidak berlaku lagi; melanjutkan tanpanya…".into(),
                    ));
                }
                Err(e) if e.retryable && attempt < 4 => {
                    attempt += 1;
                    let wait = 2u64.pow(attempt);
                    emit.send(AgentEvent::TurnReset);
                    emit.send(AgentEvent::Status(format!(
                        "{} — mencoba ulang dalam {wait} dtk…",
                        e.message
                    )));
                    for _ in 0..wait * 10 {
                        if cancel.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                Err(e) => {
                    emit.send(AgentEvent::Error(e.message));
                    return;
                }
            }
        };

        emit.send(AgentEvent::Usage {
            usage: turn.usage,
            cost: usage::cost(s, &turn.usage),
        });
        let provider = match s.provider {
            ProviderKind::Anthropic => "anthropic",
            _ => "openai",
        };
        history.push(Msg::Assistant {
            text: turn.text.clone(),
            calls: turn.calls.clone(),
            blocks: turn.blocks.clone(),
            provider: provider.into(),
        });

        if turn.stop_reason == "refusal" {
            emit.send(AgentEvent::Error(
                "Model menolak melanjutkan permintaan ini (stop_reason: refusal).".into(),
            ));
            // Tool calls cut off by a refusal are never executed, but they still need results.
            if !turn.calls.is_empty() {
                history.push(Msg::Tools {
                    results: turn
                        .calls
                        .iter()
                        .map(|c| ToolResult {
                            id: c.id.clone(),
                            name: c.name.clone(),
                            content: "Not executed: the turn ended with a refusal.".into(),
                            is_error: true,
                        })
                        .collect(),
                });
            }
            return;
        }
        if turn.calls.is_empty() {
            if turn.stop_reason == "max_tokens" {
                emit.send(AgentEvent::Error(
                    "Jawaban terpotong (max_tokens). Naikkan Max tokens di Pengaturan.".into(),
                ));
            }
            return;
        }

        emit.send(AgentEvent::Status("Menjalankan tools…".into()));
        let mut results = Vec::new();
        for call in &turn.calls {
            let (content, is_error, summary, change) = if cancel.load(Ordering::Relaxed) {
                ("Not executed: cancelled by the user.".to_string(), true, "dibatalkan".to_string(), None)
            } else if turn.stop_reason == "max_tokens" {
                (
                    "Not executed: your output hit max_tokens before this tool input was complete. Split the work into smaller writes or edits.".to_string(),
                    true,
                    "terpotong (max_tokens)".to_string(),
                    None,
                )
            } else if call.invalid {
                let wrapped = serde_json::json!({ "INVALID_JSON": call.raw }).to_string();
                (wrapped, true, "input JSON tidak valid".to_string(), None)
            } else if req.mode == Mode::Skill && !crate::skill::allowed_in_skill_mode(&req.root, &call.name, &call.input) {
                (
                    format!("Not allowed while writing the project skill: only read files and write {}.", crate::skill::REL),
                    true,
                    "ditolak (mode skill)".to_string(),
                    None,
                )
            } else {
                let o = tools::execute(&req.root, &call.name, &call.input, s.allow_commands, cancel);
                (o.content, o.is_error, o.summary, o.change)
            };
            emit.send(AgentEvent::ToolEnd {
                id: call.id.clone(),
                name: call.name.clone(),
                input: call.input.clone(),
                ok: !is_error,
                summary,
                change,
            });
            results.push(ToolResult {
                id: call.id.clone(),
                name: call.name.clone(),
                content,
                is_error,
            });
        }
        history.push(Msg::Tools { results });
    }
    emit.send(AgentEvent::Error(format!(
        "Batas {max_steps} langkah tercapai. Kirim pesan lagi untuk melanjutkan."
    )));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(mode: Mode, skill: Option<&str>) -> AgentRequest {
        AgentRequest {
            settings: Settings::default(),
            root: PathBuf::from("C:/proj"),
            project_name: "toko".into(),
            overview: "3 files".into(),
            history: vec![],
            prompt: "x".into(),
            mode,
            skill: skill.map(str::to_string),
            instructions: vec![("CLAUDE.md".into(), "Pakai tab.".into())],
        }
    }

    #[test]
    fn task_prompt_carries_skill_and_instructions() {
        let p = system_prompt(&req(Mode::Task, Some("# toko\n## Ringkasan\nKasir")));
        assert!(p.contains("<skill>\n# toko"));
        assert!(p.contains("Project instructions from CLAUDE.md"));
        assert!(!p.contains(crate::skill::MODE_MARKER));
    }

    #[test]
    fn live_state_stays_out_of_the_system_prompt() {
        assert!(!system_prompt(&req(Mode::Task, None)).contains("3 files"));
        // The snapshot rides along with the task, and only when it changed.
        let first = task_message("tambah login", "3 files\n", &[]);
        assert_eq!(first, "<project_state>\n3 files\n</project_state>\n\ntambah login");
        let history = vec![Msg::User { text: first }];
        assert_eq!(task_message("lagi", "3 files", &history), "lagi");
        assert!(task_message("lagi", "4 files", &history).starts_with("<project_state>\n4 files"));
    }

    #[test]
    fn context_key_follows_prompt_and_tools() {
        let a = req(Mode::Task, Some("skill v1"));
        let mut b = req(Mode::Task, Some("skill v1"));
        b.overview = "99 files".into();
        b.prompt = "tugas lain".into();
        assert_eq!(context_key(&a), context_key(&b));
        assert_ne!(context_key(&a), context_key(&req(Mode::Task, Some("skill v2"))));
        let mut c = req(Mode::Task, Some("skill v1"));
        c.settings.allow_commands = true;
        assert_ne!(context_key(&a), context_key(&c));
    }

    #[test]
    fn stale_thinking_is_recognised_and_dropped() {
        let e = ApiError::from_status(
            400,
            r#"{"error":{"message":"messages.1.content.0: Invalid `signature` in `thinking` block. The block is bound to a different conversation."}}"#,
        );
        assert!(e.stale_thinking && !e.retryable);
        assert!(!ApiError::from_status(400, "thinking is not supported").stale_thinking);
        let mut history = vec![Msg::Assistant {
            text: "ok".into(),
            calls: vec![],
            blocks: vec![
                serde_json::json!({"type": "thinking", "thinking": "", "signature": "S"}),
                serde_json::json!({"type": "text", "text": "ok"}),
            ],
            provider: "anthropic".into(),
        }];
        drop_thinking(&mut history);
        let Msg::Assistant { blocks, .. } = &history[0] else { unreachable!() };
        assert_eq!(blocks.len(), 1);
    }

    #[test]
    fn skill_prompt_asks_for_the_skill_file() {
        let p = system_prompt(&req(Mode::Skill, None));
        assert!(p.contains(crate::skill::MODE_MARKER));
        assert!(p.contains(".ckdtr/skill.md"));
        assert!(p.contains("generated: 20"));
    }
}
