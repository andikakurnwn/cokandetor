//! The project view, laid out like VS Code: activity bar, Explorer/Search
//! side bar, tabbed editor with breadcrumbs, minimap and live agent edits,
//! the agent chat as a secondary side bar, and a blue status bar.

use egui::text::{CCursor, CCursorRange, LayoutJob};
use egui::{
    Align, Align2, CentralPanel, Color32, FontId, Frame, Id, Key, Margin, Modifiers, Order, Panel, Pos2, Rect, Response,
    RichText, ScrollArea, Sense, Stroke, StrokeKind, TextBuffer, TextEdit, Ui, pos2, vec2,
};
use std::collections::HashSet;
use std::path::Path;

use crate::app::{Action, Project};
use crate::config::Settings;
use crate::db::DbState;
use crate::project::{self, FileNode};
use crate::theme::vs;
use crate::icons::{self, Icon};
use crate::{chat, office, theme};

pub struct Live {
    pub tool_id: String,
    pub content: String,
    /// Byte offset where the agent is currently typing.
    pub caret: usize,
    pub verb: &'static str,
}

pub struct Flash {
    pub start: usize,
    pub end: usize,
    pub at: f64,
}

#[derive(Default, Clone, Copy)]
pub struct ScrollInfo {
    pub offset: f32,
    pub content: f32,
    pub view: f32,
}

pub struct OpenFile {
    pub rel: String,
    pub content: String,
    pub saved: String,
    pub syntax: String,
    pub live: Option<Live>,
    pub flash: Option<Flash>,
    /// Disk content that changed while the buffer had unsaved edits.
    pub conflict: Option<String>,
    pub readonly: Option<String>,
    /// The file was deleted on disk while this tab had unsaved edits.
    pub deleted: bool,
    pub cursor: (usize, usize),
    pub scroll: ScrollInfo,
    pub scroll_to: Option<f32>,
    /// Line to put the cursor on (from search / quick open).
    pub jump: Option<usize>,
    minimap: Option<(u64, Vec<MiniRun>)>,
}

impl OpenFile {
    pub fn dirty(&self) -> bool {
        self.readonly.is_none() && self.content != self.saved
    }

    fn name(&self) -> &str {
        self.rel.rsplit('/').next().unwrap_or(&self.rel)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum SideView {
    #[default]
    Explorer,
    Search,
    Database,
}

pub struct SearchHit {
    pub rel: String,
    pub line: usize,
    pub text: String,
}

pub struct QuickOpen {
    pub query: String,
    pub sel: usize,
}

pub struct EditorState {
    pub tabs: Vec<OpenFile>,
    pub active: Option<usize>,
    pub new_file: Option<String>,
    pub side: SideView,
    pub side_open: bool,
    pub chat_open: bool,
    pub expanded: HashSet<String>,
    pub search: String,
    pub search_case: bool,
    pub hits: Vec<SearchHit>,
    pub searched: bool,
    /// The query (and case mode) that produced `hits`.
    hits_query: (String, bool),
    pub quick: Option<QuickOpen>,
    /// Tab waiting for "save changes?" confirmation.
    pub pending_close: Option<usize>,
    /// `src/` is opened once for new projects, then left to the user.
    auto_expanded: bool,
    branch: Option<String>,
    branch_at: f64,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active: None,
            new_file: None,
            side: SideView::Explorer,
            side_open: true,
            chat_open: true,
            expanded: HashSet::new(),
            search: String::new(),
            search_case: false,
            hits: Vec::new(),
            searched: false,
            hits_query: (String::new(), false),
            quick: None,
            pending_close: None,
            auto_expanded: false,
            branch: None,
            branch_at: -100.0,
        }
    }
}

pub fn norm_rel(rel: &str) -> String {
    rel.trim().replace('\\', "/").trim_start_matches("./").to_string()
}

/// First and last (exclusive) changed line of `new` compared to `old`.
pub fn changed_lines(old: &str, new: &str) -> Option<(usize, usize)> {
    if old == new {
        return None;
    }
    let a: Vec<&str> = old.split('\n').collect();
    let b: Vec<&str> = new.split('\n').collect();
    let mut s = 0;
    while s < a.len() && s < b.len() && a[s] == b[s] {
        s += 1;
    }
    let mut e = 0;
    while e < a.len() - s && e < b.len() - s && a[a.len() - 1 - e] == b[b.len() - 1 - e] {
        e += 1;
    }
    let end = (b.len() - e).max(s + 1).min(b.len().max(1));
    Some((s, end))
}

fn load(path: &Path) -> (String, Option<String>) {
    match std::fs::metadata(path) {
        Err(_) => (String::new(), None),
        Ok(m) if m.len() > 8 * 1024 * 1024 => (String::new(), Some(format!("File terlalu besar ({} MB).", m.len() / 1_048_576))),
        Ok(_) => match std::fs::read(path) {
            Ok(bytes) if bytes.iter().take(8000).any(|&b| b == 0) => {
                (String::new(), Some("File biner — tidak bisa ditampilkan sebagai teks.".into()))
            }
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => (text, None),
                Err(_) => (
                    String::new(),
                    Some("File ini bukan UTF-8 — tidak dibuka agar isinya tidak rusak saat disimpan.".into()),
                ),
            },
            Err(e) => (String::new(), Some(format!("Gagal membaca: {e}"))),
        },
    }
}

impl EditorState {
    pub fn find(&self, rel: &str) -> Option<usize> {
        self.tabs.iter().position(|t| t.rel == rel)
    }

    /// Opens (or focuses) a tab. Paths outside the project are refused.
    pub fn open(&mut self, root: &Path, rel: &str, focus: bool) -> Option<usize> {
        let path = crate::agent::tools::resolve(root, &norm_rel(rel)).ok()?;
        if path == root {
            return None;
        }
        let rel = crate::agent::tools::rel_display(root, &path);
        // Reveal the file in the Explorer.
        let mut acc = String::new();
        for part in rel.split('/').collect::<Vec<_>>().iter().rev().skip(1).rev() {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(part);
            self.expanded.insert(acc.clone());
        }
        let i = match self.find(&rel) {
            Some(i) => i,
            None => {
                let (content, readonly) = load(&root.join(&rel));
                self.tabs.push(OpenFile {
                    syntax: theme::syntax_name(&rel),
                    rel,
                    saved: content.clone(),
                    content,
                    live: None,
                    flash: None,
                    conflict: None,
                    readonly,
                    deleted: false,
                    cursor: (1, 1),
                    scroll: ScrollInfo::default(),
                    scroll_to: None,
                    jump: None,
                    minimap: None,
                });
                self.tabs.len() - 1
            }
        };
        if focus || self.active.is_none() {
            self.active = Some(i);
        }
        Some(i)
    }

    pub fn close(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        self.tabs.remove(i);
        self.pending_close = None;
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(a) if a > i => Some(a - 1),
            Some(a) if a == i => Some(a.min(self.tabs.len() - 1)),
            other => other,
        };
    }

    pub fn save(&mut self, root: &Path, i: usize) -> Result<(), String> {
        let t = &mut self.tabs[i];
        if let Some(reason) = &t.readonly {
            return Err(reason.clone());
        }
        let path = crate::agent::tools::resolve(root, &t.rel)?;
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&path, &t.content).map_err(|e| format!("Gagal menyimpan {}: {e}", t.rel))?;
        t.saved = t.content.clone();
        t.conflict = None;
        t.deleted = false;
        Ok(())
    }

    /// Saves every dirty tab before the agent runs. Tabs in conflict with
    /// the disk (or whose file was deleted) are left alone so they are never
    /// resolved silently. Returns (saved, skipped).
    pub fn save_all(&mut self, root: &Path) -> (usize, usize) {
        let (mut saved, mut skipped) = (0, 0);
        for i in 0..self.tabs.len() {
            let t = &self.tabs[i];
            if !t.dirty() {
                continue;
            }
            if t.conflict.is_some() || t.deleted {
                skipped += 1;
            } else if self.save(root, i).is_ok() {
                saved += 1;
            }
        }
        (saved, skipped)
    }

    /// The agent changed a file on disk.
    pub fn apply_change(&mut self, rel: &str, after: Option<&str>, now: f64) {
        let rel = norm_rel(rel);
        let Some(i) = self.find(&rel) else { return };
        let t = &mut self.tabs[i];
        t.live = None;
        let Some(after) = after else {
            if t.dirty() {
                // Keep the user's edits; saving recreates the file.
                t.deleted = true;
                t.saved.clear();
            } else {
                t.readonly = Some("File ini dihapus oleh agent.".into());
            }
            return;
        };
        if t.dirty() {
            t.conflict = Some(after.to_string());
            return;
        }
        if let Some((s, e)) = changed_lines(&t.content, after) {
            t.flash = Some(Flash { start: s, end: e, at: now });
        }
        t.content = after.to_string();
        t.saved = t.content.clone();
        t.readonly = None;
        t.deleted = false;
    }

    /// Something (maybe an external tool) touched a file on disk.
    pub fn reload_from_disk(&mut self, root: &Path, rel: &str, now: f64) {
        let Some(i) = self.find(rel) else { return };
        if self.tabs[i].live.is_some() {
            return;
        }
        let path = root.join(rel);
        let t = &mut self.tabs[i];
        if !path.exists() {
            if t.dirty() {
                t.deleted = true;
                t.saved.clear();
            } else if t.readonly.is_none() {
                t.readonly = Some("File ini sudah dihapus dari disk.".into());
            }
            return;
        }
        let (content, readonly) = load(&path);
        if readonly.is_some() {
            return;
        }
        // A deleted file that came back (e.g. git checkout) is editable again.
        if t.readonly.is_some() {
            t.readonly = None;
            t.saved = content.clone();
            t.content = content;
            return;
        }
        if content == t.saved {
            return;
        }
        if t.dirty() {
            if content != t.content {
                t.conflict = Some(content);
            }
            return;
        }
        if let Some((s, e)) = changed_lines(&t.content, &content) {
            t.flash = Some(Flash { start: s, end: e, at: now });
        }
        t.saved = content.clone();
        t.content = content;
    }

    fn refresh_branch(&mut self, root: &Path, now: f64) {
        if now - self.branch_at < 5.0 {
            return;
        }
        self.branch_at = now;
        self.branch = std::fs::read_to_string(root.join(".git").join("HEAD")).ok().map(|h| {
            let h = h.trim();
            h.strip_prefix("ref: refs/heads/")
                .map(str::to_string)
                .unwrap_or_else(|| h.chars().take(7).collect())
        });
    }
}

/// VS Code-style file icon: a short coloured glyph per file type.
pub fn file_badge(name: &str) -> (&'static str, Color32) {
    let c = |h: u32| Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8);
    let base = name.rsplit('/').next().unwrap_or(name).to_lowercase();
    if base == "dockerfile" || base.starts_with("docker-compose") {
        return ("◈", c(0x2496ed));
    }
    if base == "makefile" {
        return ("M", c(0xe37933));
    }
    if base.starts_with(".env") {
        return ("⚙", c(0xfaf743));
    }
    if base.starts_with(".git") {
        return ("◆", c(0xf14e32));
    }
    match project::extension(name).as_str() {
        "rs" => ("rs", c(0xdea584)),
        "py" | "pyw" => ("py", c(0x4b8bbe)),
        "js" | "mjs" | "cjs" => ("JS", c(0xf1e05a)),
        "ts" | "mts" => ("TS", c(0x3178c6)),
        "tsx" | "jsx" => ("⚛", c(0x61dafb)),
        "json" => ("{}", c(0xcbcb41)),
        "md" | "markdown" => ("M↓", c(0x519aba)),
        "html" | "htm" => ("<>", c(0xe44d26)),
        "css" | "scss" | "sass" | "less" => ("#", c(0xa074c4)),
        "toml" | "yaml" | "yml" | "ini" | "lock" | "cfg" | "env" => ("⚙", c(0x8a9aa6)),
        "sql" => ("db", c(0xf29111)),
        "java" | "kt" | "kts" => ("J", c(0xe76f00)),
        "go" => ("go", c(0x00add8)),
        "c" | "h" => ("C", c(0xa8b9cc)),
        "cpp" | "cc" | "hpp" | "cxx" => ("C+", c(0xf34b7d)),
        "cs" => ("C#", c(0x9b4f96)),
        "php" => ("php", c(0x8892bf)),
        "rb" => ("rb", c(0xcc342d)),
        "sh" | "bat" | "cmd" | "ps1" => ("$", c(0x89e051)),
        "dart" => ("◆", c(0x40c4ff)),
        "vue" => ("V", c(0x41b883)),
        "svelte" => ("S", c(0xff3e00)),
        "swift" => ("sw", c(0xf05138)),
        "xml" | "xaml" | "plist" => ("<>", c(0xe37933)),
        "lua" => ("lu", c(0x51a0cf)),
        "r" => ("R", c(0x358a5b)),
        "scala" => ("sc", c(0xdc322f)),
        "ex" | "exs" => ("ex", c(0xa074c4)),
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "webp" | "bmp" => ("▣", c(0xa074c4)),
        "txt" | "log" => ("≡", c(0xa0a0a0)),
        _ => ("•", c(0x8a8a8a)),
    }
}

fn paint_badge(ui: &Ui, pos: Pos2, name: &str, size: f32) {
    let (g, c) = file_badge(name);
    let font = if g.chars().count() > 1 { FontId::monospace(size * 0.78) } else { FontId::monospace(size) };
    ui.painter().text(pos, Align2::CENTER_CENTER, g, font, c);
}

/// One activity bar button. Shared with the database view.
pub fn activity_button(ui: &mut Ui, icon: Icon, active: bool, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::click());
    let color = if active || resp.hovered() { vs::ACTIVITY_FG } else { vs::ACTIVITY_DIM };
    if active {
        ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(2.0, 48.0)), 0.0, vs::ACTIVITY_FG);
    }
    icons::paint(ui.painter(), icon, rect.center(), 22.0, color);
    resp.on_hover_text(tip)
}

pub fn sidebar_title(ui: &mut Ui, title: &str, add_actions: impl FnOnce(&mut Ui)) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(RichText::new(title).size(11.0).color(vs::SIDEBAR_TITLE));
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(6.0);
            add_actions(ui);
        });
    });
    ui.add_space(4.0);
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    idx: usize,
    p: &mut Project,
    dbs: &[DbState],
    settings: &mut Settings,
    clock_label: &str,
    now: f64,
    actions: &mut Vec<Action>,
) {
    let root = p.entry.path.clone();
    let mut open_req: Option<(String, Option<usize>)> = None;
    p.editor.refresh_branch(&root, now);

    // Keyboard shortcuts.
    let (save, quick, find, side, close) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::COMMAND, Key::S),
            i.consume_key(Modifiers::COMMAND, Key::P),
            i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::F),
            i.consume_key(Modifiers::COMMAND, Key::B),
            i.consume_key(Modifiers::COMMAND, Key::W),
        )
    });
    if let (true, Some(a)) = (save, p.editor.active) {
        match p.editor.save(&root, a) {
            Ok(()) => actions.push(Action::Toast(format!("Disimpan: {}", p.editor.tabs[a].rel))),
            Err(e) => actions.push(Action::Toast(e)),
        }
    }
    if quick {
        p.editor.quick = Some(QuickOpen { query: String::new(), sel: 0 });
    }
    if find {
        p.editor.side = SideView::Search;
        p.editor.side_open = true;
    }
    if side {
        p.editor.side_open = !p.editor.side_open;
    }
    if let (true, Some(a)) = (close, p.editor.active) {
        if p.editor.tabs[a].dirty() {
            p.editor.pending_close = Some(a);
        } else {
            p.editor.close(a);
        }
    }

    Panel::bottom("status")
        .frame(Frame::new().fill(if p.agent.is_some() { vs::STATUS_BUSY } else { vs::STATUS_BG }).inner_margin(Margin::symmetric(8, 2)))
        .show_separator_line(false)
        .show(ui, |ui| status_bar(ui, p, clock_label));

    Panel::left("activity")
        .frame(Frame::new().fill(vs::ACTIVITY_BG))
        .resizable(false)
        .exact_size(48.0)
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let e = &mut p.editor;
            for (icon, view, tip) in [
                (Icon::Files, SideView::Explorer, "Explorer (Ctrl+B)"),
                (Icon::Search, SideView::Search, "Search (Ctrl+Shift+F)"),
                (Icon::Database, SideView::Database, "Lemari database"),
            ] {
                if activity_button(ui, icon, e.side_open && e.side == view, tip).clicked() {
                    if e.side == view {
                        e.side_open = !e.side_open;
                    } else {
                        e.side = view;
                        e.side_open = true;
                    }
                }
            }
            if activity_button(ui, Icon::Chat, e.chat_open, "Chat agent").clicked() {
                e.chat_open = !e.chat_open;
            }
            ui.with_layout(egui::Layout::bottom_up(Align::Center), |ui| {
                if activity_button(ui, Icon::Gear, false, "Pengaturan").clicked() {
                    actions.push(Action::OpenSettings);
                }
                if activity_button(ui, Icon::Home, false, "Kembali ke kantor (Esc)").clicked() {
                    actions.push(Action::BackToWorld);
                }
            });
        });

    if p.editor.side_open {
        Panel::left("sidebar")
            .frame(Frame::new().fill(vs::SIDEBAR_BG))
            .resizable(true)
            .default_size(260.0)
            .size_range(170.0..=520.0)
            .show_separator_line(false)
            .show(ui, |ui| match p.editor.side {
                SideView::Explorer => explorer(ui, idx, p, &mut open_req, actions),
                SideView::Search => search_view(ui, p, &mut open_req),
                SideView::Database => database_list(ui, dbs, actions),
            });
    }

    if p.editor.chat_open {
        Panel::right("agent")
            .frame(Frame::new().fill(vs::SIDEBAR_BG).inner_margin(Margin { left: 10, right: 10, top: 0, bottom: 6 }))
            .resizable(true)
            .default_size(380.0)
            .size_range(280.0..=760.0)
            .show(ui, |ui| {
                let mut req = None;
                chat::panel(ui, idx, p, settings, actions, &mut req);
                if let Some(r) = req {
                    open_req = Some((r, None));
                }
            });
    }

    CentralPanel::no_frame()
        .frame(Frame::new().fill(vs::EDITOR_BG))
        .show(ui, |ui| {
            tab_bar(ui, p);
            match p.editor.active {
                Some(a) if a < p.editor.tabs.len() => {
                    breadcrumbs(ui, &p.editor.tabs[a].rel);
                    let font = settings.editor_font_size;
                    file_view(ui, idx, &mut p.editor, a, &root, font, now, actions);
                }
                _ => welcome(ui, idx, p, now, actions),
            }
        });

    quick_open(ui, p, &mut open_req);
    confirm_close(ui, p, &root, actions);

    if let Some((rel, line)) = open_req {
        match p.editor.open(&root, &rel, true) {
            Some(i) => {
                if let Some(l) = line {
                    p.editor.tabs[i].jump = Some(l);
                }
            }
            None => actions.push(Action::Toast(format!("Tidak bisa membuka {rel}: di luar folder project."))),
        }
    }
}

// ---------------------------------------------------------------- side bar

fn explorer(ui: &mut Ui, idx: usize, p: &mut Project, open_req: &mut Option<(String, Option<usize>)>, actions: &mut Vec<Action>) {
    let has_skill = p.has_skill;
    sidebar_title(ui, "EXPLORER", |ui| {
        let tip = if has_skill {
            "Perbarui skill project (.ckdtr/skill.md)"
        } else {
            "Buat skill project (.ckdtr/skill.md)"
        };
        if icons::icon_button(ui, Icon::Sparkle, tip).clicked() {
            actions.push(Action::MakeSkill(idx));
        }
        if icons::icon_button(ui, Icon::Refresh, "Pindai ulang").clicked() {
            actions.push(Action::Rescan(idx));
        }
        if icons::icon_button(ui, Icon::Collapse, "Ciutkan semua folder").clicked() {
            p.editor.expanded.clear();
        }
        if icons::icon_button(ui, Icon::Plus, "File baru").clicked() {
            p.editor.new_file = Some(String::new());
        }
    });

    // Section header, like VS Code's workspace folder row.
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    icons::paint(ui.painter(), Icon::ChevronDown, pos2(rect.left() + 14.0, rect.center().y), 12.0, vs::TEXT_BRIGHT);
    ui.painter().text(
        pos2(rect.left() + 24.0, rect.center().y),
        Align2::LEFT_CENTER,
        p.entry.name.to_uppercase(),
        FontId::proportional(11.5),
        vs::TEXT_BRIGHT,
    );

    if let Some(name) = &mut p.editor.new_file {
        let mut create = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let r = ui.add(
                TextEdit::singleline(name)
                    .hint_text("src/baru.rs")
                    .desired_width(ui.available_width() - 10.0),
            );
            r.request_focus();
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                create = true;
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                cancel = true;
            }
        });
        if create && !name.trim().is_empty() {
            let rel = norm_rel(name);
            match crate::agent::tools::resolve(&p.entry.path, &rel) {
                Ok(path) => {
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    if !path.exists() {
                        let _ = std::fs::write(&path, "");
                    }
                    *open_req = Some((rel, None));
                    actions.push(Action::Rescan(idx));
                }
                Err(e) => actions.push(Action::Toast(e)),
            }
            cancel = true;
        }
        if cancel {
            p.editor.new_file = None;
        }
    }

    let active = p.editor.active.and_then(|a| p.editor.tabs.get(a)).map(|t| t.rel.clone());
    let live: Vec<String> = p.editor.tabs.iter().filter(|t| t.live.is_some()).map(|t| t.rel.clone()).collect();
    let dirty: Vec<String> = p.editor.tabs.iter().filter(|t| t.dirty()).map(|t| t.rel.clone()).collect();
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        match &p.scan {
            Some(scan) => {
                if !p.editor.auto_expanded {
                    p.editor.auto_expanded = true;
                    if scan.root.children.iter().any(|c| c.is_dir && c.name == "src") {
                        p.editor.expanded.insert("src".into());
                    }
                }
                let mut ctx = TreeCtx {
                    expanded: &mut p.editor.expanded,
                    active: active.as_deref(),
                    live: &live,
                    dirty: &dirty,
                    open_req,
                };
                tree(ui, &scan.root, 0, &mut ctx);
                if scan.truncated {
                    ui.label(RichText::new(format!("  … dibatasi {} file", project::MAX_FILES)).weak().small());
                }
            }
            None => {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.spinner();
                    ui.label("Memindai…");
                });
            }
        }
    });
}

struct TreeCtx<'a> {
    expanded: &'a mut HashSet<String>,
    active: Option<&'a str>,
    live: &'a [String],
    dirty: &'a [String],
    open_req: &'a mut Option<(String, Option<usize>)>,
}

fn tree(ui: &mut Ui, node: &FileNode, depth: usize, cx: &mut TreeCtx) {
    for c in &node.children {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
        let selected = !c.is_dir && cx.active == Some(c.rel.as_str());
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, 0.0, if resp.hovered() { vs::LIST_FOCUS } else { vs::LIST_ACTIVE });
        } else if resp.hovered() {
            painter.rect_filled(rect, 0.0, vs::LIST_HOVER);
        }
        for d in 0..depth {
            let x = rect.left() + 16.0 + d as f32 * 12.0;
            painter.vline(x, rect.y_range(), Stroke::new(1.0, Color32::from_rgb(0x40, 0x40, 0x40)));
        }
        let x = rect.left() + 10.0 + depth as f32 * 12.0;
        let is_live = if c.is_dir {
            cx.live.iter().any(|l| l.starts_with(&format!("{}/", c.rel)))
        } else {
            cx.live.iter().any(|l| l == &c.rel)
        };
        let color = if is_live {
            theme::LIVE
        } else if cx.dirty.iter().any(|d| d == &c.rel) {
            Color32::from_rgb(0xe2, 0xc0, 0x8d)
        } else {
            vs::TEXT
        };
        if c.is_dir {
            let open = cx.expanded.contains(&c.rel) || is_live;
            let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
            icons::paint(painter, chevron, pos2(x + 5.0, rect.center().y), 12.0, Color32::from_rgb(0xc5, 0xc5, 0xc5));
            painter.text(pos2(x + 14.0, rect.center().y), Align2::LEFT_CENTER, &c.name, FontId::proportional(13.0), color);
            if is_live {
                painter.circle_filled(pos2(rect.right() - 14.0, rect.center().y), 3.5, theme::LIVE);
            }
            if resp.clicked() {
                if cx.expanded.contains(&c.rel) {
                    cx.expanded.remove(&c.rel);
                } else {
                    cx.expanded.insert(c.rel.clone());
                }
            }
            if open {
                tree(ui, c, depth + 1, cx);
            }
        } else {
            paint_badge(ui, pos2(x + 8.0, rect.center().y), &c.name, 12.0);
            ui.painter().text(pos2(x + 20.0, rect.center().y), Align2::LEFT_CENTER, &c.name, FontId::proportional(13.0), color);
            if cx.dirty.iter().any(|d| d == &c.rel) {
                ui.painter().text(pos2(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER, "M", FontId::proportional(11.0), color);
            }
            let resp = resp.on_hover_text(format!("{} · {} baris", c.rel, c.lines));
            if resp.clicked() {
                *cx.open_req = Some((c.rel.clone(), None));
            }
        }
    }
}

fn run_search(root: &Path, query: &str, case: bool) -> Vec<SearchHit> {
    let needle = if case { query.to_string() } else { query.to_lowercase() };
    let mut hits = Vec::new();
    for e in project::walker(root, None).flatten() {
        if !e.file_type().is_some_and(|t| t.is_file()) || e.metadata().map(|m| m.len()).unwrap_or(0) > 1_048_576 {
            continue;
        }
        let Ok(bytes) = std::fs::read(e.path()) else { continue };
        if bytes.iter().take(8000).any(|&b| b == 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let rel = e.path().strip_prefix(root).unwrap_or(e.path()).to_string_lossy().replace('\\', "/");
        for (i, line) in text.lines().enumerate() {
            let hay = if case { line.to_string() } else { line.to_lowercase() };
            if hay.contains(&needle) {
                hits.push(SearchHit {
                    rel: rel.clone(),
                    line: i,
                    text: line.trim().chars().take(160).collect(),
                });
                if hits.len() >= 500 {
                    return hits;
                }
            }
        }
    }
    hits
}

fn search_view(ui: &mut Ui, p: &mut Project, open_req: &mut Option<(String, Option<usize>)>) {
    sidebar_title(ui, "SEARCH", |_| {});
    let e = &mut p.editor;
    let mut go = false;
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        let r = ui.add(TextEdit::singleline(&mut e.search).hint_text("Cari").desired_width(ui.available_width() - 44.0));
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            go = true;
        }
        if ui
            .selectable_label(e.search_case, RichText::new("Aa").size(12.0))
            .on_hover_text("Match Case")
            .clicked()
        {
            e.search_case = !e.search_case;
            go = true;
        }
    });
    if go && !e.search.trim().is_empty() {
        e.hits = run_search(&p.entry.path, &e.search, e.search_case);
        e.hits_query = (e.search.clone(), e.search_case);
        e.searched = true;
    }
    ui.add_space(4.0);
    if e.searched {
        let files: HashSet<&str> = e.hits.iter().map(|h| h.rel.as_str()).collect();
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                RichText::new(format!("{} hasil di {} file{}", e.hits.len(), files.len(), if e.hits.len() >= 500 { " (dibatasi)" } else { "" }))
                    .size(11.5)
                    .color(vs::LINE_NO),
            );
        });
    }
    let (query, case) = &e.hits_query;
    let needle = if *case { query.clone() } else { query.to_lowercase() };
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let mut last = "";
        for h in &e.hits {
            if h.rel != last {
                last = &h.rel;
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
                paint_badge(ui, pos2(rect.left() + 20.0, rect.center().y), &h.rel, 12.0);
                let name = h.rel.rsplit('/').next().unwrap_or(&h.rel);
                let dir = h.rel.strip_suffix(name).unwrap_or("").trim_end_matches('/');
                let g = ui.painter().layout_no_wrap(name.to_string(), FontId::proportional(13.0), vs::TEXT_BRIGHT);
                let w = g.size().x;
                ui.painter().galley(pos2(rect.left() + 32.0, rect.center().y - g.size().y / 2.0), g, vs::TEXT_BRIGHT);
                ui.painter().text(pos2(rect.left() + 38.0 + w, rect.center().y), Align2::LEFT_CENTER, dir, FontId::proportional(11.5), vs::LINE_NO);
            }
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, 0.0, vs::LIST_HOVER);
            }
            // Highlight the match inside the line.
            let mut job = LayoutJob::default();
            let font = FontId::proportional(12.5);
            let lower = if *case { h.text.clone() } else { h.text.to_lowercase() };
            // Lower-casing can shift byte offsets, so only slice when they line up.
            let parts = lower
                .find(&needle)
                .filter(|_| !needle.is_empty())
                .map(|at| (at, at + needle.len()))
                .and_then(|(a, b)| Some((h.text.get(..a)?, h.text.get(a..b)?, h.text.get(b..)?)));
            match parts {
                Some((before, hit, after)) => {
                    job.append(before, 0.0, egui::TextFormat::simple(font.clone(), vs::TEXT));
                    job.append(
                        hit,
                        0.0,
                        egui::TextFormat {
                            background: vs::MATCH,
                            ..egui::TextFormat::simple(font.clone(), vs::TEXT_BRIGHT)
                        },
                    );
                    job.append(after, 0.0, egui::TextFormat::simple(font, vs::TEXT));
                }
                None => job.append(&h.text, 0.0, egui::TextFormat::simple(font, vs::TEXT)),
            }
            let g = ui.painter().layout_job(job);
            ui.painter().with_clip_rect(rect).galley(pos2(rect.left() + 34.0, rect.center().y - g.size().y / 2.0), g, vs::TEXT);
            if resp.clicked() {
                *open_req = Some((h.rel.clone(), Some(h.line)));
            }
        }
    });
}

fn database_list(ui: &mut Ui, dbs: &[DbState], actions: &mut Vec<Action>) {
    sidebar_title(ui, "LEMARI DATABASE", |ui| {
        if icons::icon_button(ui, Icon::Plus, "Tambah lemari (koneksi baru)").clicked() {
            actions.push(Action::NewDb);
        }
    });
    if dbs.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("Belum ada lemari. Klik + untuk menghubungkan database.").weak().size(12.0));
        });
    }
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    for (i, d) in dbs.iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, 0.0, vs::LIST_HOVER);
        }
        let led = match d.status {
            crate::db::Status::Connected => theme::OK,
            crate::db::Status::Error => theme::DANGER,
            crate::db::Status::Busy => theme::WARM,
            crate::db::Status::Idle => vs::LINE_NO,
        };
        ui.painter().text(pos2(rect.left() + 22.0, rect.center().y), Align2::CENTER_CENTER, d.cfg.kind.code(), FontId::monospace(12.0), Color32::from_rgb(0xf2, 0x91, 0x11));
        ui.painter().text(pos2(rect.left() + 40.0, rect.top() + 12.0), Align2::LEFT_CENTER, &d.cfg.name, FontId::proportional(13.0), vs::TEXT_BRIGHT);
        ui.painter().text(pos2(rect.left() + 40.0, rect.top() + 28.0), Align2::LEFT_CENTER, d.cfg.target(), FontId::proportional(11.0), vs::LINE_NO);
        ui.painter().circle_filled(pos2(rect.right() - 14.0, rect.center().y), 4.0, led);
        if resp.clicked() {
            actions.push(Action::OpenDb(i));
        }
    }
}

// ---------------------------------------------------------------- editor group

fn tab_bar(ui: &mut Ui, p: &mut Project) {
    let mut close: Option<usize> = None;
    let mut activate: Option<usize> = None;
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 35.0), Sense::hover());
    ui.painter().rect_filled(bar, 0.0, vs::TAB_BAR_BG);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(Align::Min)));
    ScrollArea::horizontal().id_salt("tabs").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(&mut child, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        for (i, t) in p.editor.tabs.iter().enumerate() {
            let selected = p.editor.active == Some(i);
            let name = t.name().to_string();
            let g = ui.painter().layout_no_wrap(name, FontId::proportional(13.0), vs::TEXT);
            let w = g.size().x + 66.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(w, 35.0), Sense::click());
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, if selected { vs::TAB_ACTIVE_BG } else { vs::TAB_BG });
            painter.vline(rect.right() - 0.5, rect.y_range(), Stroke::new(1.0, vs::TAB_BORDER));
            if selected {
                painter.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, vs::FOCUS_BLUE));
            }
            paint_badge(ui, pos2(rect.left() + 18.0, rect.center().y), &t.rel, 12.0);
            let fg = if t.live.is_some() {
                theme::LIVE
            } else if selected {
                vs::TEXT_BRIGHT
            } else {
                vs::TAB_FG
            };
            ui.painter().galley(pos2(rect.left() + 30.0, rect.center().y - g.size().y / 2.0), g, fg);
            let close_rect = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), vec2(20.0, 20.0));
            let over_close = resp.hover_pos().is_some_and(|p| close_rect.contains(p));
            if over_close {
                ui.painter().rect_filled(close_rect, 3.0, Color32::from_rgb(0x45, 0x45, 0x45));
            }
            if t.dirty() && !over_close {
                ui.painter().circle_filled(close_rect.center(), 4.0, vs::TEXT);
            } else if t.live.is_some() && !over_close {
                ui.painter().circle_filled(close_rect.center(), 4.0, theme::LIVE);
            } else if selected || resp.hovered() {
                icons::paint(ui.painter(), Icon::Close, close_rect.center(), 13.0, vs::TEXT);
            }
            if resp.clicked() {
                if over_close {
                    close = Some(i);
                } else {
                    activate = Some(i);
                }
            }
            if resp.middle_clicked() {
                close = Some(i);
            }
            resp.on_hover_text(&t.rel);
        }
    });
    if let Some(i) = activate {
        p.editor.active = Some(i);
    }
    if let Some(i) = close {
        if p.editor.tabs[i].dirty() {
            p.editor.pending_close = Some(i);
        } else {
            p.editor.close(i);
        }
    }
}

/// VS Code's "Do you want to save the changes?" prompt.
fn confirm_close(ui: &mut Ui, p: &mut Project, root: &Path, actions: &mut Vec<Action>) {
    let Some(i) = p.editor.pending_close.filter(|&i| i < p.editor.tabs.len()) else {
        p.editor.pending_close = None;
        return;
    };
    let name = p.editor.tabs[i].name().to_string();
    let mut choice = None;
    egui::Window::new("Simpan perubahan?")
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            ui.label(format!("Simpan perubahan pada {name} sebelum ditutup?"));
            ui.label(RichText::new("Perubahan akan hilang jika tidak disimpan.").small().color(vs::LINE_NO));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("Simpan").color(Color32::WHITE)).fill(vs::FOCUS_BLUE)).clicked() {
                    choice = Some(1);
                }
                if ui.button("Jangan Simpan").clicked() {
                    choice = Some(2);
                }
                if ui.button("Batal").clicked() {
                    choice = Some(0);
                }
            });
        });
    if ui.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::Escape)) {
        choice = Some(0);
    }
    match choice {
        Some(1) => match p.editor.save(root, i) {
            Ok(()) => p.editor.close(i),
            Err(e) => {
                actions.push(Action::Toast(e));
                p.editor.pending_close = None;
            }
        },
        Some(2) => p.editor.close(i),
        Some(_) => p.editor.pending_close = None,
        None => {}
    }
}

fn breadcrumbs(ui: &mut Ui, rel: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, vs::EDITOR_BG);
    let mut x = rect.left() + 14.0;
    let parts: Vec<&str> = rel.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        if last {
            paint_badge(ui, pos2(x + 6.0, rect.center().y), part, 11.0);
            x += 16.0;
        }
        let g = painter.layout_no_wrap(part.to_string(), FontId::proportional(12.5), vs::BREADCRUMB);
        let w = g.size().x;
        painter.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, vs::BREADCRUMB);
        x += w + 6.0;
        if !last {
            icons::paint(painter, Icon::ChevronRight, pos2(x + 3.0, rect.center().y), 10.0, vs::BREADCRUMB);
            x += 12.0;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn file_view(ui: &mut Ui, idx: usize, ed: &mut EditorState, a: usize, root: &Path, font: f32, now: f64, actions: &mut Vec<Action>) {
    let t = &mut ed.tabs[a];
    if let Some(live) = &t.live {
        let pulse = ((now * 4.0).sin() * 0.5 + 0.5) as f32;
        Frame::new()
            .fill(theme::LIVE.gamma_multiply(0.12 + 0.1 * pulse))
            .inner_margin(Margin::symmetric(14, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(format!("⚡ LIVE — agent sedang {} {} …", live.verb.to_lowercase(), t.rel))
                        .color(theme::LIVE)
                        .strong(),
                );
            });
    }
    let mut take_disk = false;
    let mut keep_mine = false;
    if t.conflict.is_some() {
        Frame::new()
            .fill(Color32::from_rgb(0x3a, 0x2a, 0x10))
            .inner_margin(Margin::symmetric(14, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠ File berubah di disk sementara ada editan yang belum disimpan.").color(theme::WARM));
                    take_disk = ui.button("Muat versi disk").clicked();
                    keep_mine = ui.button("Simpan punyaku").clicked();
                });
            });
    }
    if take_disk
        && let Some(disk) = t.conflict.take() {
            if let Some((s, e)) = changed_lines(&t.content, &disk) {
                t.flash = Some(Flash { start: s, end: e, at: now });
            }
            t.saved = disk.clone();
            t.content = disk;
        }
    if keep_mine
        && let Err(e) = ed.save(root, a) {
            actions.push(Action::Toast(e));
        }
    let t = &mut ed.tabs[a];

    if t.deleted {
        Frame::new()
            .fill(Color32::from_rgb(0x3a, 0x2a, 0x10))
            .inner_margin(Margin::symmetric(14, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("⚠ File ini dihapus di disk. Editan Anda masih di sini — Ctrl+S untuk membuatnya lagi.")
                        .color(theme::WARM),
                );
            });
    }

    if let Some(reason) = &t.readonly {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| ui.label(RichText::new(reason).color(vs::LINE_NO)));
        return;
    }

    let id = Id::unique(("code", idx, &t.rel));
    let live_text = t.live.as_ref().map(|l| (l.content.clone(), l.caret));
    let shown: &str = live_text.as_ref().map(|(c, _)| c.as_str()).unwrap_or(&t.content);
    let lines = shown.split('\n').count();

    // Minimap on the right, like VS Code.
    if lines < 30_000 {
        let content = shown.to_string();
        let syntax = t.syntax.clone();
        Panel::right(("minimap", idx))
            .frame(Frame::new().fill(vs::EDITOR_BG))
            .resizable(false)
            .exact_size(84.0)
            .show_separator_line(false)
            .show(ui, |ui| minimap(ui, t, &content, &syntax, font));
    }

    if let Some((content, caret)) = live_text {
        let caret_char = content[..caret.min(content.len())].chars().count();
        let mut text: &str = content.as_str();
        let opts = CodeOpts { font, interactive: false, flash: None, caret: Some(caret_char), jump: None, now };
        let out = code_view(ui, id.with("live"), &mut text, &t.syntax, &opts, t.scroll_to.take());
        t.scroll = out.scroll;
    } else {
        let flash = t.flash.as_ref().filter(|f| now - f.at < 2.5).map(|f| (f.start, f.end, f.at));
        let opts = CodeOpts { font, interactive: true, flash, caret: None, jump: t.jump.take(), now };
        let scroll_to = t.scroll_to.take();
        let tabs = uses_tabs(&t.content);
        let mut buf = CodeBuffer { text: &mut t.content, tabs };
        let out = code_view(ui, id, &mut buf, &t.syntax, &opts, scroll_to);
        if let Some(c) = out.cursor {
            t.cursor = c;
        }
        t.scroll = out.scroll;
        if t.flash.as_ref().is_some_and(|f| now - f.at >= 2.5) {
            t.flash = None;
        }
    }
}

#[derive(Clone, Copy)]
struct MiniRun {
    line: u32,
    col: u16,
    len: u16,
    color: Color32,
}

fn fnv64(s: &str) -> u64 {
    crate::config::fnv(s) ^ s.len() as u64
}

fn minimap_runs(job: &LayoutJob) -> Vec<MiniRun> {
    let mut out = Vec::new();
    let text = job.text.as_str();
    let mut line = 0u32;
    let mut col = 0u16;
    let sections: Vec<(std::ops::Range<usize>, Color32)> = if job.sections.is_empty() {
        vec![(0..text.len(), vs::TEXT)]
    } else {
        job.sections.iter().map(|s| (s.byte_range.start.0..s.byte_range.end.0, s.format.color)).collect()
    };
    for (range, color) in sections {
        let slice = text.get(range.clone()).unwrap_or("");
        let mut run_start: Option<u16> = None;
        for ch in slice.chars() {
            if ch == '\n' {
                if let Some(s) = run_start.take() {
                    out.push(MiniRun { line, col: s, len: col - s, color });
                }
                line += 1;
                col = 0;
                continue;
            }
            let w = if ch == '\t' { 4 } else { 1 };
            if ch.is_whitespace() {
                if let Some(s) = run_start.take() {
                    out.push(MiniRun { line, col: s, len: col - s, color });
                }
            } else if run_start.is_none() {
                run_start = Some(col);
            }
            col = col.saturating_add(w);
        }
        if let Some(s) = run_start {
            out.push(MiniRun { line, col: s, len: col - s, color });
        }
    }
    out
}

fn minimap(ui: &mut Ui, t: &mut OpenFile, content: &str, syntax: &str, font: f32) {
    let rect = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(rect, Sense::click_and_drag());
    let key = fnv64(content);
    if t.minimap.as_ref().is_none_or(|(k, _)| *k != key) {
        let job = theme::highlight_code(ui, font, content, syntax);
        t.minimap = Some((key, minimap_runs(&job)));
    }
    let Some((_, runs)) = &t.minimap else { return };
    let total_lines = content.split('\n').count().max(1) as f32;
    let lh = 2.0;
    let mm_h = total_lines * lh;
    let s = t.scroll;
    let ratio = if s.content > s.view { (s.offset / (s.content - s.view)).clamp(0.0, 1.0) } else { 0.0 };
    let mm_off = ((mm_h - rect.height()).max(0.0)) * ratio;
    let painter = ui.painter_at(rect);
    for r in runs {
        let y = rect.top() + r.line as f32 * lh - mm_off;
        if y < rect.top() - lh || y > rect.bottom() {
            continue;
        }
        let x = rect.left() + 6.0 + r.col as f32 * 1.1;
        if x > rect.right() {
            continue;
        }
        painter.rect_filled(
            Rect::from_min_size(pos2(x, y), vec2((r.len as f32 * 1.1).min(rect.right() - x), 1.4)),
            0.0,
            r.color.gamma_multiply(0.75),
        );
    }
    if s.content > 0.0 {
        let vy = rect.top() + s.offset / s.content * mm_h - mm_off;
        let vh = (s.view / s.content * mm_h).max(8.0);
        let a = if resp.hovered() || resp.dragged() { 40 } else { 22 };
        painter.rect_filled(Rect::from_min_size(pos2(rect.left(), vy), vec2(rect.width(), vh)), 0.0, Color32::from_white_alpha(a));
    }
    painter.vline(rect.left(), rect.y_range(), Stroke::new(1.0, Color32::from_black_alpha(80)));
    if let Some(pos) = resp.interact_pointer_pos()
        && (resp.clicked() || resp.dragged()) {
            let line_frac = ((pos.y - rect.top() + mm_off) / mm_h).clamp(0.0, 1.0);
            t.scroll_to = Some((line_frac * s.content - s.view / 2.0).max(0.0));
        }
}

/// Whether a file indents with tabs (otherwise spaces, VS Code's default).
pub fn uses_tabs(text: &str) -> bool {
    let (mut tabs, mut spaces) = (0, 0);
    for line in text.lines().take(2000) {
        if line.starts_with('\t') {
            tabs += 1;
        } else if line.starts_with("  ") {
            spaces += 1;
        }
    }
    tabs > spaces
}

/// Text buffer adapter that behaves like a code editor: Tab inserts spaces up
/// to the next tab stop (unless the file uses tabs) and Enter keeps the
/// current line's indentation, adding one level after an opening bracket.
pub struct CodeBuffer<'a> {
    pub text: &'a mut String,
    pub tabs: bool,
}

const TAB_WIDTH: usize = 4;

impl TextBuffer for CodeBuffer<'_> {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        self.text.as_str()
    }

    fn insert_text(&mut self, text: &str, char_index: egui::text::CharIndex) -> usize {
        let ci: usize = char_index.into();
        let byte = self.text.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(self.text.len());
        let line_start = self.text[..byte].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let insert = match text {
            "\t" if !self.tabs => {
                let col = self.text[line_start..byte].chars().count();
                " ".repeat(TAB_WIDTH - col % TAB_WIDTH)
            }
            "\n" => {
                let line = &self.text[line_start..byte];
                let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
                let opens = matches!(line.trim_end().chars().last(), Some('{' | '(' | '[' | ':'));
                let unit = if self.tabs { "\t".to_string() } else { " ".repeat(TAB_WIDTH) };
                format!("\n{indent}{}", if opens { unit.as_str() } else { "" })
            }
            _ => text.to_string(),
        };
        self.text.insert_str(byte, &insert);
        insert.chars().count()
    }

    fn delete_char_range(&mut self, char_range: std::ops::Range<egui::text::CharIndex>) {
        self.text.delete_char_range(char_range);
    }

    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<CodeBuffer<'static>>()
    }
}

pub struct CodeOpts {
    pub font: f32,
    pub interactive: bool,
    pub flash: Option<(usize, usize, f64)>,
    pub caret: Option<usize>,
    pub jump: Option<usize>,
    pub now: f64,
}

pub struct CodeOut {
    pub cursor: Option<(usize, usize)>,
    pub scroll: ScrollInfo,
}

/// Renders a code buffer VS Code style: line numbers, indent guides, current
/// line border, flash of agent changes and the agent's typing caret.
pub fn code_view(ui: &mut Ui, id: Id, text: &mut dyn TextBuffer, syntax: &str, o: &CodeOpts, scroll_to: Option<f32>) -> CodeOut {
    let font_id = FontId::monospace(o.font);
    let line_count = text.as_str().split('\n').count().max(1);
    let digits = line_count.to_string().len().max(3);
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font_id, ' ')).max(1.0);
    let gutter = (digits as f32 * char_w + 44.0).min(126.0);

    // Indent levels per line for the guides; blank lines take their neighbours'.
    let indents: Vec<usize> = {
        let raw: Vec<Option<usize>> = text
            .as_str()
            .split('\n')
            .map(|l| {
                if l.trim().is_empty() {
                    None
                } else {
                    Some(l.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { 4 } else { 1 }).sum::<usize>() / 4)
                }
            })
            .collect();
        let mut out = vec![0; raw.len()];
        let mut prev = 0;
        let mut next = vec![0; raw.len()];
        let mut nx = 0;
        for i in (0..raw.len()).rev() {
            if let Some(v) = raw[i] {
                nx = v;
            }
            next[i] = nx;
        }
        for i in 0..raw.len() {
            out[i] = match raw[i] {
                Some(v) => {
                    prev = v;
                    v
                }
                None => prev.min(next[i]),
            };
        }
        out
    };

    let mut layouter = |ui: &Ui, buf: &dyn TextBuffer, _wrap: f32| {
        let mut job = theme::highlight_code(ui, o.font, buf.as_str(), syntax);
        job.wrap.max_width = f32::INFINITY;
        ui.fonts_mut(|f| f.layout_job(job))
    };

    let mut cursor_pos = None;
    let mut area = ScrollArea::both().id_salt(id.with("scroll")).auto_shrink([false, false]);
    if let Some(y) = scroll_to {
        area = area.vertical_scroll_offset(y);
    }
    let out = area.show(ui, |ui| {
        let edit = TextEdit::multiline(text)
            .id(id)
            .font(font_id.clone())
            .code_editor()
            .lock_focus(true)
            .interactive(o.interactive)
            .desired_width(f32::INFINITY)
            .desired_rows(40)
            // A custom frame replaces `margin()`, so the gutter space lives here.
            .frame(Frame::NONE.inner_margin(Margin {
                left: gutter as i8,
                right: 12,
                top: 4,
                bottom: 60,
            }))
            .layouter(&mut layouter)
            .show(ui);

        let painter = ui.painter();
        let rect = edit.response.rect;
        let origin = edit.galley_pos;
        let clip = ui.clip_rect();
        let rows = &edit.galley.rows;
        let right = rect.right().max(clip.right());

        let cur_line = edit.cursor_range.map(|r| {
            let idx: usize = r.primary.index.into();
            text.as_str().chars().take(idx).filter(|&c| c == '\n').count()
        });

        // Current line border.
        if let Some(l) = cur_line.filter(|_| o.interactive)
            && let Some(row) = rows.get(l) {
                let y0 = origin.y + row.pos.y;
                painter.rect_stroke(
                    Rect::from_min_max(pos2(rect.left() + gutter - 2.0, y0), pos2(right, y0 + row.rect().height())),
                    0.0,
                    Stroke::new(2.0, vs::LINE_HL),
                    StrokeKind::Inside,
                );
            }

        // Flash rows the agent just changed.
        if let Some((start, end, at)) = o.flash
            && start < rows.len() {
                let end = end.min(rows.len()).max(start + 1);
                let y0 = origin.y + rows[start].pos.y;
                let y1 = origin.y + rows[end - 1].rect().max.y;
                let k = (1.0 - (o.now - at) / 2.5).clamp(0.0, 1.0) as f32;
                painter.rect_filled(
                    Rect::from_min_max(pos2(rect.left() + gutter - 4.0, y0), pos2(right, y1)),
                    0.0,
                    Color32::from_rgba_unmultiplied(80, 200, 120, (k * 50.0) as u8),
                );
                painter.rect_filled(
                    Rect::from_min_max(pos2(rect.left() + gutter - 8.0, y0), pos2(rect.left() + gutter - 5.0, y1)),
                    0.0,
                    Color32::from_rgba_unmultiplied(80, 200, 120, (k * 255.0) as u8),
                );
                ui.ctx().request_repaint();
            }

        // Line numbers and indent guides.
        let num_font = FontId::monospace(o.font * 0.92);
        let mut line = 0usize;
        let mut starts_line = true;
        for row in rows.iter() {
            let y = origin.y + row.pos.y;
            let h = row.rect().height();
            let visible = y + h >= clip.top() && y <= clip.bottom();
            if starts_line && visible {
                let color = if Some(line) == cur_line { vs::LINE_NO_ACTIVE } else { vs::LINE_NO };
                painter.text(
                    pos2(rect.left() + gutter - 22.0, y + h * 0.5),
                    Align2::RIGHT_CENTER,
                    (line + 1).to_string(),
                    num_font.clone(),
                    color,
                );
                if let Some(&lvl) = indents.get(line) {
                    for g in 0..lvl {
                        let x = origin.x + g as f32 * 4.0 * char_w + 0.5;
                        painter.vline(x, y..=y + h, Stroke::new(1.0, vs::INDENT_GUIDE));
                    }
                }
            }
            if starts_line {
                line += 1;
            }
            starts_line = row.ends_with_newline;
        }

        // Jump to a line requested by search / quick open.
        if let Some(l) = o.jump {
            if let Some(row) = rows.get(l.min(rows.len().saturating_sub(1))) {
                let r = Rect::from_min_size(pos2(origin.x, origin.y + row.pos.y), vec2(10.0, row.rect().height()));
                ui.scroll_to_rect(r, Some(Align::Center));
            }
            let char_idx: usize = text.as_str().split('\n').take(l).map(|s| s.chars().count() + 1).sum();
            // A file opened this frame has no stored state yet; create it.
            let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
            state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(char_idx))));
            state.store(ui.ctx(), id);
            ui.memory_mut(|m| m.request_focus(id));
        }

        // The agent's typing caret, kept in view.
        if let Some(c) = o.caret {
            let r = edit.galley.pos_from_cursor(CCursor::new(c)).translate(origin.to_vec2());
            let block = Rect::from_min_size(r.min, vec2(char_w, r.height().max(o.font)));
            if (o.now * 2.5) as i64 % 2 == 0 {
                painter.rect_filled(block, 0.0, theme::LIVE);
            }
            painter.rect_filled(
                Rect::from_min_max(pos2(rect.left() + gutter - 4.0, r.min.y), pos2(right, r.max.y)),
                0.0,
                theme::LIVE.gamma_multiply(0.07),
            );
            ui.scroll_to_rect(block.expand(60.0), Some(Align::Center));
            ui.ctx().request_repaint();
        }

        if let Some(r) = edit.cursor_range {
            let idx: usize = r.primary.index.into();
            let before: String = text.as_str().chars().take(idx).collect();
            let line = before.matches('\n').count() + 1;
            let col = before.rsplit('\n').next().map(|s| s.chars().count()).unwrap_or(0) + 1;
            cursor_pos = Some((line, col));
        }
    });
    CodeOut {
        cursor: cursor_pos,
        scroll: ScrollInfo {
            offset: out.state.clamped_offset().y,
            content: out.content_size.y,
            view: out.inner_rect.height(),
        },
    }
}

// ---------------------------------------------------------------- overlays

fn all_files(node: &FileNode, out: &mut Vec<String>) {
    for c in &node.children {
        if c.is_dir {
            all_files(c, out);
        } else {
            out.push(c.rel.clone());
        }
    }
}

/// Subsequence fuzzy match; higher is better.
fn fuzzy(query: &str, cand: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let c = cand.to_lowercase();
    let name_start = c.rfind('/').map(|i| i + 1).unwrap_or(0);
    let mut qi = 0;
    let mut score = 0;
    let mut last = None;
    for (i, ch) in c.char_indices() {
        if qi < q.len() && ch == q[qi] {
            score += 1;
            if last.is_some_and(|l: usize| l + 1 == i) {
                score += 3;
            }
            if i >= name_start {
                score += 2;
            }
            last = Some(i);
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score - c.len() as i32 / 10)
}

fn quick_open(ui: &mut Ui, p: &mut Project, open_req: &mut Option<(String, Option<usize>)>) {
    let Some(q) = &mut p.editor.quick else { return };
    let mut files = Vec::new();
    if let Some(s) = &p.scan {
        all_files(&s.root, &mut files);
    }
    let mut ranked: Vec<(i32, String)> = files.into_iter().filter_map(|f| fuzzy(&q.query, &f).map(|s| (s, f))).collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.len().cmp(&b.1.len())));
    ranked.truncate(14);
    let (up, down, enter, esc) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Escape),
        )
    });
    if up {
        q.sel = q.sel.saturating_sub(1);
    }
    if down && q.sel + 1 < ranked.len() {
        q.sel += 1;
    }
    q.sel = q.sel.min(ranked.len().saturating_sub(1));
    let mut close = esc;
    if enter {
        if let Some((_, f)) = ranked.get(q.sel) {
            *open_req = Some((f.clone(), None));
        }
        close = true;
    }
    egui::Area::new(Id::unique("quick_open"))
        .order(Order::Foreground)
        .anchor(Align2::CENTER_TOP, vec2(0.0, 48.0))
        .show(ui.ctx(), |ui| {
            Frame::new()
                .fill(vs::SIDEBAR_BG)
                .stroke(Stroke::new(1.0, Color32::from_rgb(0x45, 0x45, 0x45)))
                .corner_radius(6)
                .inner_margin(Margin::same(6))
                .show(ui, |ui| {
                    ui.set_width(600.0);
                    let r = ui.add(
                        TextEdit::singleline(&mut q.query)
                            .hint_text("Cari file berdasarkan nama (ketik untuk filter)")
                            .desired_width(f32::INFINITY),
                    );
                    r.request_focus();
                    if r.changed() {
                        q.sel = 0;
                    }
                    ui.add_space(4.0);
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    if ranked.is_empty() {
                        ui.label(RichText::new("  Tidak ada file yang cocok").color(vs::LINE_NO));
                    }
                    for (i, (_, f)) in ranked.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                        if i == q.sel {
                            ui.painter().rect_filled(rect, 3.0, vs::LIST_FOCUS);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(rect, 3.0, vs::LIST_HOVER);
                        }
                        paint_badge(ui, pos2(rect.left() + 14.0, rect.center().y), f, 12.0);
                        let name = f.rsplit('/').next().unwrap_or(f);
                        let g = ui.painter().layout_no_wrap(name.to_string(), FontId::proportional(13.0), vs::TEXT_BRIGHT);
                        let w = g.size().x;
                        ui.painter().galley(pos2(rect.left() + 28.0, rect.center().y - g.size().y / 2.0), g, vs::TEXT_BRIGHT);
                        ui.painter().text(pos2(rect.left() + 36.0 + w, rect.center().y), Align2::LEFT_CENTER, f, FontId::proportional(11.5), vs::LINE_NO);
                        if resp.clicked() {
                            *open_req = Some((f.clone(), None));
                            close = true;
                        }
                    }
                });
        });
    if close {
        p.editor.quick = None;
    }
}

fn status_bar(ui: &mut Ui, p: &Project, clock_label: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        if let Some(b) = &p.editor.branch {
            icons::status_item(ui, Some(Icon::Branch), b);
        }
        if p.agent.is_some() {
            ui.add(egui::Spinner::new().size(11.0).color(Color32::WHITE));
            icons::status_item(ui, None, &format!("Agent: {}", p.status));
        } else {
            icons::status_item(ui, Some(Icon::Robot), "Agent siap");
        }
        if p.has_skill {
            icons::status_item(ui, Some(Icon::Sparkle), "Skill")
                .on_hover_text("Agent membaca .ckdtr/skill.md di awal setiap tugas");
        }
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            icons::status_item(ui, Some(Icon::Bell), "");
            icons::status_item(ui, Some(Icon::Clock), clock_label);
            if let Some(t) = p.editor.active.and_then(|a| p.editor.tabs.get(a)) {
                icons::status_item(ui, None, t.syntax.trim_end_matches(" (Babel)"));
                icons::status_item(ui, None, if t.content.contains("\r\n") { "CRLF" } else { "LF" });
                icons::status_item(ui, None, "UTF-8");
                let spaces = if uses_tabs(&t.content) { "Tab Size: 4" } else { "Spaces: 4" };
                icons::status_item(ui, None, spaces);
                icons::status_item(ui, None, &format!("Ln {}, Col {}", t.cursor.0, t.cursor.1));
                if t.dirty() {
                    icons::status_item(ui, None, "● belum disimpan");
                }
            } else if let Some(s) = &p.scan {
                let text = format!("Lv {} · {} file · {} baris", p.level(), project::fmt_count(s.stats.files), project::fmt_count(s.stats.lines));
                icons::status_item(ui, None, &text);
            }
        });
    });
}

fn welcome(ui: &mut Ui, idx: usize, p: &Project, now: f64, actions: &mut Vec<Action>) {
    let rect = ui.available_rect_before_wrap();
    let painter = ui.painter_at(rect);
    let left = rect.left() + (rect.width() * 0.08).max(30.0);
    let top = rect.top() + (rect.height() * 0.1).max(30.0);
    painter.text(pos2(left, top), Align2::LEFT_TOP, &p.entry.name, FontId::proportional(34.0), vs::TEXT_BRIGHT);
    let stats = match &p.scan {
        Some(s) => format!(
            "Level {} · {} file · {} baris kode",
            p.level(),
            project::fmt_count(s.stats.files),
            project::fmt_count(s.stats.lines)
        ),
        None => "Memindai project…".into(),
    };
    painter.text(pos2(left, top + 48.0), Align2::LEFT_TOP, stats, FontId::proportional(15.0), vs::LINE_NO);
    if let Some((generated, description)) = &p.skill_info {
        let y = top + 74.0;
        icons::paint(&painter, Icon::Sparkle, pos2(left + 7.0, y + 8.0), 14.0, theme::WARM);
        let mut line = String::from("Skill project");
        if !generated.is_empty() {
            line.push_str(&format!(" · dibuat {generated}"));
        }
        if !description.is_empty() {
            let d: String = description.chars().take(90).collect();
            line.push_str(&format!(" — {d}"));
        }
        painter.text(pos2(left + 20.0, y), Align2::LEFT_TOP, line, FontId::proportional(13.0), vs::BREADCRUMB);
    }
    let gap = if p.skill_info.is_some() { 22.0 } else { 0.0 };
    painter.text(pos2(left, top + 96.0 + gap), Align2::LEFT_TOP, "Mulai", FontId::proportional(18.0), vs::TEXT_BRIGHT);

    let mut y = top + 128.0 + gap;
    let link = |ui: &mut Ui, y: f32, icon: Icon, text: &str| -> bool {
        let r = Rect::from_min_size(pos2(left, y), vec2(320.0, 24.0));
        let resp = ui.interact(r, Id::unique(("welcome", text)), Sense::click());
        let color = if resp.hovered() { Color32::from_rgb(0x4d, 0xaa, 0xfc) } else { Color32::from_rgb(0x37, 0x94, 0xff) };
        icons::paint(ui.painter(), icon, pos2(left + 8.0, y + 12.0), 16.0, color);
        ui.painter().text(pos2(left + 26.0, y + 12.0), Align2::LEFT_CENTER, text, FontId::proportional(14.0), color);
        resp.clicked()
    };
    if link(ui, y, Icon::File, "File Baru…") {
        actions.push(Action::NewFile);
    }
    y += 28.0;
    if link(ui, y, Icon::Search, "Buka File Cepat…  (Ctrl+P)") {
        actions.push(Action::QuickOpen);
    }
    y += 28.0;
    if link(ui, y, Icon::Folder, "Buka di File Explorer") {
        actions.push(Action::RevealCurrent);
    }
    y += 28.0;
    if link(ui, y, Icon::Home, "Kembali ke kantor") {
        actions.push(Action::BackToWorld);
    }
    y += 28.0;
    if p.agent.is_none() {
        let label = if p.has_skill { "Perbarui skill project (.ckdtr)" } else { "Buat skill project (.ckdtr)" };
        if link(ui, y, Icon::Sparkle, label) {
            actions.push(Action::MakeSkill(idx));
        }
        y += 28.0;
    }
    if p.has_skill && link(ui, y, Icon::File, "Lihat skill project") {
        actions.push(Action::OpenSkill(idx));
    }
    y += 52.0;
    painter.text(pos2(left, y), Align2::LEFT_TOP, "Tips", FontId::proportional(18.0), vs::TEXT_BRIGHT);
    for (k, tip) in [
        "Beri tugas di panel Chat kanan — kodenya berubah live di sini.",
        "Ctrl+P buka file · Ctrl+Shift+F cari · Ctrl+B sembunyikan side bar.",
        "Ctrl+S simpan · Ctrl+W tutup tab · Esc kembali ke kantor.",
    ]
    .iter()
    .enumerate()
    {
        painter.text(pos2(left, y + 32.0 + k as f32 * 24.0), Align2::LEFT_TOP, *tip, FontId::proportional(13.5), vs::TEXT);
    }

    // The employee at their desk.
    let u = 7.0;
    let o = pos2(rect.right() - (rect.width() * 0.08).max(30.0) - 33.0 * u, rect.center().y - 4.0 * u);
    // Only when it fits beside the tips text.
    if o.x - 16.0 * u > left + 500.0 {
        office::desk_preview(&painter, o, u, p, now);
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::text::CharIndex;

    fn temp_project(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cokandetor-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        dir
    }

    #[test]
    fn tab_inserts_spaces_to_the_next_stop() {
        let mut s = String::from("ab");
        let mut b = CodeBuffer { text: &mut s, tabs: false };
        let n = b.insert_text("\t", CharIndex(2));
        assert_eq!((n, s.as_str()), (2, "ab  "));
        let mut s = String::from("x");
        let mut b = CodeBuffer { text: &mut s, tabs: true };
        b.insert_text("\t", CharIndex(0));
        assert_eq!(s, "\tx");
    }

    #[test]
    fn enter_keeps_and_increases_indentation() {
        let mut s = String::from("fn main() {\n    let a = [");
        let end = s.chars().count();
        let mut b = CodeBuffer { text: &mut s, tabs: false };
        let n = b.insert_text("\n", CharIndex(end));
        assert_eq!(&s[s.len() - 9..], "\n        ");
        assert_eq!(n, 9);
        let mut s = String::from("    x = 1");
        let mut b = CodeBuffer { text: &mut s, tabs: false };
        b.insert_text("\n", CharIndex(9));
        assert_eq!(s, "    x = 1\n    ");
    }

    #[test]
    fn open_refuses_paths_outside_the_project() {
        let root = temp_project("open");
        std::fs::write(root.join("src/a.rs"), "fn a() {}").unwrap();
        let mut ed = EditorState::default();
        assert!(ed.open(&root, "../escape.txt", true).is_none());
        assert!(ed.open(&root, "C:/Windows/win.ini", true).is_none());
        let i = ed.open(&root, "./src//a.rs", true).unwrap();
        assert_eq!(ed.tabs[i].rel, "src/a.rs");
        let abs = root.join("src").join("a.rs");
        assert_eq!(ed.open(&root, &abs.to_string_lossy(), true), Some(i));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn conflicts_and_deletions_keep_user_edits() {
        let root = temp_project("conflict");
        std::fs::write(root.join("src/a.rs"), "agent v1").unwrap();
        std::fs::write(root.join("src/b.rs"), "b").unwrap();
        let mut ed = EditorState::default();
        let a = ed.open(&root, "src/a.rs", true).unwrap();
        let b = ed.open(&root, "src/b.rs", true).unwrap();
        ed.tabs[a].content = "mine".into();
        ed.apply_change("src/a.rs", Some("agent v2"), 0.0);
        assert!(ed.tabs[a].conflict.is_some());
        ed.tabs[b].content = "edited b".into();
        // Sending a prompt saves b but must not resolve a's conflict.
        assert_eq!(ed.save_all(&root), (1, 1));
        assert_eq!(std::fs::read_to_string(root.join("src/a.rs")).unwrap(), "agent v1");
        assert_eq!(std::fs::read_to_string(root.join("src/b.rs")).unwrap(), "edited b");
        // The agent deletes b while it has unsaved edits: they stay editable.
        ed.tabs[b].content = "more edits".into();
        ed.apply_change("src/b.rs", None, 0.0);
        assert!(ed.tabs[b].deleted && ed.tabs[b].readonly.is_none() && ed.tabs[b].dirty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn uses_tabs_detects_indentation_style() {
        assert!(uses_tabs("func main() {\n\tx := 1\n\ty := 2\n}"));
        assert!(!uses_tabs("fn main() {\n    let x = 1;\n}"));
    }
}
