//! Persistent settings and the list of projects living in the pixel world.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderKind {
    Anthropic,
    OpenAI,
    Local,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 3] = [Self::Anthropic, Self::OpenAI, Self::Local];

    pub fn label(self) -> &'static str {
        match self {
            Self::Anthropic => "Claude (Anthropic)",
            Self::OpenAI => "GPT (OpenAI)",
            Self::Local => "Lokal / OpenAI-compatible (Ollama, LM Studio)",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Self::Anthropic => "Claude",
            Self::OpenAI => "GPT",
            Self::Local => "Lokal",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub provider: ProviderKind,

    pub anthropic_key: String,
    pub anthropic_model: String,
    pub anthropic_base_url: String,
    /// `output_config.effort`: low | medium | high | xhigh | max (empty = server default).
    pub anthropic_effort: String,
    /// Adaptive thinking with summarized display.
    pub anthropic_thinking: bool,
    /// Server-side refusal fallback (`fallbacks: "default"`).
    pub anthropic_fallback: bool,

    pub openai_key: String,
    pub openai_model: String,
    pub openai_base_url: String,

    pub local_key: String,
    pub local_model: String,
    pub local_base_url: String,

    pub max_tokens: u32,
    pub max_steps: u32,
    pub allow_commands: bool,
    pub follow_agent: bool,
    pub custom_instructions: String,
    pub editor_font_size: f32,
    /// Hours from UTC: 7 = WIB, 8 = WITA, 9 = WIT.
    pub tz_offset: i32,
    /// Pin the office clock to this hour (preview); `None` follows real time.
    pub time_preview: Option<f32>,
    /// Write `.ckdtr/skill.md` with the agent when a project is added.
    pub auto_skill: bool,
    /// Start every task with an empty conversation (cheapest; no memory of earlier tasks).
    pub fresh_context: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: ProviderKind::Anthropic,
            anthropic_key: String::new(),
            anthropic_model: "claude-opus-5-5".into(),
            anthropic_base_url: "https://api.anthropic.com".into(),
            anthropic_effort: "high".into(),
            anthropic_thinking: true,
            anthropic_fallback: true,
            openai_key: String::new(),
            openai_model: "gpt-5".into(),
            openai_base_url: "https://api.openai.com/v1".into(),
            local_key: String::new(),
            local_model: "qwen2.5-coder".into(),
            local_base_url: "http://localhost:11434/v1".into(),
            max_tokens: 64000,
            max_steps: 40,
            allow_commands: false,
            follow_agent: true,
            custom_instructions: String::new(),
            editor_font_size: 14.0,
            tz_offset: 7,
            time_preview: None,
            auto_skill: true,
            fresh_context: false,
        }
    }
}

impl Settings {
    /// API key for the active provider, falling back to the usual env vars.
    pub fn api_key(&self) -> String {
        let (key, env) = match self.provider {
            ProviderKind::Anthropic => (&self.anthropic_key, "ANTHROPIC_API_KEY"),
            ProviderKind::OpenAI => (&self.openai_key, "OPENAI_API_KEY"),
            ProviderKind::Local => (&self.local_key, ""),
        };
        if !key.trim().is_empty() {
            return key.trim().to_string();
        }
        if env.is_empty() {
            return String::new();
        }
        std::env::var(env).unwrap_or_default().trim().to_string()
    }

    pub fn model(&self) -> &str {
        match self.provider {
            ProviderKind::Anthropic => &self.anthropic_model,
            ProviderKind::OpenAI => &self.openai_model,
            ProviderKind::Local => &self.local_model,
        }
    }

    pub fn model_mut(&mut self) -> &mut String {
        match self.provider {
            ProviderKind::Anthropic => &mut self.anthropic_model,
            ProviderKind::OpenAI => &mut self.openai_model,
            ProviderKind::Local => &mut self.local_model,
        }
    }

    pub fn needs_key(&self) -> bool {
        self.provider != ProviderKind::Local
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub path: PathBuf,
    pub name: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub settings: Settings,
    pub projects: Vec<ProjectEntry>,
    /// Database connections, shown as storage cabinets in the office.
    pub databases: Vec<crate::db::DbConfig>,
}

pub fn app_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("COKANDETOR_HOME") {
        return PathBuf::from(home);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Cokandetor")
}

fn config_path() -> PathBuf {
    app_dir().join("config.json")
}

pub fn sessions_dir() -> PathBuf {
    app_dir().join("sessions")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        // Notepad and PowerShell like to prepend a UTF-8 BOM.
        match serde_json::from_str(text.trim_start_matches('\u{feff}')) {
            Ok(cfg) => cfg,
            Err(_) => {
                // Keep the unreadable file instead of overwriting it on the next save.
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let _ = std::fs::copy(&path, path.with_file_name(format!("config.rusak-{stamp}.json")));
                Self::default()
            }
        }
    }

    /// Writes atomically (temp file + rename) so a crash never leaves half a config.
    pub fn save(&self) {
        let path = config_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
}

/// Stable 64-bit FNV-1a hash, used for sprite seeds and session file names.
pub fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn session_path(project: &Path) -> PathBuf {
    let key = project.to_string_lossy().to_lowercase();
    sessions_dir().join(format!("{:016x}.json", fnv(&key)))
}
