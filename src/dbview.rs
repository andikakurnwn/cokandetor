//! Database view opened from a storage cabinet: tables in the side bar, a SQL
//! editor, and a result grid — all in the VS Code look.

use egui::{
    Align, Align2, CentralPanel, Color32, FontId, Frame, Id, Key, Margin, Modifiers, Panel, Rect, RichText, ScrollArea, Sense,
    Stroke, Ui, pos2, vec2,
};
use egui_extras::{Column, TableBuilder};

use crate::app::Action;
use crate::db::{DbState, Status, TableInfo};
use crate::editor::{self, CodeOpts};
use crate::icons::{self, Icon, IconButton, Kind};
use crate::theme::{self, vs};

pub fn show(ui: &mut Ui, idx: usize, d: &mut DbState, font: f32, clock_label: &str, now: f64, actions: &mut Vec<Action>) {
    let ctx = ui.ctx().clone();

    Panel::bottom("db_status")
        .frame(Frame::new().fill(if d.status == Status::Busy { vs::STATUS_BUSY } else { vs::STATUS_BG }).inner_margin(Margin::symmetric(8, 2)))
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                icons::status_item(ui, Some(Icon::Database), &d.cfg.name);
                let state = match d.status {
                    Status::Idle => "belum terhubung".to_string(),
                    Status::Busy => "memproses…".to_string(),
                    Status::Connected => format!("terhubung · {}", short_version(&d.version)),
                    Status::Error => "error".to_string(),
                };
                icons::status_item(ui, None, &state);
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    icons::status_item(ui, Some(Icon::Clock), clock_label);
                    icons::status_item(ui, None, d.cfg.kind.label());
                    if let Some(r) = &d.result {
                        icons::status_item(ui, None, &format!("{} baris · {} ms", r.rows.len(), r.elapsed_ms));
                    }
                });
            });
        });

    Panel::left("db_activity")
        .frame(Frame::new().fill(vs::ACTIVITY_BG))
        .resizable(false)
        .exact_size(48.0)
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            editor::activity_button(ui, Icon::Database, true, "Database");
            ui.with_layout(egui::Layout::bottom_up(Align::Center), |ui| {
                if editor::activity_button(ui, Icon::Gear, false, "Pengaturan").clicked() {
                    actions.push(Action::OpenSettings);
                }
                if editor::activity_button(ui, Icon::Home, false, "Kembali ke kantor (Esc)").clicked() {
                    actions.push(Action::BackToWorld);
                }
            });
        });

    Panel::left("db_sidebar")
        .frame(Frame::new().fill(vs::SIDEBAR_BG))
        .resizable(true)
        .default_size(280.0)
        .size_range(200.0..=520.0)
        .show_separator_line(false)
        .show(ui, |ui| sidebar(ui, idx, d, &ctx, actions));

    CentralPanel::no_frame()
        .frame(Frame::new().fill(vs::EDITOR_BG))
        .show(ui, |ui| {
            // Toolbar.
            let run_key = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
            Frame::new().fill(vs::TAB_BAR_BG).inner_margin(Margin::symmetric(10, 5)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let run = ui
                        .add(IconButton::new(Icon::Play, "Jalankan").kind(Kind::Primary))
                        .on_hover_text("Ctrl+Enter");
                    if run.clicked() || run_key {
                        let sql = d.sql.clone();
                        d.run(&ctx, sql);
                    }
                    egui::ComboBox::from_id_salt(("db_history", idx))
                        .selected_text("Riwayat")
                        .width(90.0)
                        .show_ui(ui, |ui| {
                            for h in d.history.iter().rev() {
                                let label: String = h.chars().take(60).collect();
                                if ui.selectable_label(false, label.replace('\n', " ")).clicked() {
                                    d.sql = h.clone();
                                }
                            }
                        });
                    if d.status == Status::Busy {
                        ui.spinner();
                    }
                    ui.label(RichText::new(format!("{} — {}", d.cfg.kind.label(), d.cfg.target())).size(12.0).color(vs::LINE_NO));
                });
            });

            Panel::top(("sql_editor", idx))
                .frame(Frame::new().fill(vs::EDITOR_BG))
                .resizable(true)
                .default_size(220.0)
                .size_range(80.0..=600.0)
                .show(ui, |ui| {
                    let opts = CodeOpts { font, interactive: true, flash: None, caret: None, jump: None, now };
                    let mut buf = editor::CodeBuffer { text: &mut d.sql, tabs: false };
                    editor::code_view(ui, Id::unique(("sql", idx)), &mut buf, "sql", &opts, None);
                });

            results(ui, d);
        });
}

fn short_version(v: &str) -> String {
    v.split(" on ").next().unwrap_or(v).chars().take(40).collect()
}

fn sidebar(ui: &mut Ui, idx: usize, d: &mut DbState, ctx: &egui::Context, actions: &mut Vec<Action>) {
    editor::sidebar_title(ui, "DATABASE", |ui| {
        if icons::icon_button(ui, Icon::Refresh, "Muat ulang daftar tabel").clicked() {
            d.refresh_tables(ctx);
        }
        if icons::icon_button(ui, Icon::Edit, "Edit koneksi").clicked() {
            actions.push(Action::EditDb(idx));
        }
    });

    // Connection card.
    Frame::new()
        .fill(Color32::from_rgb(0x2a, 0x2a, 0x2c))
        .corner_radius(4)
        .inner_margin(Margin::same(10))
        .outer_margin(Margin::symmetric(10, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(d.cfg.kind.code()).monospace().color(Color32::from_rgb(0xf2, 0x91, 0x11)));
                ui.label(RichText::new(&d.cfg.name).strong().color(vs::TEXT_BRIGHT));
            });
            ui.label(RichText::new(d.cfg.target()).size(11.5).color(vs::LINE_NO));
            let (led, text) = match d.status {
                Status::Idle => (vs::LINE_NO, "Belum terhubung".to_string()),
                Status::Busy => (theme::WARM, "Memproses…".to_string()),
                Status::Connected => (theme::OK, short_version(&d.version)),
                Status::Error => (theme::DANGER, "Error".to_string()),
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new("●").color(led));
                ui.label(RichText::new(text).size(12.0).color(vs::TEXT));
            });
            ui.horizontal(|ui| {
                if ui.button("Hubungkan").clicked() {
                    d.connect(ctx);
                }
                if ui.add_enabled(d.status != Status::Idle, egui::Button::new("Putuskan")).clicked() {
                    d.disconnect();
                }
            });
        });
    ui.add_space(6.0);

    let views = d.tables.iter().filter(|t| t.is_view).count();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    icons::paint(ui.painter(), Icon::ChevronDown, pos2(rect.left() + 14.0, rect.center().y), 12.0, vs::TEXT_BRIGHT);
    ui.painter().text(
        pos2(rect.left() + 24.0, rect.center().y),
        Align2::LEFT_CENTER,
        format!("TABEL ({}{})", d.tables.len() - views, if views > 0 { format!(" + {views} view") } else { String::new() }),
        FontId::proportional(11.5),
        vs::TEXT_BRIGHT,
    );

    let mut open: Option<TableInfo> = None;
    let mut toggle: Option<TableInfo> = None;
    let multi_schema = d.tables.iter().any(|t| t.schema != d.tables[0].schema);
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        if d.tables.is_empty() && d.status != Status::Busy {
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(RichText::new("Klik Hubungkan untuk memuat tabel.").size(12.0).color(vs::LINE_NO));
            });
        }
        for t in &d.tables {
            let key = t.key();
            let expanded = d.expanded.contains(&key);
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, 0.0, vs::LIST_HOVER);
            }
            let chev = Rect::from_min_size(rect.min, vec2(26.0, 22.0));
            let chevron = if expanded { Icon::ChevronDown } else { Icon::ChevronRight };
            icons::paint(ui.painter(), chevron, chev.center(), 12.0, vs::TEXT);
            ui.painter().text(
                pos2(rect.left() + 32.0, rect.center().y),
                Align2::CENTER_CENTER,
                if t.is_view { "◫" } else { "▦" },
                FontId::proportional(12.0),
                if t.is_view { Color32::from_rgb(0xa0, 0x74, 0xc4) } else { Color32::from_rgb(0x4e, 0xc9, 0xb0) },
            );
            let label = if multi_schema { format!("{}.{}", t.schema, t.name) } else { t.name.clone() };
            ui.painter().text(pos2(rect.left() + 44.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.0), vs::TEXT);
            if resp.clicked() {
                if resp.interact_pointer_pos().is_some_and(|p| chev.contains(p)) {
                    toggle = Some(t.clone());
                } else {
                    open = Some(t.clone());
                }
            }
            let resp = resp.on_hover_text("Klik: tampilkan 100 baris · panah: lihat kolom");
            resp.context_menu(|ui| {
                if ui.button("SELECT 100 baris").clicked() {
                    open = Some(t.clone());
                    ui.close();
                }
                if ui.button("Hitung baris (COUNT)").clicked() {
                    d.sql = format!("SELECT COUNT(*) FROM {};", t.qualified(d.cfg.kind));
                    ui.close();
                }
            });
            if expanded {
                match d.columns.get(&key) {
                    Some(cols) => {
                        for c in cols {
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
                            ui.painter().vline(r.left() + 30.0, r.y_range(), Stroke::new(1.0, Color32::from_rgb(0x40, 0x40, 0x40)));
                            ui.painter().text(pos2(r.left() + 44.0, r.center().y), Align2::LEFT_CENTER, &c.name, FontId::proportional(12.5), Color32::from_rgb(0x9c, 0xdc, 0xfe));
                            let ty = format!("{}{}", c.data_type, if c.nullable { "" } else { " · not null" });
                            ui.painter().text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, ty, FontId::proportional(11.5), vs::LINE_NO);
                        }
                    }
                    None => {
                        ui.horizontal(|ui| {
                            ui.add_space(44.0);
                            ui.label(RichText::new("memuat kolom…").size(11.5).color(vs::LINE_NO));
                        });
                    }
                }
            }
        }
    });
    if let Some(t) = toggle {
        let key = t.key();
        if !d.expanded.remove(&key) {
            d.expanded.insert(key.clone());
            if !d.columns.contains_key(&key) {
                d.load_columns(ctx, t);
            }
        }
    }
    if let Some(t) = open {
        d.sql = format!("SELECT * FROM {} LIMIT 100;", t.qualified(d.cfg.kind));
        let sql = d.sql.clone();
        d.run(ctx, sql);
    }
}

fn results(ui: &mut Ui, d: &DbState) {
    Frame::new().fill(vs::TAB_BAR_BG).inner_margin(Margin::symmetric(12, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("HASIL").size(11.0).color(vs::SIDEBAR_TITLE));
            if let Some(r) = &d.result {
                let msg = if r.columns.is_empty() {
                    format!("{} baris terpengaruh · {} ms", r.affected.unwrap_or(0), r.elapsed_ms)
                } else {
                    format!(
                        "{} baris{} · {} ms",
                        r.rows.len(),
                        if r.truncated { format!(" (dibatasi {})", crate::db::MAX_ROWS) } else { String::new() },
                        r.elapsed_ms
                    )
                };
                ui.label(RichText::new(msg).size(12.0).color(vs::TEXT));
            }
        });
    });
    if let Some(e) = &d.error {
        Frame::new().fill(Color32::from_rgb(0x3a, 0x16, 0x1e)).inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(format!("⚠ {e}")).color(Color32::from_rgb(0xff, 0x9a, 0x9a)));
        });
        return;
    }
    let Some(r) = &d.result else {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("Tulis SQL di atas lalu tekan ▶ Jalankan (Ctrl+Enter).").color(vs::LINE_NO)));
        return;
    };
    if r.columns.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(RichText::new(format!("✔ Perintah selesai. {} baris terpengaruh.", r.affected.unwrap_or(0))).color(theme::OK)));
        return;
    }
    let row_h = 20.0;
    ScrollArea::horizontal().id_salt("result_h").auto_shrink([false, false]).show(ui, |ui| {
        let mut table = TableBuilder::new(ui)
            .id_salt("result_grid")
            .striped(true)
            .resizable(true)
            .auto_shrink([false, false])
            .cell_layout(egui::Layout::left_to_right(Align::Center))
            .column(Column::exact(44.0));
        for _ in &r.columns {
            table = table.column(Column::initial(140.0).at_least(40.0).clip(true));
        }
        table
            .header(24.0, |mut h| {
                h.col(|ui| {
                    ui.label(RichText::new("#").size(12.0).color(vs::LINE_NO));
                });
                for c in &r.columns {
                    h.col(|ui| {
                        ui.label(RichText::new(c).strong().size(12.5).color(vs::TEXT_BRIGHT));
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, r.rows.len(), |mut row| {
                    let i = row.index();
                    let data = &r.rows[i];
                    row.col(|ui| {
                        ui.label(RichText::new((i + 1).to_string()).size(11.5).color(vs::LINE_NO));
                    });
                    for v in data {
                        row.col(|ui| match v {
                            Some(s) => {
                                let one_line: String = s.chars().take(300).collect::<String>().replace('\n', "⏎");
                                ui.add(egui::Label::new(RichText::new(one_line).monospace().size(12.5).color(vs::TEXT)).truncate())
                                    .on_hover_text(s.chars().take(2000).collect::<String>());
                            }
                            None => {
                                ui.label(RichText::new("NULL").italics().size(12.0).color(vs::LINE_NO));
                            }
                        });
                    }
                });
            });
    });
}
