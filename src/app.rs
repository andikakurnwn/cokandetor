//! Application state and the glue between the office, the editor, the
//! database cabinets, the agent threads and the file watcher.

use egui::{Align, Align2, Color32, Frame, Layout, Margin, Panel, RichText, Sense, Ui, vec2};
use notify::Watcher;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use crate::agent::{self, AgentEvent, AgentHandle, Msg, Usage, usage};
use crate::chat::{ChatItem, ToolStatus, tool_label};
use crate::config::{self, Config, ProjectEntry, ProviderKind};
use crate::db::{DbConfig, DbKind, DbState, SslMode};
use crate::editor::{self, EditorState, Live, norm_rel};
use crate::office::{self, OfficeUi};
use crate::partial_json;
use crate::project::{self, Scan};
use crate::sprite::{self, Look};
use crate::theme::{self, vs};
use crate::icons::{self, Icon};
use crate::{clock, dbview, skill};

pub enum Action {
    Open(usize),
    BackToWorld,
    Send { project: usize, prompt: String },
    Stop(usize),
    ClearChat(usize),
    /// Forget the agent's conversation but keep the chat log.
    NewContext(usize),
    Rescan(usize),
    Reveal(usize),
    Remove(usize),
    AddProject,
    NewProject,
    SaveConfig,
    Toast(String),
    OpenSettings,
    /// Editor shortcuts that act on the open project.
    NewFile,
    QuickOpen,
    RevealCurrent,
    OpenDb(usize),
    NewDb,
    EditDb(usize),
    RemoveDb(usize),
    /// (Re)build the project's `.ckdtr/skill.md` with the agent.
    MakeSkill(usize),
    /// Open `.ckdtr/skill.md` in the editor.
    OpenSkill(usize),
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Session {
    chat: Vec<ChatItem>,
    history: Vec<Msg>,
    /// [`agent::context_key`] the history was built with.
    context_key: u64,
    /// Tokens in the conversation's last call (what the next task resends).
    context_tokens: u64,
    usage: Usage,
    cost: f64,
    /// Some calls had no known price, so `cost` is a lower bound.
    cost_partial: bool,
}

pub struct Project {
    pub entry: ProjectEntry,
    pub look: Look,
    pub scan: Option<Scan>,
    scan_rx: Option<Receiver<Scan>>,
    rescan_at: Option<f64>,
    pub editor: EditorState,
    pub chat: Vec<ChatItem>,
    pub history: Vec<Msg>,
    pub agent: Option<AgentHandle>,
    pub status: String,
    /// Short text for the speech bubble in the world.
    pub activity: String,
    pub input: String,
    pub level_up_at: Option<f64>,
    pub done_at: Option<f64>,
    /// The running agent is writing the project skill, not doing a task.
    pub skill_run: bool,
    /// `.ckdtr/skill.md` exists.
    pub has_skill: bool,
    /// (generated date, description) from the skill's front matter.
    pub skill_info: Option<(String, String)>,
    turn_mark: usize,
    context_key: u64,
    pub context_tokens: u64,
    /// Token use over the project's lifetime and its rough cost in USD.
    pub usage_total: Usage,
    pub cost_total: f64,
    pub cost_partial: bool,
    /// The running task's use (`task_cost` is `None` once a price is unknown).
    task_usage: Usage,
    task_cost: Option<f64>,
}

impl Project {
    fn new(entry: ProjectEntry) -> Self {
        let session: Session = std::fs::read_to_string(config::session_path(&entry.path))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut chat = session.chat;
        for item in chat.iter_mut() {
            if let ChatItem::Tool { status, .. } = item
                && *status == ToolStatus::Running {
                    *status = ToolStatus::Failed;
                }
        }
        let mut p = Self {
            look: Look::from_name(&entry.name),
            entry,
            scan: None,
            scan_rx: None,
            rescan_at: None,
            editor: EditorState::default(),
            chat,
            history: session.history,
            agent: None,
            status: String::new(),
            activity: String::new(),
            input: String::new(),
            level_up_at: None,
            done_at: None,
            skill_run: false,
            has_skill: false,
            skill_info: None,
            turn_mark: 0,
            context_key: session.context_key,
            context_tokens: session.context_tokens,
            usage_total: session.usage,
            cost_total: session.cost,
            cost_partial: session.cost_partial,
            task_usage: Usage::default(),
            task_cost: Some(0.0),
        };
        p.start_scan();
        p
    }

    pub fn level(&self) -> u32 {
        project::level_for_lines(self.lines())
    }

    pub fn lines(&self) -> usize {
        self.scan.as_ref().map(|s| s.stats.lines).unwrap_or(0)
    }

    fn start_scan(&mut self) {
        let (tx, rx) = channel();
        let root = self.entry.path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(project::scan(&root));
        });
        self.scan_rx = Some(rx);
        self.rescan_at = None;
    }

    fn poll_scan(&mut self, now: f64) {
        if let Some(rx) = &self.scan_rx {
            match rx.try_recv() {
                Ok(scan) => {
                    let before = self.scan.as_ref().map(|s| project::level_for_lines(s.stats.lines));
                    let after = project::level_for_lines(scan.stats.lines);
                    if before.is_some_and(|b| after > b) {
                        self.level_up_at = Some(now);
                    }
                    self.scan = Some(scan);
                    self.refresh_skill();
                    self.scan_rx = None;
                }
                Err(TryRecvError::Disconnected) => self.scan_rx = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if self.scan_rx.is_none() && self.rescan_at.is_some_and(|t| now >= t) {
            self.start_scan();
        }
    }

    /// Re-reads `.ckdtr/skill.md` (cheap; called after scans and skill runs).
    fn refresh_skill(&mut self) {
        let text = skill::read(&self.entry.path);
        self.has_skill = text.is_some();
        self.skill_info = text.map(|t| {
            (
                skill::front_matter(&t, "generated").unwrap_or_default(),
                skill::front_matter(&t, "description").unwrap_or_default(),
            )
        });
    }

    fn schedule_rescan(&mut self, now: f64) {
        if self.rescan_at.is_none() {
            self.rescan_at = Some(now + 0.8);
        }
    }

    fn save_session(&self) {
        let s = Session {
            chat: self.chat.clone(),
            history: self.history.clone(),
            context_key: self.context_key,
            context_tokens: self.context_tokens,
            usage: self.usage_total,
            cost: self.cost_total,
            cost_partial: self.cost_partial,
        };
        let path = config::session_path(&self.entry.path);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string(&s) {
            let _ = std::fs::write(path, json);
        }
    }

    /// Starts the agent's conversation over; the chat log stays.
    fn new_context(&mut self, note: &str) {
        self.history.clear();
        self.context_tokens = 0;
        self.chat.push(ChatItem::Info(note.into()));
    }

    fn start_task_usage(&mut self) {
        self.task_usage = Usage::default();
        self.task_cost = Some(0.0);
    }

    fn poll_agent(&mut self, follow: bool, now: f64) {
        let mut events = Vec::new();
        let mut lost = false;
        if let Some(a) = &self.agent {
            loop {
                match a.rx.try_recv() {
                    Ok(ev) => events.push(ev),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        lost = true;
                        break;
                    }
                }
            }
        }
        for ev in events {
            self.on_event(ev, follow, now);
        }
        // The thread ended without its Done event (it panicked).
        if lost && self.agent.is_some() {
            self.on_event(AgentEvent::Error("Agent berhenti tak terduga.".into()), follow, now);
            let history = self.history.clone();
            self.on_event(AgentEvent::Done { history }, follow, now);
        }
    }

    fn on_event(&mut self, ev: AgentEvent, follow: bool, now: f64) {
        match ev {
            AgentEvent::Status(s) => self.status = s,
            AgentEvent::TurnStart => self.turn_mark = self.chat.len(),
            AgentEvent::TurnReset => {
                self.chat.truncate(self.turn_mark);
                for t in &mut self.editor.tabs {
                    t.live = None;
                }
            }
            AgentEvent::Text(t) => {
                self.activity = "menjelaskan".into();
                match self.chat.last_mut() {
                    Some(ChatItem::Assistant(s)) => s.push_str(&t),
                    _ => self.chat.push(ChatItem::Assistant(t)),
                }
            }
            AgentEvent::Thinking(t) => {
                self.activity = "berpikir".into();
                match self.chat.last_mut() {
                    Some(ChatItem::Thinking(s)) => s.push_str(&t),
                    _ => self.chat.push(ChatItem::Thinking(t)),
                }
            }
            AgentEvent::ToolStart { id, name } => {
                let (icon, verb) = tool_label(&name);
                self.activity = format!("{icon} {verb}");
                self.chat.push(ChatItem::Tool {
                    id,
                    name,
                    path: String::new(),
                    status: ToolStatus::Running,
                    summary: String::new(),
                });
            }
            AgentEvent::ToolInput { id, name, partial } => self.on_tool_input(&id, &name, &partial, follow),
            AgentEvent::ToolEnd {
                id,
                name,
                input,
                ok,
                summary,
                change,
            } => {
                let root = self.entry.path.clone();
                let path = input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(|p| match crate::agent::tools::resolve(&root, p) {
                        Ok(abs) => crate::agent::tools::rel_display(&root, &abs),
                        Err(_) => norm_rel(p),
                    })
                    .unwrap_or_default();
                let shown = if name == "run_command" {
                    input.get("command").and_then(|v| v.as_str()).unwrap_or("").to_string()
                } else if name == "search_files" {
                    input.get("query").and_then(|v| v.as_str()).map(|q| format!("\"{q}\"")).unwrap_or_default()
                } else {
                    path.clone()
                };
                if let Some(ChatItem::Tool {
                    path: p,
                    status,
                    summary: s,
                    ..
                }) = self.chat.iter_mut().rev().find(|c| matches!(c, ChatItem::Tool { id: i, .. } if *i == id))
                {
                    *p = shown;
                    *status = if ok { ToolStatus::Ok } else { ToolStatus::Failed };
                    *s = summary;
                }
                // A failed write leaves the live preview behind: drop it.
                for t in &mut self.editor.tabs {
                    if t.live.as_ref().is_some_and(|l| l.tool_id == id) {
                        t.live = None;
                    }
                }
                if let Some(ch) = change {
                    self.editor.apply_change(&ch.path, ch.after.as_deref(), now);
                    if follow && ch.after.is_some()
                        && let Some(i) = self.editor.find(&norm_rel(&ch.path)) {
                            self.editor.active = Some(i);
                        }
                    self.schedule_rescan(now);
                }
            }
            AgentEvent::Error(e) => self.chat.push(ChatItem::Error(e)),
            AgentEvent::Usage { usage, cost } => {
                self.task_usage.add(&usage);
                self.task_cost = self.task_cost.zip(cost).map(|(a, b)| a + b);
                self.usage_total.add(&usage);
                match cost {
                    Some(c) => self.cost_total += c,
                    None => self.cost_partial = true,
                }
                if !self.skill_run {
                    self.context_tokens = usage.prompt() + usage.output;
                }
            }
            AgentEvent::Stopped => self.chat.push(ChatItem::Info("Dihentikan oleh pengguna.".into())),
            AgentEvent::Done { history } => {
                // Tools cut off by Stop or an error never report back.
                for item in self.chat.iter_mut() {
                    if let ChatItem::Tool { status, summary, .. } = item
                        && *status == ToolStatus::Running {
                            *status = ToolStatus::Failed;
                            *summary = "tidak selesai".into();
                        }
                }
                if self.skill_run {
                    // Onboarding has its own throwaway conversation.
                    self.skill_run = false;
                    self.refresh_skill();
                    if self.has_skill {
                        self.chat.push(ChatItem::Info(format!(
                            "Skill project tersimpan di {} — dipakai otomatis di setiap tugas.",
                            skill::REL
                        )));
                    }
                } else {
                    self.history = history;
                }
                if self.task_usage.calls > 0 {
                    self.chat.push(ChatItem::Info(format!(
                        "Pemakaian: {} panggilan model · {}",
                        self.task_usage.calls,
                        usage::summary(&self.task_usage, self.task_cost)
                    )));
                }
                self.agent = None;
                self.status.clear();
                self.activity.clear();
                self.done_at = Some(now);
                for t in &mut self.editor.tabs {
                    t.live = None;
                }
                self.schedule_rescan(now);
                self.save_session();
            }
        }
    }

    /// Streams partially generated file content into the editor.
    fn on_tool_input(&mut self, id: &str, name: &str, partial: &str, follow: bool) {
        let fields = partial_json::string_fields(partial);
        let Some(path) = partial_json::get(&fields, "path").filter(|f| f.complete) else {
            return;
        };
        // Same normalisation as the tool itself; skip paths it would refuse.
        let Ok(abs) = crate::agent::tools::resolve(&self.entry.path, &path.value) else {
            return;
        };
        let rel = crate::agent::tools::rel_display(&self.entry.path, &abs);
        if rel.is_empty() {
            return;
        }
        if let Some(ChatItem::Tool { path: p, .. }) =
            self.chat.iter_mut().rev().find(|c| matches!(c, ChatItem::Tool { id: i, .. } if i == id))
            && p.is_empty() {
                *p = rel.clone();
            }
        let (icon, verb) = tool_label(name);
        self.activity = format!("{icon} {}", rel.rsplit('/').next().unwrap_or(&rel));
        self.status = format!("{verb} {rel}…");

        let root = self.entry.path.clone();
        match name {
            "write_file" => {
                let content = partial_json::get(&fields, "content")
                    .map(|f| f.value.clone())
                    .unwrap_or_default();
                if !follow && self.editor.find(&rel).is_none() {
                    return;
                }
                let Some(i) = self.editor.open(&root, &rel, follow) else { return };
                let caret = content.len();
                self.editor.tabs[i].live = Some(Live {
                    tool_id: id.to_string(),
                    content,
                    caret,
                    verb,
                });
            }
            "edit_file" => {
                let (Some(old), Some(new)) = (
                    partial_json::get(&fields, "old_text").filter(|f| f.complete),
                    partial_json::get(&fields, "new_text"),
                ) else {
                    return;
                };
                if old.value.is_empty() || (!follow && self.editor.find(&rel).is_none()) {
                    return;
                }
                let Some(i) = self.editor.open(&root, &rel, follow) else { return };
                let base = self.editor.tabs[i].saved.clone();
                let (old_s, new_s) = if !base.contains(&old.value) && base.contains("\r\n") {
                    (old.value.replace('\n', "\r\n"), new.value.replace('\n', "\r\n"))
                } else {
                    (old.value.clone(), new.value.clone())
                };
                let Some(at) = base.find(&old_s) else { return };
                let mut content = String::with_capacity(base.len() + new_s.len());
                content.push_str(&base[..at]);
                content.push_str(&new_s);
                let caret = content.len();
                content.push_str(&base[at + old_s.len()..]);
                self.editor.tabs[i].live = Some(Live {
                    tool_id: id.to_string(),
                    content,
                    caret,
                    verb,
                });
            }
            _ => {}
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
enum View {
    World,
    Project(usize),
    Database(usize),
}

struct NewProjectDialog {
    name: String,
    parent: Option<PathBuf>,
}

/// Editing (or creating, when `index` is None) a database cabinet.
struct DbDialog {
    index: Option<usize>,
    cfg: DbConfig,
    show_password: bool,
}

pub struct App {
    config: Config,
    projects: Vec<Project>,
    dbs: Vec<DbState>,
    view: View,
    office: OfficeUi,
    show_settings: bool,
    show_keys: bool,
    new_project: Option<NewProjectDialog>,
    db_dialog: Option<DbDialog>,
    toasts: Vec<(String, f64)>,
    watcher: Option<notify::RecommendedWatcher>,
    fs_rx: Receiver<notify::Event>,
    ctx: egui::Context,
    /// Whether a text field had focus at the end of the previous frame.
    typing_last_frame: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        // Loading the grammar set takes a moment; do it off the UI thread.
        std::thread::spawn(|| {
            theme::syntect();
        });
        let config = Config::load();
        let (tx, fs_rx) = channel();
        let repaint = cc.egui_ctx.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                let _ = tx.send(ev);
                repaint.request_repaint();
            }
        })
        .ok();
        let dbs = config.databases.iter().cloned().map(DbState::new).collect();
        let mut app = Self {
            projects: Vec::new(),
            dbs,
            view: View::World,
            office: OfficeUi::default(),
            show_settings: false,
            show_keys: false,
            new_project: None,
            db_dialog: None,
            toasts: Vec::new(),
            watcher,
            fs_rx,
            ctx: cc.egui_ctx.clone(),
            typing_last_frame: false,
            config,
        };
        let entries = app.config.projects.clone();
        for e in entries {
            if e.path.is_dir() {
                app.attach(e);
            }
        }
        app
    }

    fn attach(&mut self, entry: ProjectEntry) {
        if let Some(w) = &mut self.watcher {
            let _ = w.watch(&entry.path, notify::RecursiveMode::Recursive);
        }
        self.projects.push(Project::new(entry));
    }

    fn add_folder(&mut self, path: PathBuf) {
        let path = std::fs::canonicalize(&path).map(strip_verbatim).unwrap_or(path);
        if self.projects.iter().any(|p| same_path(&p.entry.path, &path)) {
            self.toast("Project itu sudah punya meja di kantor.");
            return;
        }
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let entry = ProjectEntry { path, name: name.clone() };
        self.config.projects.push(entry.clone());
        self.config.save();
        self.attach(entry);
        self.office.target = self.projects.len() - 1;
        // A new employee onboards: learn the project and write its skill.
        let idx = self.projects.len() - 1;
        if self.config.settings.auto_skill && self.ai_ready() && !skill::exists(&self.projects[idx].entry.path) {
            self.make_skill(idx);
        }
        self.toast(format!("✨ {name} mulai bekerja di kantor!"));
    }

    fn save_dbs(&mut self) {
        self.config.databases = self.dbs.iter().map(|d| d.cfg.clone()).collect();
        self.config.save();
    }

    fn toast(&mut self, msg: impl Into<String>) {
        let now = self.ctx.input(|i| i.time);
        self.toasts.push((msg.into(), now));
    }

    fn clock_label(&self) -> String {
        let s = &self.config.settings;
        let t = clock::now(s.tz_offset, s.time_preview);
        format!("{} {}", t.hm(), clock::zone_label(s.tz_offset))
    }

    fn poll(&mut self, now: f64) {
        let follow = self.config.settings.follow_agent;
        for p in &mut self.projects {
            p.poll_scan(now);
            p.poll_agent(follow, now);
        }
        for d in &mut self.dbs {
            d.poll();
        }
        while let Ok(ev) = self.fs_rx.try_recv() {
            if matches!(ev.kind, notify::EventKind::Access(_)) {
                continue;
            }
            for path in &ev.paths {
                let Some(p) = self
                    .projects
                    .iter_mut()
                    .filter(|p| path.starts_with(&p.entry.path))
                    .max_by_key(|p| p.entry.path.components().count())
                else {
                    continue;
                };
                let inside = path.strip_prefix(&p.entry.path).unwrap_or(path);
                if inside.components().any(|c| project::is_junk_dir(c.as_os_str())) {
                    continue;
                }
                p.schedule_rescan(now);
                if let Ok(rel) = path.strip_prefix(&p.entry.path) {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    let root = p.entry.path.clone();
                    p.editor.reload_from_disk(&root, &rel, now);
                }
            }
        }
    }

    fn send(&mut self, idx: usize, prompt: String) {
        let ctx = self.ctx.clone();
        let settings = self.config.settings.clone();
        let fresh = settings.fresh_context;
        let Some(p) = self.projects.get_mut(idx) else { return };
        if p.agent.is_some() || prompt.is_empty() {
            return;
        }
        let (saved, skipped) = p.editor.save_all(&p.entry.path);
        if saved > 0 {
            p.chat.push(ChatItem::Info(format!("{saved} file disimpan sebelum agent mulai.")));
        }
        if skipped > 0 {
            p.chat.push(ChatItem::Info(format!(
                "{skipped} file berkonflik/terhapus tidak disimpan — agent melihat versi di disk."
            )));
        }
        let overview = p.scan.as_ref().map(|s| project::overview(s, 80)).unwrap_or_default();
        let mut req = agent::AgentRequest {
            settings,
            root: p.entry.path.clone(),
            project_name: p.entry.name.clone(),
            overview,
            history: Vec::new(),
            prompt: prompt.clone(),
            mode: agent::Mode::Task,
            skill: skill::read(&p.entry.path),
            instructions: skill::instructions(&p.entry.path),
        };
        // Earlier turns are only worth resending while the prompt they were
        // built on still holds; otherwise they cost full price (and the API
        // rejects their thinking).
        let key = agent::context_key(&req);
        if !p.history.is_empty() {
            if fresh {
                p.new_context("Konteks baru: riwayat tugas sebelumnya tidak dikirim ulang.");
            } else if key != p.context_key {
                p.new_context(
                    "Skill, instruksi, atau pengaturan agent berubah — memulai konteks baru \
                     (riwayat lama tidak bisa memakai cache lagi).",
                );
            }
        }
        p.context_key = key;
        req.history = p.history.clone();
        p.chat.push(ChatItem::User(prompt));
        p.start_task_usage();
        p.status = "Memulai…".into();
        p.activity = "bangun & bersiap".into();
        p.done_at = None;
        p.skill_run = false;
        p.agent = Some(agent::spawn(req, ctx));
    }

    /// Is an AI provider usable right now (a key, or a local server)?
    fn ai_ready(&self) -> bool {
        let s = &self.config.settings;
        !s.needs_key() || !s.api_key().is_empty()
    }

    /// Has the agent explore the project and write `.ckdtr/skill.md`.
    fn make_skill(&mut self, idx: usize) {
        let ctx = self.ctx.clone();
        let settings = self.config.settings.clone();
        let ready = self.ai_ready();
        let Some(busy) = self.projects.get(idx).map(|p| p.agent.is_some()) else { return };
        if busy {
            self.toast("Karyawan ini sedang bekerja; coba lagi setelah tugasnya selesai.");
            return;
        }
        if !ready {
            self.toast("Hubungkan AI dulu (isi API key di Pengaturan) untuk membuat skill.");
            return;
        }
        let p = &mut self.projects[idx];
        p.chat.push(ChatItem::Info(format!(
            "Mempelajari project untuk membuat skill {} …",
            skill::REL
        )));
        let overview = p.scan.as_ref().map(|s| project::overview(s, 80)).unwrap_or_default();
        let req = agent::AgentRequest {
            settings,
            root: p.entry.path.clone(),
            project_name: p.entry.name.clone(),
            overview,
            history: Vec::new(),
            prompt: skill::TASK.into(),
            mode: agent::Mode::Skill,
            skill: None,
            instructions: skill::instructions(&p.entry.path),
        };
        p.start_task_usage();
        p.status = "Mempelajari project…".into();
        p.activity = "📖 mempelajari project".into();
        p.done_at = None;
        p.skill_run = true;
        p.agent = Some(agent::spawn(req, ctx));
    }

    fn current_project(&self) -> Option<usize> {
        match self.view {
            View::Project(i) => Some(i),
            _ => None,
        }
    }

    fn handle(&mut self, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Open(i) if i < self.projects.len() => self.view = View::Project(i),
                Action::BackToWorld => self.view = View::World,
                Action::Send { project, prompt } => self.send(project, prompt),
                Action::Stop(i) => {
                    if let Some(a) = self.projects.get(i).and_then(|p| p.agent.as_ref()) {
                        a.stop();
                    }
                }
                Action::ClearChat(i) => {
                    if let Some(p) = self.projects.get_mut(i) {
                        p.chat.clear();
                        p.history.clear();
                        p.context_tokens = 0;
                        p.save_session();
                    }
                }
                Action::NewContext(i) => {
                    if let Some(p) = self.projects.get_mut(i).filter(|p| p.agent.is_none()) {
                        p.new_context("Konteks baru dimulai — tugas berikutnya tidak mengirim ulang riwayat lama.");
                        p.save_session();
                    }
                }
                Action::Rescan(i) => {
                    if let Some(p) = self.projects.get_mut(i) {
                        p.start_scan();
                    }
                }
                Action::Reveal(i) => {
                    if let Some(p) = self.projects.get(i) {
                        let _ = std::process::Command::new("explorer").arg(&p.entry.path).spawn();
                    }
                }
                Action::RevealCurrent => {
                    if let Some(i) = self.current_project() {
                        let _ = std::process::Command::new("explorer").arg(&self.projects[i].entry.path).spawn();
                    }
                }
                Action::NewFile => {
                    if let Some(i) = self.current_project() {
                        let e = &mut self.projects[i].editor;
                        e.side = editor::SideView::Explorer;
                        e.side_open = true;
                        e.new_file = Some(String::new());
                    }
                }
                Action::QuickOpen => {
                    if let Some(i) = self.current_project() {
                        self.projects[i].editor.quick = Some(editor::QuickOpen { query: String::new(), sel: 0 });
                    }
                }
                Action::Remove(i) if i < self.projects.len() => {
                    if let Some(a) = &self.projects[i].agent {
                        a.stop();
                    }
                    let p = self.projects.remove(i);
                    if let Some(w) = &mut self.watcher {
                        let _ = w.unwatch(&p.entry.path);
                    }
                    self.config.projects.retain(|e| !same_path(&e.path, &p.entry.path));
                    self.config.save();
                    self.view = View::World;
                    self.office.target = 0;
                    self.toast(format!("{} meninggalkan kantor (folder tidak dihapus).", p.entry.name));
                }
                Action::AddProject => {
                    if let Some(dir) = rfd::FileDialog::new().set_title("Pilih folder project").pick_folder() {
                        self.add_folder(dir);
                    }
                }
                Action::NewProject => {
                    self.new_project = Some(NewProjectDialog {
                        name: String::new(),
                        parent: dirs::document_dir(),
                    });
                }
                Action::OpenDb(i) if i < self.dbs.len() => {
                    self.view = View::Database(i);
                    if self.dbs[i].status == crate::db::Status::Idle {
                        let ctx = self.ctx.clone();
                        self.dbs[i].connect(&ctx);
                    }
                }
                Action::NewDb => {
                    self.db_dialog = Some(DbDialog {
                        index: None,
                        cfg: DbConfig {
                            name: format!("Lemari {}", self.dbs.len() + 1),
                            ..Default::default()
                        },
                        show_password: false,
                    });
                }
                Action::EditDb(i) if i < self.dbs.len() => {
                    self.db_dialog = Some(DbDialog {
                        index: Some(i),
                        cfg: self.dbs[i].cfg.clone(),
                        show_password: false,
                    });
                }
                Action::RemoveDb(i) if i < self.dbs.len() => {
                    let d = self.dbs.remove(i);
                    if let Some(dlg) = &mut self.db_dialog {
                        match dlg.index {
                            Some(j) if j == i => self.db_dialog = None,
                            Some(j) if j > i => dlg.index = Some(j - 1),
                            _ => {}
                        }
                    }
                    self.save_dbs();
                    if matches!(self.view, View::Database(_)) {
                        self.view = View::World;
                    }
                    self.toast(format!("Lemari \"{}\" disingkirkan (database-nya tidak disentuh).", d.cfg.name));
                }
                Action::MakeSkill(i) => self.make_skill(i),
                Action::OpenSkill(i) if i < self.projects.len() => {
                    let root = self.projects[i].entry.path.clone();
                    if skill::exists(&root) {
                        self.view = View::Project(i);
                        let ed = &mut self.projects[i].editor;
                        ed.open(&root, skill::REL, true);
                    } else {
                        self.toast("Project ini belum punya skill — buat dulu.");
                    }
                }
                Action::OpenSettings => self.show_settings = true,
                Action::SaveConfig => self.config.save(),
                Action::Toast(t) => self.toast(t),
                _ => {}
            }
        }
    }

    fn top_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        Panel::top("topbar")
            .frame(Frame::new().fill(vs::TITLE_BG).inner_margin(Margin::symmetric(10, 4)))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    let now = ui.input(|i| i.time);
                    let logo = Look::from_name("cokandetor");
                    sprite::draw(
                        &ui.painter_at(rect.expand(4.0)),
                        &logo,
                        &sprite::DrawOpts {
                            bottom_center: rect.center_bottom(),
                            px: 1.4,
                            flip: false,
                            squash: 1.0,
                            blink: (now % 3.5) < 0.12,
                            walk_frame: false,
                            working: false,
                            level: 1,
                            time: now,
                            alpha: 1.0,
                        },
                    );
                    ui.label(RichText::new("COKANDETOR").monospace().strong().size(14.0).color(theme::ACCENT));
                    ui.add_space(6.0);
                    match self.view {
                        View::World => {
                            if icons::button(ui, Icon::FolderPlus, "Tambah Folder").clicked() {
                                actions.push(Action::AddProject);
                            }
                            if icons::button(ui, Icon::Sparkle, "Project Baru").clicked() {
                                actions.push(Action::NewProject);
                            }
                            if icons::button(ui, Icon::Database, "Lemari DB Baru").clicked() {
                                actions.push(Action::NewDb);
                            }
                        }
                        View::Project(_) | View::Database(_) => {
                            if icons::button(ui, Icon::Home, "Kantor").clicked() {
                                actions.push(Action::BackToWorld);
                            }
                        }
                    }
                    let title = match self.view {
                        View::World => format!("Kantor — {} karyawan · {} lemari", self.projects.len(), self.dbs.len()),
                        View::Project(i) => self
                            .projects
                            .get(i)
                            .map(|p| format!("{} — Lv {} — Cokandetor", p.entry.name, p.level()))
                            .unwrap_or_default(),
                        View::Database(i) => self.dbs.get(i).map(|d| format!("{} — Database — Cokandetor", d.cfg.name)).unwrap_or_default(),
                    };
                    let center = ui.max_rect().center().x;
                    let g = ui.painter().layout_no_wrap(title, egui::FontId::proportional(13.0), vs::TEXT);
                    let pos = egui::pos2(center - g.size().x / 2.0, ui.max_rect().center().y - g.size().y / 2.0);
                    if pos.x > ui.cursor().left() + 20.0 {
                        ui.painter().galley(pos, g, vs::TEXT);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icons::icon_button(ui, Icon::Gear, "Pengaturan").clicked() {
                            self.show_settings = !self.show_settings;
                        }
                        let s = &self.config.settings;
                        let key_ok = !s.needs_key() || !s.api_key().is_empty();
                        let dot = if key_ok { theme::OK } else { theme::DANGER };
                        ui.label(RichText::new(format!("{} · {}", s.provider.short(), s.model())).small().color(vs::TEXT))
                            .on_hover_text(if key_ok { "Siap terhubung" } else { "API key belum diisi" });
                        ui.label(RichText::new("●").color(dot));
                        let busy = self.projects.iter().filter(|p| p.agent.is_some()).count();
                        if busy > 0 {
                            ui.label(RichText::new(format!("⚡ {busy} karyawan bekerja")).color(theme::LIVE).small());
                        }
                    });
                });
            });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = self.show_settings;
        let before = serde_json::to_string(&self.config.settings).unwrap_or_default();
        egui::Window::new("⚙ Pengaturan")
            .open(&mut open)
            .default_width(480.0)
            .collapsible(false)
            .show(ctx, |ui| {
                let s = &mut self.config.settings;
                ui.label(RichText::new("Provider AI").strong());
                for k in ProviderKind::ALL {
                    ui.radio_value(&mut s.provider, k, k.label());
                }
                ui.checkbox(&mut self.show_keys, "Tampilkan API key");
                ui.add_space(6.0);
                egui::Grid::new("settings_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    let key_edit = |ui: &mut Ui, v: &mut String, hint: &str, show: bool| {
                        ui.add(egui::TextEdit::singleline(v).password(!show).hint_text(hint).desired_width(300.0));
                    };
                    match s.provider {
                        ProviderKind::Anthropic => {
                            ui.label("API key");
                            key_edit(ui, &mut s.anthropic_key, "sk-ant-… (atau env ANTHROPIC_API_KEY)", self.show_keys);
                            ui.end_row();
                            ui.label("Model");
                            ui.text_edit_singleline(&mut s.anthropic_model);
                            ui.end_row();
                            ui.label("Base URL");
                            ui.text_edit_singleline(&mut s.anthropic_base_url);
                            ui.end_row();
                            ui.label("Effort");
                            egui::ComboBox::from_id_salt("effort")
                                .selected_text(if s.anthropic_effort.is_empty() { "default" } else { &s.anthropic_effort })
                                .show_ui(ui, |ui| {
                                    for e in ["", "low", "medium", "high", "xhigh", "max"] {
                                        ui.selectable_value(&mut s.anthropic_effort, e.to_string(), if e.is_empty() { "default" } else { e });
                                    }
                                });
                            ui.end_row();
                            ui.label("Thinking");
                            ui.checkbox(&mut s.anthropic_thinking, "Adaptive thinking (ringkasan tampil di chat)");
                            ui.end_row();
                            ui.label("Fallback");
                            ui.checkbox(&mut s.anthropic_fallback, "Refusal fallback server-side (fallbacks: \"default\")");
                            ui.end_row();
                        }
                        ProviderKind::OpenAI => {
                            ui.label("API key");
                            key_edit(ui, &mut s.openai_key, "sk-… (atau env OPENAI_API_KEY)", self.show_keys);
                            ui.end_row();
                            ui.label("Model");
                            ui.text_edit_singleline(&mut s.openai_model);
                            ui.end_row();
                            ui.label("Base URL");
                            ui.text_edit_singleline(&mut s.openai_base_url);
                            ui.end_row();
                        }
                        ProviderKind::Local => {
                            ui.label("Base URL");
                            ui.text_edit_singleline(&mut s.local_base_url);
                            ui.end_row();
                            ui.label("Model");
                            ui.text_edit_singleline(&mut s.local_model);
                            ui.end_row();
                            ui.label("API key");
                            key_edit(ui, &mut s.local_key, "(opsional)", self.show_keys);
                            ui.end_row();
                        }
                    }
                });
                if s.provider == ProviderKind::Local {
                    ui.label(
                        RichText::new("Mode offline penuh: jalankan Ollama/LM Studio lalu pilih model yang mendukung tool calling.")
                            .small()
                            .weak(),
                    );
                }
                ui.separator();
                ui.label(RichText::new("Kantor").strong());
                egui::Grid::new("office_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Zona waktu");
                    egui::ComboBox::from_id_salt("tz")
                        .selected_text(match s.tz_offset {
                            8 => "WITA (UTC+8)",
                            9 => "WIT (UTC+9)",
                            _ => "WIB (UTC+7)",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut s.tz_offset, 7, "WIB (UTC+7) — Jakarta, Medan, Pontianak");
                            ui.selectable_value(&mut s.tz_offset, 8, "WITA (UTC+8) — Bali, Makassar, Balikpapan");
                            ui.selectable_value(&mut s.tz_offset, 9, "WIT (UTC+9) — Jayapura, Ambon");
                        });
                    ui.end_row();
                    ui.label("Langit kantor");
                    ui.horizontal(|ui| {
                        let mut preview = s.time_preview.is_some();
                        if ui.checkbox(&mut preview, "Pratinjau jam").changed() {
                            s.time_preview = preview.then_some(20.0);
                        }
                        if let Some(h) = &mut s.time_preview {
                            ui.add(egui::Slider::new(h, 0.0..=23.99).suffix(" h"));
                        } else {
                            ui.label(RichText::new("mengikuti jam Indonesia sekarang").weak());
                        }
                    });
                    ui.end_row();
                });
                ui.separator();
                ui.label(RichText::new("Agent & editor").strong());
                egui::Grid::new("agent_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Max tokens / giliran");
                    ui.add(egui::DragValue::new(&mut s.max_tokens).range(1024..=128_000).speed(512));
                    ui.end_row();
                    ui.label("Max langkah tool");
                    ui.add(egui::DragValue::new(&mut s.max_steps).range(1..=200));
                    ui.end_row();
                    ui.label("Ukuran font editor");
                    ui.add(egui::DragValue::new(&mut s.editor_font_size).range(9.0..=28.0).speed(0.2));
                    ui.end_row();
                });
                ui.checkbox(&mut s.follow_agent, "Ikuti agent: buka file yang sedang ditulis secara live");
                ui.checkbox(&mut s.auto_skill, "Buat skill project (.ckdtr/skill.md) otomatis saat project ditambahkan")
                    .on_hover_text("Agent mempelajari project sekali lalu menulis ringkasannya; setiap tugas berikutnya langsung memakai ringkasan itu.");
                ui.checkbox(&mut s.fresh_context, "Mulai konteks baru di setiap tugas (paling hemat token)")
                    .on_hover_text("Agent tidak mengirim ulang riwayat tugas sebelumnya, jadi tidak ingat percakapan lama. Skill project tetap dipakai.");
                ui.checkbox(&mut s.allow_commands, "Izinkan agent menjalankan perintah shell (cmd) — hati-hati");
                ui.label("Instruksi tambahan untuk agent:");
                ui.add(
                    egui::TextEdit::multiline(&mut s.custom_instructions)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .hint_text("mis. Selalu tulis komentar dalam Bahasa Indonesia"),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Disimpan otomatis di {}  (API key & password DB disimpan sebagai teks biasa)",
                        config::app_dir().join("config.json").display()
                    ))
                    .small()
                    .weak(),
                );
            });
        self.show_settings = open;
        if serde_json::to_string(&self.config.settings).unwrap_or_default() != before {
            self.config.save();
        }
    }

    fn db_window(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.db_dialog else { return };
        let mut open = true;
        let mut save = false;
        let mut save_open = false;
        let title = if dlg.index.is_some() { "🗄 Edit Lemari Database" } else { "🗄 Lemari Database Baru" };
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                let c = &mut dlg.cfg;
                ui.label(RichText::new("Setiap lemari di kantor adalah satu koneksi ke server database.").weak());
                ui.add_space(4.0);
                egui::Grid::new("db_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Nama lemari");
                    ui.text_edit_singleline(&mut c.name);
                    ui.end_row();
                    ui.label("Jenis");
                    let before = c.kind;
                    egui::ComboBox::from_id_salt("db_kind").selected_text(c.kind.label()).show_ui(ui, |ui| {
                        for k in DbKind::ALL {
                            ui.selectable_value(&mut c.kind, k, k.label());
                        }
                    });
                    if c.kind != before {
                        c.port = c.kind.default_port();
                        if c.kind == DbKind::MySql && c.user == "postgres" {
                            c.user = "root".into();
                            c.database.clear();
                        }
                    }
                    ui.end_row();
                    if c.kind == DbKind::Sqlite {
                        ui.label("File");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut c.file).desired_width(250.0).hint_text("C:\\data\\app.db"));
                            if ui.button("Pilih…").clicked()
                                && let Some(f) = rfd::FileDialog::new()
                                    .add_filter("SQLite", &["db", "sqlite", "sqlite3", "db3"])
                                    .add_filter("Semua file", &["*"])
                                    .pick_file()
                                {
                                    c.file = f.display().to_string();
                                }
                        });
                        ui.end_row();
                    } else {
                        ui.label("Host");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut c.host).desired_width(200.0));
                            ui.label("Port");
                            ui.add(egui::DragValue::new(&mut c.port).range(1..=65535));
                        });
                        ui.end_row();
                        ui.label("User");
                        ui.text_edit_singleline(&mut c.user);
                        ui.end_row();
                        ui.label("Password");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut c.password).password(!dlg.show_password).desired_width(200.0));
                            ui.checkbox(&mut dlg.show_password, "lihat");
                        });
                        ui.end_row();
                        ui.label("Database");
                        ui.text_edit_singleline(&mut c.database);
                        ui.end_row();
                        ui.label("SSL/TLS");
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut c.ssl, SslMode::Disable, "Mati");
                            ui.selectable_value(&mut c.ssl, SslMode::Prefer, "Jika ada");
                            ui.selectable_value(&mut c.ssl, SslMode::Require, "Wajib");
                        });
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut c.trust_cert, "Mode Wajib: percayai sertifikat server (self-signed)")
                            .on_hover_text("\"Jika ada\" mengenkripsi tanpa memverifikasi sertifikat, seperti psql/mysql. \"Wajib\" memverifikasi sertifikat kecuali opsi ini dicentang.");
                        ui.end_row();
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    save_open = ui
                        .add(egui::Button::new(RichText::new("Simpan & Hubungkan").color(Color32::WHITE)).fill(vs::FOCUS_BLUE))
                        .clicked();
                    save = ui.button("Simpan").clicked();
                });
            });
        if save || save_open {
            let cfg = dlg.cfg.clone();
            let i = match dlg.index {
                Some(i) if i < self.dbs.len() => {
                    self.dbs[i].cfg = cfg;
                    self.dbs[i].disconnect();
                    i
                }
                _ => {
                    self.dbs.push(DbState::new(cfg));
                    self.dbs.len() - 1
                }
            };
            self.db_dialog = None;
            self.save_dbs();
            if save_open {
                self.view = View::Database(i);
                let ctx = self.ctx.clone();
                self.dbs[i].connect(&ctx);
            } else {
                let name = self.dbs[i].cfg.name.clone();
                self.toast(format!("🗄 Lemari \"{name}\" disimpan."));
            }
            return;
        }
        if !open {
            self.db_dialog = None;
        }
    }

    fn new_project_window(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.new_project else { return };
        let mut open = true;
        let mut create: Option<PathBuf> = None;
        egui::Window::new("✨ Project Baru")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label("Nama project");
                ui.text_edit_singleline(&mut dlg.name);
                ui.label("Lokasi");
                ui.horizontal(|ui| {
                    let loc = dlg
                        .parent
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "(pilih folder)".into());
                    ui.label(RichText::new(loc).monospace().small());
                    if ui.button("Pilih…").clicked()
                        && let Some(d) = rfd::FileDialog::new().set_title("Lokasi project baru").pick_folder() {
                            dlg.parent = Some(d);
                        }
                });
                let name = dlg.name.trim();
                let valid = !name.is_empty()
                    && dlg.parent.is_some()
                    && name != "."
                    && name != ".."
                    && !name.ends_with('.')
                    && !name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']);
                ui.add_space(6.0);
                if ui.add_enabled(valid, egui::Button::new("Buat & rekrut karyawan")).clicked()
                    && let Some(parent) = &dlg.parent {
                        create = Some(parent.join(dlg.name.trim()));
                    }
            });
        if let Some(dir) = create {
            let name = dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            match std::fs::create_dir_all(&dir) {
                Ok(()) => {
                    let readme = dir.join("README.md");
                    if !readme.exists() {
                        let _ = std::fs::write(&readme, format!("# {name}\n\nDibuat dengan Cokandetor.\n"));
                    }
                    self.new_project = None;
                    self.add_folder(dir);
                }
                Err(e) => self.toast(format!("Gagal membuat folder: {e}")),
            }
            return;
        }
        if !open {
            self.new_project = None;
        }
    }

    fn toasts_ui(&mut self, ctx: &egui::Context, now: f64) {
        self.toasts.retain(|(_, t)| now - t < 4.0);
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::unique("toasts"))
            .order(egui::Order::Foreground)
            .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -40.0))
            .show(ctx, |ui| {
                for (msg, t) in &self.toasts {
                    let a = (1.0 - ((now - t - 3.0).max(0.0) as f32)).clamp(0.0, 1.0);
                    Frame::new()
                        .fill(Color32::from_rgb(0x25, 0x25, 0x26).gamma_multiply(a))
                        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(0x45, 0x45, 0x45).gamma_multiply(a)))
                        .inner_margin(Margin::symmetric(12, 8))
                        .corner_radius(4)
                        .show(ui, |ui| ui.label(RichText::new(msg).color(Color32::WHITE.gamma_multiply(a))));
                    ui.add_space(4.0);
                }
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.poll(now);

        match self.view {
            View::Project(i) if i >= self.projects.len() => self.view = View::World,
            View::Database(i) if i >= self.dbs.len() => self.view = View::World,
            _ => {}
        }

        let mut actions = Vec::new();
        let clock_label = self.clock_label();
        self.top_bar(ui, &mut actions);
        match self.view {
            View::World => office::show(
                ui,
                &mut self.projects,
                &mut self.dbs,
                &mut self.office,
                &mut self.config.settings,
                now,
                &mut actions,
            ),
            View::Project(i) => {
                editor::show(
                    ui,
                    i,
                    &mut self.projects[i],
                    &self.dbs,
                    &mut self.config.settings,
                    &clock_label,
                    now,
                    &mut actions,
                );
            }
            View::Database(i) => {
                let font = self.config.settings.editor_font_size;
                dbview::show(ui, i, &mut self.dbs[i], font, &clock_label, now, &mut actions);
            }
        }
        if self.view != View::World
            && self.db_dialog.is_none()
            && !self.typing_last_frame
            && ctx.input(|inp| inp.key_pressed(egui::Key::Escape))
        {
            actions.push(Action::BackToWorld);
        }
        self.settings_window(&ctx);
        self.new_project_window(&ctx);
        self.db_window(&ctx);
        self.toasts_ui(&ctx, now);
        self.handle(actions);

        if self.projects.iter().any(|p| p.agent.is_some() || p.scan_rx.is_some())
            || self.dbs.iter().any(|d| d.pending > 0)
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        // Keep the status-bar clock ticking.
        ctx.request_repaint_after(std::time::Duration::from_secs(15));
        self.typing_last_frame = ctx.egui_wants_keyboard_input();
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase().trim_end_matches(['\\', '/'])
        == b.to_string_lossy().to_lowercase().trim_end_matches(['\\', '/'])
}

/// `canonicalize` on Windows returns `\\?\C:\…`; strip that for display and joins.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => p,
    }
}
