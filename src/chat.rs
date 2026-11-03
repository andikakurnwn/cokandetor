//! The agent chat panel shown beside the editor.

use egui::{Color32, CornerRadius, Frame, Id, Key, Margin, Modifiers, RichText, ScrollArea, TextEdit, Ui};
use serde::{Deserialize, Serialize};

use crate::agent::usage;
use crate::app::{Action, Project};
use crate::config::{ProviderKind, Settings};
use crate::icons::{Icon, IconButton, Kind};
use crate::theme;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToolStatus {
    Running,
    Ok,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ChatItem {
    User(String),
    Assistant(String),
    Thinking(String),
    Tool {
        id: String,
        name: String,
        path: String,
        status: ToolStatus,
        summary: String,
    },
    Info(String),
    Error(String),
}

pub fn tool_label(name: &str) -> (&'static str, &'static str) {
    match name {
        "list_files" => ("📂", "Melihat struktur"),
        "read_file" => ("👀", "Membaca"),
        "write_file" => ("✍", "Menulis"),
        "edit_file" => ("✏", "Mengedit"),
        "delete_file" => ("🗑", "Menghapus"),
        "search_files" => ("🔍", "Mencari"),
        "run_command" => ("▶", "Menjalankan"),
        _ => ("🔧", "Tool"),
    }
}

/// Provider picker + model field, shared by the chat panel and the world prompt bar.
/// Returns true when settings changed.
pub fn provider_picker(ui: &mut Ui, settings: &mut Settings, id: &str) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(settings.provider.short())
        .width(96.0)
        .show_ui(ui, |ui| {
            for k in ProviderKind::ALL {
                changed |= ui
                    .selectable_value(&mut settings.provider, k, k.label())
                    .changed();
            }
        });
    let model = settings.model_mut();
    changed |= ui
        .add(TextEdit::singleline(model).desired_width(130.0).hint_text("model"))
        .on_hover_text("Nama model, mis. claude-opus-5-5 / gpt-5 / qwen2.5-coder")
        .changed();
    changed
}

pub fn panel(
    ui: &mut Ui,
    idx: usize,
    p: &mut Project,
    settings: &mut Settings,
    actions: &mut Vec<Action>,
    open_req: &mut Option<String>,
) {
    ui.add_space(6.0);
    ui.label(RichText::new("CHAT AGENT").size(11.0).color(theme::vs::SIDEBAR_TITLE));
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if provider_picker(ui, settings, "chat_provider") {
            actions.push(Action::SaveConfig);
        }
    });
    ui.horizontal(|ui| {
        if p.agent.is_some() {
            ui.spinner();
            ui.label(RichText::new(&p.status).color(theme::WARM));
        } else {
            ui.label(RichText::new("● Siap").color(theme::OK));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .checkbox(&mut settings.follow_agent, "Ikuti agent")
                .on_hover_text("Otomatis buka file yang sedang ditulis agent dan tampilkan perubahan live")
                .changed()
            {
                actions.push(Action::SaveConfig);
            }
        });
    });
    usage_bar(ui, p);
    ui.separator();

    let input_h = 118.0;
    let avail = ui.available_height();
    ScrollArea::vertical()
        .id_salt(("chat_scroll", idx))
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .max_height((avail - input_h).max(80.0))
        .show(ui, |ui| {
            if p.chat.is_empty() {
                ui.add_space(20.0);
                ui.label(
                    RichText::new(
                        "Beri tugas ke karakter ini.\nContoh: \"Tambahkan fungsi login dan unit test-nya\"\n\nPerubahan kode akan muncul live di editor.",
                    )
                    .weak()
                    .italics(),
                );
            }
            for (i, item) in p.chat.iter().enumerate() {
                item_ui(ui, i, item, open_req);
            }
        });

    ui.separator();
    let input_id = Id::unique(("chat_input", idx));
    let focused = ui.memory(|m| m.has_focus(input_id));
    let submit = focused && ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Enter));
    ui.add(
        TextEdit::multiline(&mut p.input)
            .id(input_id)
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .hint_text("Minta agent… (Ctrl+Enter untuk kirim)"),
    );
    ui.horizontal(|ui| {
        let working = p.agent.is_some();
        if working {
            if ui
                .add(IconButton::new(Icon::Stop, "Stop").kind(Kind::Danger))
                .clicked()
            {
                actions.push(Action::Stop(idx));
            }
        } else {
            let can = !p.input.trim().is_empty();
            let send = ui.add_enabled(
                can,
                IconButton::new(Icon::Send, "Kirim").kind(Kind::Primary),
            );
            if (send.clicked() || submit) && can {
                actions.push(Action::Send {
                    project: idx,
                    prompt: std::mem::take(&mut p.input).trim().to_string(),
                });
            }
        }
        if ui
            .add_enabled(!working && !p.history.is_empty(), IconButton::new(Icon::Refresh, "Konteks baru"))
            .on_hover_text(
                "Agent memulai percakapan baru; log chat tetap ada. Tugas berikutnya tidak                  mengirim ulang riwayat lama, jadi lebih hemat. Skill project tetap dipakai.",
            )
            .clicked()
        {
            actions.push(Action::NewContext(idx));
        }
        if ui
            .add_enabled(!working && !p.chat.is_empty(), IconButton::new(Icon::Trash, "Bersihkan"))
            .on_hover_text("Hapus percakapan & mulai sesi baru")
            .clicked()
        {
            actions.push(Action::ClearChat(idx));
        }
    });
}

/// Above this many tokens, resending the conversation with every task gets costly.
const BIG_CONTEXT: u64 = 50_000;

/// Token meter: what this project has used so far, and how big the agent's
/// conversation is (what the next task sends again).
fn usage_bar(ui: &mut Ui, p: &Project) {
    let u = &p.usage_total;
    ui.horizontal(|ui| {
        let total = if u.calls == 0 {
            "Belum ada pemakaian token".to_string()
        } else {
            let cost = if p.cost_partial && p.cost_total == 0.0 {
                usage::fmt_cost(None)
            } else if p.cost_partial {
                format!("≥ {}", usage::fmt_usd(p.cost_total))
            } else {
                usage::fmt_cost(Some(p.cost_total))
            };
            format!("Σ {} token · {cost}", usage::fmt_tokens(u.prompt() + u.output))
        };
        ui.label(RichText::new(total).small().color(theme::TEXT_DIM)).on_hover_text(format!(
            "Pemakaian project ini:
             • input harga penuh: {}
             • ditulis ke cache: {}
             • dibaca dari cache: {} ({}% dari input)
             • output: {}
             • panggilan model: {}

             Biaya adalah perkiraan dari harga daftar per juta token.",
            usage::fmt_tokens(u.input),
            usage::fmt_tokens(u.cache_write),
            usage::fmt_tokens(u.cache_read),
            u.cached_pct(),
            usage::fmt_tokens(u.output),
            u.calls,
        ));
        if p.context_tokens > 0 {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let color = if p.context_tokens >= BIG_CONTEXT { theme::WARM } else { theme::TEXT_DIM };
                ui.label(RichText::new(format!("Konteks {}", usage::fmt_tokens(p.context_tokens))).small().color(color))
                    .on_hover_text("Ukuran percakapan agent: sebanyak ini token dikirim lagi di awal tugas berikutnya.");
            });
        }
    });
    if p.context_tokens >= BIG_CONTEXT && p.agent.is_none() {
        ui.label(
            RichText::new("Konteks sudah besar — bila tugas berikutnya tidak berhubungan, klik Konteks baru agar lebih hemat.")
                .small()
                .color(theme::WARM),
        );
    }
}

fn bubble(fill: Color32) -> Frame {
    Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(8, 6))
}

fn item_ui(ui: &mut Ui, i: usize, item: &ChatItem, open_req: &mut Option<String>) {
    match item {
        ChatItem::User(t) => {
            ui.add_space(4.0);
            bubble(theme::USER_BUBBLE).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Kamu").small().strong().color(Color32::from_rgb(0x4d, 0xaa, 0xfc)));
                ui.label(t);
            });
        }
        ChatItem::Assistant(t) => {
            ui.add_space(2.0);
            bubble(theme::AGENT_BUBBLE).show(ui, |ui| {
                ui.set_width(ui.available_width());
                markdown_lite(ui, t);
            });
        }
        ChatItem::Thinking(t) => {
            egui::CollapsingHeader::new(RichText::new("berpikir…").weak().small())
                .id_salt(("think", i))
                .default_open(false)
                .show(ui, |ui| {
                    ui.label(RichText::new(t).weak().italics().small());
                });
        }
        ChatItem::Tool {
            name,
            path,
            status,
            summary,
            ..
        } => {
            let (icon, verb) = tool_label(name);
            ui.horizontal_wrapped(|ui| {
                let (mark, color) = match status {
                    ToolStatus::Running => ("⏳", theme::WARM),
                    ToolStatus::Ok => ("✔", theme::OK),
                    ToolStatus::Failed => ("✖", theme::DANGER),
                };
                ui.label(RichText::new(mark).color(color));
                ui.label(RichText::new(format!("{icon} {verb}")).color(theme::TEXT_DIM));
                if !path.is_empty() {
                    let file_like = matches!(name.as_str(), "read_file" | "write_file" | "edit_file");
                    if file_like {
                        if ui
                            .link(RichText::new(path).monospace().color(theme::ACCENT))
                            .clicked()
                        {
                            *open_req = Some(path.clone());
                        }
                    } else {
                        ui.label(RichText::new(path).monospace().weak());
                    }
                }
                if !summary.is_empty() {
                    ui.label(RichText::new(format!("· {summary}")).small().weak());
                }
            });
        }
        ChatItem::Info(t) => {
            ui.label(RichText::new(t).small().weak().italics());
        }
        ChatItem::Error(t) => {
            bubble(Color32::from_rgb(0x3a, 0x16, 0x1e)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(format!("⚠ {t}")).color(Color32::from_rgb(0xff, 0x9a, 0x9a)));
            });
        }
    }
}

/// Plain text with ``` fenced code blocks rendered in monospace.
fn markdown_lite(ui: &mut Ui, text: &str) {
    let mut in_code = false;
    let mut buf = String::new();
    let flush = |ui: &mut Ui, buf: &mut String, code: bool| {
        let t = buf.trim_end_matches('\n');
        if !t.is_empty() {
            if code {
                Frame::new()
                    .fill(theme::CODE_BG)
                    .corner_radius(CornerRadius::same(3))
                    .inner_margin(Margin::same(6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(egui::Label::new(RichText::new(t).monospace().size(12.5)).selectable(true));
                    });
            } else {
                ui.add(egui::Label::new(t).selectable(true));
            }
        }
        buf.clear();
    };
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            flush(ui, &mut buf, in_code);
            in_code = !in_code;
            continue;
        }
        buf.push_str(line);
    }
    flush(ui, &mut buf, in_code);
}
