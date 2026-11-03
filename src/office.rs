//! The office: every project is an employee at a desk. Idle employees doze in
//! their chair; employees with a task type away at their monitor. The windows
//! show the real sky for the current time in Indonesia, and the storage
//! cabinets against the wall are database connections.

use egui::{
    Align2, CentralPanel, Color32, FontId, Frame, Galley, Key, Margin, Painter, Panel, Pos2, Rect, RichText, Sense,
    Stroke, TextEdit, Ui, pos2, vec2,
};
use std::f32::consts::PI;
use std::sync::Arc;

use crate::app::{Action, Project};
use crate::clock::{self, LocalTime};
use crate::config::Settings;
use crate::db::{DbState, Status};
use crate::icons::{self, Icon, IconButton, Kind};
use crate::{chat, project, sprite, theme};

#[derive(Default)]
pub struct OfficeUi {
    pub prompt: String,
    pub target: usize,
}

/// Paints in "sprite pixel" units relative to an origin.
#[derive(Clone, Copy)]
struct Px<'a> {
    p: &'a Painter,
    o: Pos2,
    u: f32,
}

impl Px<'_> {
    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        let l = (self.o.x + x * self.u).round();
        let t = (self.o.y + y * self.u).round();
        let r = (self.o.x + (x + w) * self.u).round();
        let b = (self.o.y + (y + h) * self.u).round();
        Rect::from_min_max(pos2(l, t), pos2(r.max(l + 1.0), b.max(t + 1.0)))
    }

    fn fill(&self, x: f32, y: f32, w: f32, h: f32, c: Color32) {
        self.p.rect_filled(self.rect(x, y, w, h), 0.0, c);
    }

    /// Filled box with a 1-unit-ish dark outline.
    fn boxed(&self, x: f32, y: f32, w: f32, h: f32, c: Color32, outline: Color32) {
        let t = (0.35f32).max(1.0 / self.u);
        self.fill(x, y, w, h, outline);
        self.fill(x + t, y + t, w - 2.0 * t, h - 2.0 * t, c);
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

fn hash(a: u32, b: u32) -> u32 {
    let mut h = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h
}

// Station footprint in sprite units, relative to the character grid origin.
const ST_RIGHT: f32 = 32.0;
const ST_TOP: f32 = -7.0;
const ST_FLOOR: f32 = 17.0;
const TAG_H: f32 = 40.0;

/// Left edge of the desk; seniors get a second monitor and a wider desk.
fn desk_left(level: u32) -> f32 {
    if level >= 7 { -13.0 } else { -6.0 }
}

/// Left edge of the whole station (desk plus the floor plant for seniors).
fn st_left(level: u32) -> f32 {
    if level >= 7 { -15.5 } else { -7.0 }
}

fn desk_center(level: u32) -> f32 {
    (desk_left(level) + ST_RIGHT) / 2.0
}

#[derive(Clone, Copy, PartialEq)]
enum Mood {
    Working,
    Sleeping,
    Awake,
}

pub fn show(
    ui: &mut Ui,
    projects: &mut [Project],
    dbs: &mut [DbState],
    w: &mut OfficeUi,
    settings: &mut Settings,
    now: f64,
    actions: &mut Vec<Action>,
) {
    if !projects.is_empty() {
        Panel::bottom("office_prompt")
            .frame(Frame::new().fill(theme::vs::SIDEBAR_BG).inner_margin(Margin::symmetric(12, 8)))
            .show(ui, |ui| prompt_bar(ui, projects, w, settings, actions));
    }

    CentralPanel::no_frame().show(ui, |ui| {
        let rect = ui.available_rect_before_wrap();
        let (_bg, painter) = ui.allocate_painter(rect.size(), Sense::hover());
        let painter = painter.with_clip_rect(rect);
        let lt = clock::now(settings.tz_offset, settings.time_preview);
        let h = lt.hours;
        let dl = clock::daylight(h);

        let wall_h = (rect.height() * 0.30).clamp(150.0, 260.0);
        let wall = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + wall_h));
        let floor = Rect::from_min_max(pos2(rect.left(), wall.bottom()), rect.max);

        draw_wall(&painter, wall);
        let windows = window_rects(wall);
        for (i, win) in windows.iter().enumerate() {
            draw_window(&painter, *win, i, &windows, h, dl, now);
        }
        draw_clock(&painter, pos2(wall.center().x, wall.top() + wall_h * 0.36), (wall_h * 0.17).clamp(24.0, 40.0), &lt, settings.tz_offset);
        draw_sign(&painter, wall, h);
        draw_floor(&painter, floor);
        let lamps = lamp_positions(wall, &windows);
        for x in &lamps {
            draw_lamp(&painter, pos2(*x, rect.top()), dl);
        }
        draw_plant(&painter, pos2(floor.left() + 34.0, floor.top() + 18.0), 3.2);
        draw_plant(&painter, pos2(floor.right() - 34.0, floor.top() + 18.0), 3.2);

        // Cabinets (database connections) along the wall, right side.
        let cab_u = 4.4;
        let cab_w = 12.0 * cab_u;
        let gap = 64.0;
        let n_cab = dbs.len() + 1;
        let mut cab_x = floor.right() - 80.0 - n_cab as f32 * (cab_w + gap);
        let cab_base = floor.top() + 14.0;
        let mut cabinet_rects = Vec::new();
        for _ in 0..n_cab {
            let r = Rect::from_min_size(pos2(cab_x, cab_base - 24.0 * cab_u), vec2(cab_w, 24.0 * cab_u));
            cabinet_rects.push(r);
            cab_x += cab_w + gap;
        }

        // Desks.
        let area = Rect::from_min_max(
            pos2(floor.left() + 70.0, floor.top() + 30.0),
            pos2(floor.right() - 70.0, rect.bottom() - 8.0),
        );
        let places = layout(projects, area);

        // Interaction first so hover state can drive drawing.
        let mut cab_hover = vec![false; n_cab];
        for (i, r) in cabinet_rects.iter().enumerate() {
            let resp = ui.interact(*r, egui::Id::unique(("cabinet", i)), Sense::click());
            cab_hover[i] = resp.hovered();
            let ghost = i == dbs.len();
            if resp.clicked() {
                actions.push(if ghost { Action::NewDb } else { Action::OpenDb(i) });
            }
            if !ghost {
                let resp = resp.on_hover_text(format!(
                    "{} — {}\n{}\nKlik untuk membuka database",
                    dbs[i].cfg.name,
                    dbs[i].cfg.kind.label(),
                    dbs[i].cfg.target()
                ));
                resp.context_menu(|ui| {
                    if icons::menu_item(ui, Icon::Database, "Buka database").clicked() {
                        actions.push(Action::OpenDb(i));
                        ui.close();
                    }
                    if icons::menu_item(ui, Icon::Edit, "Edit koneksi").clicked() {
                        actions.push(Action::EditDb(i));
                        ui.close();
                    }
                    ui.separator();
                    if icons::menu_item(ui, Icon::Close, "Singkirkan lemari").clicked() {
                        actions.push(Action::RemoveDb(i));
                        ui.close();
                    }
                });
            } else {
                resp.on_hover_text("Tambah lemari baru = koneksi database baru");
            }
        }

        let mut hovered = vec![false; projects.len()];
        for (i, pl) in places.iter().enumerate() {
            let hit = Rect::from_min_max(
                pos2(pl.o.x + desk_left(projects[i].level()) * pl.u, pl.o.y - 6.0 * pl.u),
                pos2(pl.o.x + ST_RIGHT * pl.u, pl.o.y + ST_FLOOR * pl.u + TAG_H),
            );
            let resp = ui.interact(hit, egui::Id::unique(("desk", i, &projects[i].entry.path)), Sense::click());
            hovered[i] = resp.hovered();
            let p = &projects[i];
            let level = p.level();
            let resp = resp.on_hover_ui(|ui| {
                ui.label(RichText::new(&p.entry.name).strong());
                ui.label(RichText::new(p.entry.path.display().to_string()).small().weak());
                if let Some(s) = &p.scan {
                    ui.label(format!(
                        "Level {level} · {} file · {} baris",
                        s.stats.files, s.stats.lines
                    ));
                    let langs: Vec<String> = s.stats.langs.iter().take(4).map(|(l, n)| format!("{l} {n}")).collect();
                    if !langs.is_empty() {
                        ui.label(RichText::new(langs.join(" · ")).small());
                    }
                }
            });
            if resp.clicked() {
                actions.push(Action::Open(i));
            }
            resp.context_menu(|ui| {
                if icons::menu_item(ui, Icon::Folder, "Buka project").clicked() {
                    actions.push(Action::Open(i));
                    ui.close();
                }
                if icons::menu_item(ui, Icon::Target, "Jadikan target prompt").clicked() {
                    w.target = i;
                    ui.close();
                }
                if icons::menu_item(ui, Icon::Refresh, "Pindai ulang").clicked() {
                    actions.push(Action::Rescan(i));
                    ui.close();
                }
                let label = if p.has_skill { "Perbarui skill (.ckdtr)" } else { "Buat skill (.ckdtr)" };
                if icons::menu_item(ui, Icon::Sparkle, label).clicked() {
                    actions.push(Action::MakeSkill(i));
                    ui.close();
                }
                if p.has_skill && icons::menu_item(ui, Icon::File, "Lihat skill").clicked() {
                    actions.push(Action::OpenSkill(i));
                    ui.close();
                }
                if icons::menu_item(ui, Icon::Files, "Buka di Explorer").clicked() {
                    actions.push(Action::Reveal(i));
                    ui.close();
                }
                ui.separator();
                if icons::menu_item(ui, Icon::Close, "Keluarkan dari kantor").clicked() {
                    actions.push(Action::Remove(i));
                    ui.close();
                }
            });
        }

        // Paint: cabinets, then desks back to front.
        for (i, r) in cabinet_rects.iter().enumerate() {
            let px = Px { p: &painter, o: r.min, u: cab_u };
            if i < dbs.len() {
                draw_cabinet(px, &dbs[i], cab_hover[i], now);
            } else {
                draw_ghost_cabinet(px, cab_hover[i]);
            }
        }
        let moods: Vec<Mood> = projects
            .iter()
            .zip(&hovered)
            .map(|(p, &hov)| {
                if p.agent.is_some() {
                    Mood::Working
                } else if hov
                    || p.done_at.is_some_and(|t| now - t < 6.0)
                    || p.level_up_at.is_some_and(|t| now - t < 4.0)
                {
                    Mood::Awake
                } else {
                    Mood::Sleeping
                }
            })
            .collect();
        let mut order: Vec<usize> = (0..projects.len()).collect();
        order.sort_by(|&a, &b| places[a].o.y.total_cmp(&places[b].o.y));
        for &i in &order {
            let px = Px { p: &painter, o: places[i].o, u: places[i].u };
            draw_station(px, &projects[i], moods[i], hovered[i], now, i);
        }

        // Night: dim the room, then add the light sources back on top.
        let dark = 1.0 - dl;
        if dark > 0.01 {
            painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(6, 10, 32, (dark * 150.0) as u8));
            for x in &lamps {
                lamp_glow(&painter, pos2(*x, rect.top()), rect.bottom(), dark);
            }
            for &i in &order {
                let px = Px { p: &painter, o: places[i].o, u: places[i].u };
                screens(px, &projects[i], moods[i], now, i, dark);
            }
            for (i, r) in cabinet_rects.iter().enumerate().take(dbs.len()) {
                cabinet_led(Px { p: &painter, o: r.min, u: cab_u }, &dbs[i], now, true);
            }
        }

        // Labels and bubbles stay readable regardless of lighting.
        for (i, r) in cabinet_rects.iter().enumerate() {
            let (title, sub) = if i < dbs.len() {
                let d = &dbs[i];
                let s = match d.status {
                    Status::Idle => "belum terhubung",
                    Status::Busy => "memproses…",
                    Status::Connected => "terhubung",
                    Status::Error => "error",
                };
                (truncate(&d.cfg.name, 16), format!("{} · {s}", d.cfg.kind.code()))
            } else {
                ("Lemari baru".to_string(), "+ koneksi DB".to_string())
            };
            name_tag(&painter, pos2(r.center().x, r.bottom() + 4.0), &title, &sub, cab_hover[i], false, 11.0);
        }
        for &i in &order {
            let pl = places[i];
            let p = &projects[i];
            let level = p.level();
            let title = format!("{}  Lv{}", p.entry.name, level);
            let sub = match &p.scan {
                Some(_) => format!("{} baris", project::fmt_count(p.lines())),
                None => "memindai…".to_string(),
            };
            name_tag(
                &painter,
                pos2(pl.o.x + desk_center(level) * pl.u, pl.o.y + ST_FLOOR * pl.u + 4.0),
                &title,
                &sub,
                hovered[i],
                moods[i] == Mood::Working,
                13.0,
            );
            let head = pos2(pl.o.x + 8.0 * pl.u, pl.o.y - 3.0 * pl.u);
            match moods[i] {
                Mood::Working => {
                    let dots = ".".repeat(((now * 3.0) as usize % 3) + 1);
                    speech_bubble(&painter, head, &format!("{}{dots}", truncate(&p.activity, 30)), theme::LIVE);
                }
                Mood::Sleeping => zzz(&painter, pos2(pl.o.x + 13.0 * pl.u, pl.o.y + 2.0 * pl.u), pl.u, now, i),
                Mood::Awake => {
                    let text = if p.level_up_at.is_some_and(|t| now - t < 4.0) {
                        Some((format!("LEVEL UP! Lv{level}"), theme::WARM))
                    } else if p.done_at.is_some_and(|t| now - t < 6.0) {
                        Some(("Tugas selesai! ✔".to_string(), theme::OK))
                    } else if hovered[i] {
                        Some(("Hah? Klik untuk buka kodeku".to_string(), Color32::WHITE))
                    } else {
                        None
                    };
                    if let Some((t, c)) = text {
                        speech_bubble(&painter, head, &t, c);
                    }
                }
            }
        }

        if projects.is_empty() {
            empty_state(ui, &painter, floor, actions);
        } else {
            let hint = "Klik karyawan untuk membuka project · klik lemari untuk database · klik kanan untuk menu";
            let g = painter.layout_no_wrap(hint.into(), FontId::proportional(12.0), Color32::from_white_alpha(200));
            let r = Rect::from_min_size(pos2(floor.left() + 10.0, rect.bottom() - g.size().y - 12.0), g.size() + vec2(12.0, 6.0));
            painter.rect_filled(r, 3.0, Color32::from_black_alpha(110));
            painter.galley(r.min + vec2(6.0, 3.0), g, Color32::WHITE);
        }
    });
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(40));
}

#[derive(Clone, Copy)]
struct Place {
    o: Pos2,
    u: f32,
}

/// Flow desks into centred rows, shrinking everything until it fits.
fn layout(projects: &[Project], area: Rect) -> Vec<Place> {
    let gap_x = 26.0;
    let gap_y = 18.0;
    // Start big so a small office fills the floor, then shrink until it fits.
    let mut scale = 1.9f32;
    loop {
        let mut rows: Vec<Vec<(usize, f32, f32)>> = vec![vec![]];
        let mut x = 0.0;
        for (i, p) in projects.iter().enumerate() {
            let u = sprite::pixel_size(p.level()) * scale;
            let w = (ST_RIGHT - st_left(p.level())) * u;
            if x > 0.0 && x + w > area.width() {
                rows.push(vec![]);
                x = 0.0;
            }
            rows.last_mut().unwrap().push((i, u, st_left(p.level())));
            x += w + gap_x;
        }
        let heights: Vec<f32> = rows
            .iter()
            .map(|r| r.iter().map(|(_, u, _)| (ST_FLOOR - ST_TOP) * u).fold(0.0, f32::max) + TAG_H)
            .collect();
        let total: f32 = heights.iter().sum::<f32>() + gap_y * (rows.len().saturating_sub(1)) as f32;
        if total <= area.height() || scale < 0.4 {
            let mut out = vec![Place { o: Pos2::ZERO, u: 1.0 }; projects.len()];
            let mut y = area.top() + ((area.height() - total) * 0.4).max(0.0);
            for (row, h) in rows.iter().zip(&heights) {
                let width: f32 = row.iter().map(|(_, u, l)| (ST_RIGHT - l) * u).sum::<f32>()
                    + gap_x * (row.len().saturating_sub(1)) as f32;
                let mut x = area.left() + ((area.width() - width) / 2.0).max(0.0);
                let baseline = y + h - TAG_H;
                for &(i, u, l) in row {
                    out[i] = Place {
                        o: pos2(x - l * u, baseline - ST_FLOOR * u),
                        u,
                    };
                    x += (ST_RIGHT - l) * u + gap_x;
                }
                y += h + gap_y;
            }
            return out;
        }
        scale *= 0.88;
    }
}

// ---------------------------------------------------------------- room

fn draw_wall(p: &Painter, wall: Rect) {
    p.rect_filled(wall, 0.0, Color32::from_rgb(0xd9, 0xcf, 0xbd));
    let mut x = wall.left();
    while x < wall.right() {
        p.rect_filled(
            Rect::from_min_size(pos2(x, wall.top()), vec2(3.0, wall.height())),
            0.0,
            Color32::from_rgb(0xd1, 0xc6, 0xb2),
        );
        x += 28.0;
    }
    let wains = wall.bottom() - wall.height() * 0.26;
    p.rect_filled(Rect::from_min_max(pos2(wall.left(), wains), wall.max), 0.0, Color32::from_rgb(0xb8, 0xa4, 0x86));
    p.rect_filled(
        Rect::from_min_max(pos2(wall.left(), wains - 3.0), pos2(wall.right(), wains + 1.0)),
        0.0,
        Color32::from_rgb(0x8f, 0x79, 0x5a),
    );
    p.rect_filled(
        Rect::from_min_max(pos2(wall.left(), wall.bottom() - 6.0), wall.max),
        0.0,
        Color32::from_rgb(0x5e, 0x48, 0x31),
    );
}

fn window_rects(wall: Rect) -> Vec<Rect> {
    let w = (wall.width() * 0.13).clamp(120.0, 230.0);
    let h = wall.height() * 0.56;
    let top = wall.top() + wall.height() * 0.12;
    let left_zone = (wall.left() + 230.0, wall.center().x - 60.0);
    let right_zone = (wall.center().x + 60.0, wall.right() - 40.0);
    let mut out = Vec::new();
    for (a, b) in [left_zone, right_zone] {
        let n = (((b - a) / (w + 50.0)).floor() as usize).clamp(1, 3);
        let step = (b - a) / n as f32;
        for k in 0..n {
            let cx = a + step * (k as f32 + 0.5);
            out.push(Rect::from_min_size(pos2(cx - w / 2.0, top), vec2(w, h)));
        }
    }
    out
}

fn draw_window(p: &Painter, win: Rect, idx: usize, all: &[Rect], h: f32, dl: f32, now: f64) {
    let frame = Color32::from_rgb(0xee, 0xea, 0xe2);
    let dark = Color32::from_rgb(0x4e, 0x46, 0x3c);
    p.rect_filled(win.expand(5.0), 0.0, dark);
    p.rect_filled(win.expand(3.0), 0.0, frame);
    let glass = win.shrink(3.0);
    let gp = p.with_clip_rect(glass);
    let (top, horizon) = clock::sky(h);
    let bands = 14;
    for b in 0..bands {
        let t = b as f32 / (bands - 1) as f32;
        let y0 = glass.top() + glass.height() * b as f32 / bands as f32;
        let y1 = glass.top() + glass.height() * (b + 1) as f32 / bands as f32 + 1.0;
        gp.rect_filled(
            Rect::from_min_max(pos2(glass.left(), y0), pos2(glass.right(), y1)),
            0.0,
            mix(top, horizon, t),
        );
    }
    let span_l = all.first().map(|r| r.left()).unwrap_or(glass.left());
    let span_r = all.last().map(|r| r.right()).unwrap_or(glass.right());
    let span = span_r - span_l;

    // Stars.
    if dl < 0.7 {
        for k in 0..14u32 {
            let hs = hash(idx as u32 * 97 + k, 7);
            let x = glass.left() + (hs % 1000) as f32 / 1000.0 * glass.width();
            let y = glass.top() + ((hs >> 10) % 1000) as f32 / 1000.0 * glass.height() * 0.6;
            let tw = ((now * 2.0 + k as f64).sin() * 0.5 + 0.5) as f32;
            let a = ((1.0 - dl / 0.7) * (0.4 + 0.6 * tw) * 255.0) as u8;
            gp.rect_filled(Rect::from_min_size(pos2(x, y), vec2(2.0, 2.0)), 0.0, Color32::from_rgba_unmultiplied(255, 255, 230, a));
        }
    }
    // Sun (06–18) and moon (18–06) travel across all windows.
    let arc = |t: f32| {
        let x = span_l + t * span;
        let y = glass.bottom() - (t * PI).sin() * glass.height() * 0.85 + glass.height() * 0.1;
        pos2(x, y)
    };
    if (5.7..18.3).contains(&h) {
        let c = arc(((h - 5.7) / 12.6).clamp(0.0, 1.0));
        let r = glass.width().min(glass.height()) * 0.11;
        let sun = mix(Color32::from_rgb(0xff, 0x9a, 0x3c), Color32::from_rgb(0xff, 0xf1, 0xa8), dl);
        gp.circle_filled(c, r * 1.8, sun.gamma_multiply(0.25));
        gp.rect_filled(Rect::from_center_size(c, vec2(r * 2.0, r * 2.0)), r * 0.35, sun);
    } else {
        let t = ((h - 18.3).rem_euclid(24.0) / 11.4).clamp(0.0, 1.0);
        let c = arc(t);
        let r = glass.width().min(glass.height()) * 0.09;
        gp.circle_filled(c, r * 1.9, Color32::from_rgba_unmultiplied(200, 210, 255, 25));
        gp.circle_filled(c, r, Color32::from_rgb(0xf2, 0xf0, 0xdc));
        gp.circle_filled(c + vec2(r * 0.45, -r * 0.2), r * 0.85, mix(top, horizon, 0.3));
    }
    // Clouds by day.
    if dl > 0.15 {
        for k in 0..4u32 {
            let hs = hash(k, 31);
            let wdt = 40.0 + (hs % 40) as f32;
            let travel = span + 200.0;
            let x = span_l - 100.0 + ((hs % 1000) as f32 / 1000.0 * travel + now as f32 * (4.0 + k as f32)) % travel;
            let y = glass.top() + 8.0 + ((hs >> 8) % 30) as f32;
            let c = Color32::from_rgba_unmultiplied(255, 255, 255, (dl * 210.0) as u8);
            gp.rect_filled(Rect::from_min_size(pos2(x, y + 6.0), vec2(wdt, 8.0)), 0.0, c);
            gp.rect_filled(Rect::from_min_size(pos2(x + wdt * 0.2, y), vec2(wdt * 0.45, 8.0)), 0.0, c);
        }
    }
    // City skyline with lit windows at night.
    let mut x = glass.left() - 4.0;
    let mut k = 0u32;
    while x < glass.right() {
        let hs = hash(idx as u32 * 13 + k, 3);
        let bw = 14.0 + (hs % 20) as f32;
        let bh = glass.height() * (0.18 + (hs >> 6) as f32 % 100.0 / 100.0 * 0.32);
        let b = Rect::from_min_max(pos2(x, glass.bottom() - bh), pos2(x + bw, glass.bottom()));
        gp.rect_filled(b, 0.0, mix(Color32::from_rgb(0x16, 0x1d, 0x33), Color32::from_rgb(0x84, 0x98, 0xb4), dl));
        if dl < 0.6 {
            let mut wy = b.top() + 4.0;
            let mut j = 0u32;
            while wy < b.bottom() - 3.0 {
                let mut wx = b.left() + 3.0;
                while wx < b.right() - 3.0 {
                    j += 1;
                    if hash(hs, j).is_multiple_of(3) {
                        gp.rect_filled(
                            Rect::from_min_size(pos2(wx, wy), vec2(2.0, 2.0)),
                            0.0,
                            Color32::from_rgba_unmultiplied(255, 214, 120, ((0.6 - dl) / 0.6 * 230.0) as u8),
                        );
                    }
                    wx += 5.0;
                }
                wy += 6.0;
            }
        }
        x += bw + 2.0;
        k += 1;
    }
    // Mullions and sill.
    p.rect_filled(Rect::from_center_size(glass.center(), vec2(4.0, glass.height())), 0.0, frame);
    p.rect_filled(Rect::from_center_size(glass.center(), vec2(glass.width(), 4.0)), 0.0, frame);
    p.rect_filled(
        Rect::from_min_max(pos2(win.left() - 10.0, win.bottom() + 3.0), pos2(win.right() + 10.0, win.bottom() + 10.0)),
        0.0,
        Color32::from_rgb(0xf5, 0xf1, 0xea),
    );
    p.rect_filled(
        Rect::from_min_max(pos2(win.left() - 10.0, win.bottom() + 10.0), pos2(win.right() + 10.0, win.bottom() + 12.0)),
        0.0,
        dark,
    );
    // Daylight spilling onto the wall.
    if dl > 0.3 {
        p.rect_filled(win.expand(14.0), 6.0, Color32::from_rgba_unmultiplied(255, 250, 220, ((dl - 0.3) * 30.0) as u8));
    }
}

fn draw_clock(p: &Painter, c: Pos2, r: f32, lt: &LocalTime, tz: i32) {
    p.circle_filled(c, r + 3.0, Color32::from_rgb(0x3a, 0x30, 0x28));
    p.circle_filled(c, r, Color32::from_rgb(0xfa, 0xf7, 0xf0));
    for i in 0..12 {
        let a = i as f32 / 12.0 * 2.0 * PI;
        let d = vec2(a.sin(), -a.cos());
        let len = if i % 3 == 0 { 0.22 } else { 0.12 };
        p.line_segment([c + d * r * (1.0 - len), c + d * r * 0.92], Stroke::new(2.0, Color32::from_rgb(0x3a, 0x30, 0x28)));
    }
    let hm = (lt.hour % 12) as f32 + lt.minute as f32 / 60.0;
    let ha = hm / 12.0 * 2.0 * PI;
    let ma = (lt.minute as f32 + lt.second as f32 / 60.0) / 60.0 * 2.0 * PI;
    let sa = lt.second as f32 / 60.0 * 2.0 * PI;
    let dir = |a: f32| vec2(a.sin(), -a.cos());
    p.line_segment([c, c + dir(ha) * r * 0.5], Stroke::new(3.0, Color32::from_rgb(0x22, 0x1c, 0x18)));
    p.line_segment([c, c + dir(ma) * r * 0.75], Stroke::new(2.0, Color32::from_rgb(0x22, 0x1c, 0x18)));
    p.line_segment([c, c + dir(sa) * r * 0.82], Stroke::new(1.0, Color32::from_rgb(0xd0, 0x30, 0x30)));
    p.circle_filled(c, 2.5, Color32::from_rgb(0x22, 0x1c, 0x18));
    let label = format!("{} {}", lt.hm(), clock::zone_label(tz));
    let g = p.layout_no_wrap(label, FontId::monospace(13.0), Color32::WHITE);
    let r2 = Rect::from_center_size(pos2(c.x, c.y + r + 16.0), g.size() + vec2(12.0, 4.0));
    p.rect_filled(r2, 2.0, Color32::from_rgb(0x2a, 0x22, 0x1c));
    p.galley(r2.min + vec2(6.0, 2.0), g, Color32::WHITE);
    let g2 = p.layout_no_wrap(lt.date_label(), FontId::proportional(11.0), Color32::from_rgb(0x4a, 0x3e, 0x32));
    p.galley(pos2(c.x - g2.size().x / 2.0, r2.bottom() + 2.0), g2, Color32::WHITE);
}

fn draw_sign(p: &Painter, wall: Rect, h: f32) {
    let r = Rect::from_min_size(pos2(wall.left() + 28.0, wall.top() + wall.height() * 0.16), vec2(178.0, 58.0));
    p.rect_filled(r.expand(3.0), 2.0, Color32::from_rgb(0x3a, 0x2a, 0x1c));
    p.rect_filled(r, 2.0, Color32::from_rgb(0x5a, 0x40, 0x2a));
    p.text(r.center_top() + vec2(0.0, 9.0), Align2::CENTER_TOP, "COKANDETOR HQ", FontId::monospace(15.0), theme::WARM);
    p.text(
        r.center_top() + vec2(0.0, 32.0),
        Align2::CENTER_TOP,
        format!("{}!", clock::greeting(h)),
        FontId::proportional(12.0),
        Color32::from_rgb(0xf2, 0xe6, 0xd0),
    );
}

fn draw_floor(p: &Painter, floor: Rect) {
    p.rect_filled(floor, 0.0, Color32::from_rgb(0xa8, 0x7b, 0x52));
    let plank_h = 24.0;
    let seam = Color32::from_rgb(0x8a, 0x62, 0x3f);
    let mut y = floor.top();
    let mut row = 0u32;
    while y < floor.bottom() {
        if row % 2 == 1 {
            p.rect_filled(
                Rect::from_min_max(pos2(floor.left(), y), pos2(floor.right(), (y + plank_h).min(floor.bottom()))),
                0.0,
                Color32::from_rgb(0xa2, 0x76, 0x4e),
            );
        }
        p.rect_filled(Rect::from_min_max(pos2(floor.left(), y), pos2(floor.right(), y + 2.0)), 0.0, seam);
        let off = (hash(row, 5) % 160) as f32;
        let mut x = floor.left() - off;
        while x < floor.right() {
            p.rect_filled(Rect::from_min_size(pos2(x, y), vec2(2.0, plank_h)), 0.0, seam);
            x += 180.0;
        }
        y += plank_h;
        row += 1;
    }
    // Shadow where floor meets wall.
    p.rect_filled(
        Rect::from_min_size(floor.min, vec2(floor.width(), 10.0)),
        0.0,
        Color32::from_black_alpha(40),
    );
}

/// Lamps hang in the gaps between windows (but not in front of the clock).
fn lamp_positions(wall: Rect, windows: &[Rect]) -> Vec<f32> {
    let mut xs: Vec<f32> = windows
        .windows(2)
        .map(|w| (w[0].right() + w[1].left()) / 2.0)
        .filter(|x| (x - wall.center().x).abs() > 80.0)
        .collect();
    if let Some(first) = windows.first() {
        xs.insert(0, (wall.left() + 230.0 + first.left()) / 2.0);
    }
    if let Some(last) = windows.last() {
        xs.push((last.right() + wall.right()) / 2.0);
    }
    xs
}

fn draw_lamp(p: &Painter, top: Pos2, dl: f32) {
    p.line_segment([top, top + vec2(0.0, 26.0)], Stroke::new(2.0, Color32::from_rgb(0x30, 0x30, 0x34)));
    let shade = Rect::from_min_max(pos2(top.x - 22.0, top.y + 26.0), pos2(top.x + 22.0, top.y + 38.0));
    p.rect_filled(shade.shrink2(vec2(8.0, 0.0)).translate(vec2(0.0, -4.0)), 0.0, Color32::from_rgb(0x2e, 0x5d, 0x55));
    p.rect_filled(shade, 0.0, Color32::from_rgb(0x36, 0x6e, 0x64));
    let bulb = if dl < 0.5 { Color32::from_rgb(0xff, 0xe9, 0xa8) } else { Color32::from_rgb(0xd8, 0xd0, 0xb8) };
    p.rect_filled(Rect::from_center_size(pos2(top.x, shade.bottom() + 3.0), vec2(12.0, 6.0)), 2.0, bulb);
}

fn lamp_glow(p: &Painter, top: Pos2, bottom: f32, dark: f32) {
    let y0 = top.y + 41.0;
    let steps = 28;
    for i in 0..steps {
        let t0 = i as f32 / steps as f32;
        let t1 = (i + 1) as f32 / steps as f32;
        let ya = y0 + (bottom - y0) * t0;
        let yb = y0 + (bottom - y0) * t1;
        let w = 14.0 + 260.0 * t1;
        let a = (dark * 22.0 * (1.0 - t0 * 0.7)) as u8;
        p.rect_filled(
            Rect::from_min_max(pos2(top.x - w / 2.0, ya), pos2(top.x + w / 2.0, yb)),
            0.0,
            Color32::from_rgba_unmultiplied(255, 220, 140, a),
        );
    }
    p.circle_filled(pos2(top.x, y0), 14.0, Color32::from_rgba_unmultiplied(255, 236, 170, (dark * 120.0) as u8));
    p.rect_filled(
        Rect::from_center_size(pos2(top.x, y0 - 2.0), vec2(12.0, 6.0)),
        2.0,
        Color32::from_rgb(0xff, 0xf0, 0xb8),
    );
}

fn draw_plant(p: &Painter, base: Pos2, u: f32) {
    let px = Px { p, o: base - vec2(4.0 * u, 16.0 * u), u };
    let leaf = Color32::from_rgb(0x3f, 0x8f, 0x4a);
    let leaf2 = Color32::from_rgb(0x2e, 0x6e, 0x38);
    px.fill(3.0, 0.0, 2.0, 4.0, leaf);
    px.fill(0.0, 2.0, 3.0, 2.0, leaf2);
    px.fill(5.0, 1.0, 3.0, 2.0, leaf2);
    px.fill(1.0, 4.0, 6.0, 4.0, leaf);
    px.fill(2.0, 5.0, 1.0, 1.0, leaf2);
    px.fill(5.0, 6.0, 1.0, 1.0, leaf2);
    px.boxed(1.0, 8.0, 6.0, 8.0, Color32::from_rgb(0xc0, 0x64, 0x3a), Color32::from_rgb(0x6a, 0x34, 0x1e));
    px.fill(1.0, 8.0, 6.0, 1.0, Color32::from_rgb(0x8f, 0x48, 0x28));
}

// ---------------------------------------------------------------- desks

/// The employee at their desk, for the editor's welcome page.
pub fn desk_preview(painter: &Painter, o: Pos2, u: f32, p: &Project, now: f64) {
    let mood = if p.agent.is_some() { Mood::Working } else { Mood::Awake };
    draw_station(Px { p: painter, o, u }, p, mood, false, now, 0);
}

fn draw_station(px: Px, p: &Project, mood: Mood, hovered: bool, now: f64, i: usize) {
    let level = p.level();
    let alpha = if p.scan.is_none() { 0.55 } else { 1.0 };
    let outline = Color32::from_rgb(0x2a, 0x1e, 0x14);

    // Floor shadow and hover marker.
    let dl = desk_left(level);
    let dw = ST_RIGHT - dl;
    px.fill(dl - 1.0, ST_FLOOR - 0.4, dw + 2.0, 1.0, Color32::from_black_alpha(60));
    if hovered {
        px.fill(dl - 1.0, ST_FLOOR + 0.6, dw + 2.0, 0.6, theme::ACCENT.gamma_multiply(0.8));
    }
    if level >= 7 {
        // Floor plant beside the desk.
        let pp = Px { o: px.o + vec2(-15.0 * px.u, 7.0 * px.u), ..px };
        draw_plant_units(pp);
    }

    // Chair back.
    let chair = Color32::from_rgb(0x3d, 0x42, 0x50);
    px.boxed(1.0, 3.0, 14.0, 9.0, chair, Color32::from_rgb(0x1f, 0x22, 0x2a));
    px.fill(2.0, 3.6, 12.0, 0.8, Color32::from_rgb(0x52, 0x58, 0x6b));

    // The employee. Sleeping slumps a little and nods.
    let slump = match mood {
        Mood::Sleeping => 0.9 + 0.25 * ((now * 1.3 + i as f64).sin() as f32),
        Mood::Working => -0.15 * ((now * 9.0).sin().abs() as f32),
        Mood::Awake => 0.0,
    };
    let blink = mood == Mood::Sleeping || ((now + i as f64 * 1.7) % 4.0) < 0.12;
    sprite::draw(
        px.p,
        &p.look,
        &sprite::DrawOpts {
            bottom_center: px.o + vec2(8.0 * px.u, (16.0 + slump) * px.u),
            px: px.u,
            flip: false,
            squash: 1.0,
            blink,
            walk_frame: false,
            working: false,
            level,
            time: now,
            alpha,
        },
    );

    // Monitors.
    if level >= 7 {
        monitor_frame(px, -12.0, 1.0, 10.5, 8.0);
    }
    monitor_frame(px, 17.0, 0.0, 12.0, 9.0);
    screens(px, p, mood, now, i, 0.0);

    // Desk.
    let top = Color32::from_rgb(0xc8, 0x9e, 0x6e);
    let front = Color32::from_rgb(0x9c, 0x72, 0x48);
    px.fill(dl, 11.0, dw, 1.4, outline);
    px.fill(dl + 0.3, 11.2, dw - 0.6, 1.0, top);
    px.fill(dl + 0.3, 11.2, dw - 0.6, 0.3, Color32::from_rgb(0xdc, 0xb6, 0x86));
    px.boxed(dl + 1.0, 12.2, dw - 2.0, ST_FLOOR - 12.2, front, outline);
    px.fill(-1.0, 12.8, 18.0, 3.8, Color32::from_rgb(0x8a, 0x63, 0x3e));
    px.boxed(22.0, 13.0, 8.4, 3.6, Color32::from_rgb(0xa8, 0x7d, 0x52), outline);
    px.fill(25.0, 14.4, 2.4, 0.6, Color32::from_rgb(0xe8, 0xd4, 0xa8));

    // Keyboard, mouse and hands.
    px.boxed(2.0, 10.3, 12.0, 1.0, Color32::from_rgb(0xdc, 0xdf, 0xe6), Color32::from_rgb(0x55, 0x58, 0x60));
    for k in 0..5 {
        px.fill(3.0 + k as f32 * 2.2, 10.65, 1.2, 0.3, Color32::from_rgb(0xa6, 0xab, 0xb6));
    }
    px.boxed(15.0, 10.4, 1.4, 0.9, Color32::from_rgb(0xdc, 0xdf, 0xe6), Color32::from_rgb(0x55, 0x58, 0x60));
    if mood == Mood::Working {
        let tap = (now * 9.0) as i64 % 2 == 0;
        let skin = p.look.skin;
        px.fill(4.0, if tap { 9.5 } else { 9.9 }, 1.6, 0.9, skin);
        px.fill(10.4, if tap { 9.9 } else { 9.5 }, 1.6, 0.9, skin);
    }

    // Desk items that come with seniority.
    if level >= 2 {
        px.boxed(29.6, 8.8, 1.9, 2.4, Color32::from_rgb(0xf2, 0xf2, 0xf2), outline);
        px.fill(29.9, 9.1, 1.3, 0.4, Color32::from_rgb(0x5a, 0x34, 0x1e));
        px.fill(31.5, 9.4, 0.5, 1.0, outline);
        if mood == Mood::Working {
            for k in 0..3 {
                let t = ((now * 1.5 + k as f64 * 0.33) % 1.0) as f32;
                let x = 30.2 + ((now * 3.0 + k as f64).sin() as f32) * 0.4;
                px.fill(x, 8.4 - t * 3.0, 0.4, 0.4, Color32::from_rgba_unmultiplied(255, 255, 255, ((1.0 - t) * 160.0) as u8));
            }
        }
    }
    if (4..7).contains(&level) {
        let pot = Px { o: px.o + vec2(-5.6 * px.u, 5.5 * px.u), u: px.u * 0.7, ..px };
        draw_plant_units(pot);
    }
    if (5..7).contains(&level) {
        let books = [
            Color32::from_rgb(0xc0, 0x39, 0x2b),
            Color32::from_rgb(0x29, 0x80, 0xb9),
            Color32::from_rgb(0xf3, 0x9c, 0x12),
        ];
        for (k, c) in books.iter().enumerate() {
            px.boxed(-2.6 + k as f32 * 0.2, 10.2 - k as f32 * 0.8, 2.4, 0.8, *c, outline);
        }
    }
}

fn draw_plant_units(px: Px) {
    let leaf = Color32::from_rgb(0x3f, 0x8f, 0x4a);
    let leaf2 = Color32::from_rgb(0x2e, 0x6e, 0x38);
    px.fill(1.5, 0.0, 1.0, 2.5, leaf);
    px.fill(0.0, 1.5, 1.5, 1.0, leaf2);
    px.fill(2.5, 1.0, 1.5, 1.0, leaf2);
    px.fill(0.5, 2.5, 3.0, 2.0, leaf);
    px.boxed(0.5, 4.5, 3.0, 3.0, Color32::from_rgb(0xc0, 0x64, 0x3a), Color32::from_rgb(0x6a, 0x34, 0x1e));
}

fn monitor_frame(px: Px, x: f32, y: f32, w: f32, h: f32) {
    let bezel = Color32::from_rgb(0x22, 0x25, 0x2c);
    px.boxed(x, y, w, h, bezel, Color32::from_rgb(0x0c, 0x0d, 0x10));
    let cx = x + w / 2.0;
    px.fill(cx - 1.0, y + h, 2.0, 11.2 - (y + h), Color32::from_rgb(0x30, 0x33, 0x3b));
    px.boxed(cx - 3.0, 10.6, 6.0, 0.7, Color32::from_rgb(0x3a, 0x3e, 0x48), Color32::from_rgb(0x15, 0x16, 0x1a));
}

/// Screen contents; drawn once normally and again on top of the night overlay
/// (with `glow` > 0) so monitors light up the dark office.
fn screens(px: Px, p: &Project, mood: Mood, now: f64, i: usize, glow: f32) {
    let level = p.level();
    let mut list = vec![(17.0f32, 0.0f32, 12.0f32, 9.0f32, false)];
    if level >= 7 {
        list.push((-12.0, 1.0, 10.5, 8.0, true));
    }
    for (x, y, w, h, terminal) in list {
        let sx = x + 0.7;
        let sy = y + 0.7;
        let sw = w - 1.4;
        let sh = h - 1.6;
        let on = mood != Mood::Sleeping;
        if glow > 0.0 && on {
            let halo = Px { ..px };
            halo.fill(sx - 2.0, sy - 2.0, sw + 4.0, sh + 4.0, Color32::from_rgba_unmultiplied(110, 190, 255, (glow * 30.0) as u8));
        }
        match mood {
            Mood::Working => {
                px.fill(sx, sy, sw, sh, Color32::from_rgb(0x0f, 0x17, 0x26));
                let palette = if terminal {
                    [theme::OK, theme::OK, Color32::from_rgb(0xa0, 0xa0, 0xa0), theme::OK]
                } else {
                    [
                        Color32::from_rgb(0x56, 0x9c, 0xd6),
                        Color32::from_rgb(0xce, 0x91, 0x78),
                        Color32::from_rgb(0x9c, 0xdc, 0xfe),
                        Color32::from_rgb(0xdc, 0xdc, 0xaa),
                    ]
                };
                let scroll = (now * 2.5) as u32;
                let lines = (sh / 0.9) as u32;
                for l in 0..lines {
                    let hs = hash(scroll + l + i as u32 * 31, if terminal { 9 } else { 4 });
                    let indent = (hs % 3) as f32 * 0.8;
                    let mut cx = sx + 0.4 + indent;
                    let segs = 1 + (hs >> 3) % 3;
                    for s in 0..segs {
                        let len = 0.8 + ((hs >> (5 + s * 3)) % 4) as f32 * 0.7;
                        if cx + len > sx + sw - 0.3 {
                            break;
                        }
                        let c = palette[((hs >> (11 + s * 2)) % 4) as usize];
                        px.fill(cx, sy + 0.35 + l as f32 * 0.9, len, 0.45, c);
                        cx += len + 0.4;
                    }
                    if l + 1 == lines && (now * 3.0) as i64 % 2 == 0 {
                        px.fill(cx, sy + 0.25 + l as f32 * 0.9, 0.35, 0.65, Color32::WHITE);
                    }
                }
            }
            Mood::Awake => {
                px.fill(sx, sy, sw, sh, Color32::from_rgb(0x1e, 0x1e, 0x1e));
                px.fill(sx, sy, sw, 0.6, Color32::from_rgb(0x33, 0x33, 0x33));
                px.fill(sx, sy + sh - 0.5, sw, 0.5, theme::vs::STATUS_BG);
                for l in 0..((sh - 1.4) / 0.9) as u32 {
                    let hs = hash(l + i as u32 * 31, 4);
                    let len = 1.5 + (hs % 6) as f32 * 0.8;
                    px.fill(sx + 0.5 + (hs % 2) as f32 * 0.8, sy + 1.0 + l as f32 * 0.9, len.min(sw - 1.5), 0.4, Color32::from_rgb(0x8c, 0x9c, 0xb0));
                }
            }
            Mood::Sleeping => {
                px.fill(sx, sy, sw, sh, Color32::from_rgb(0x0a, 0x0c, 0x12));
                // Bouncing screensaver logo.
                let t = now * 0.6 + i as f64;
                let bx = ((t * 0.9).sin() * 0.5 + 0.5) as f32 * (sw - 1.6);
                let by = ((t * 1.3).cos() * 0.5 + 0.5) as f32 * (sh - 1.2);
                px.fill(sx + bx, sy + by, 1.6, 1.2, p.look.shirt.gamma_multiply(0.7));
            }
        }
        px.fill(x + w / 2.0 - 0.2, y + h - 0.6, 0.4, 0.3, if on { theme::OK } else { theme::WARM });
    }
}

// ---------------------------------------------------------------- cabinets

fn draw_cabinet(px: Px, d: &DbState, hovered: bool, now: f64) {
    let outline = Color32::from_rgb(0x1c, 0x22, 0x2c);
    let body = Color32::from_rgb(0x6f, 0x7d, 0x92);
    px.fill(-0.5, 23.6, 13.0, 0.8, Color32::from_black_alpha(70));
    px.boxed(0.0, 0.0, 12.0, 24.0, body, outline);
    px.fill(10.6, 0.4, 1.0, 23.2, Color32::from_rgb(0x5b, 0x67, 0x7a));
    px.fill(0.4, 0.4, 11.2, 0.8, Color32::from_rgb(0x8a, 0x98, 0xad));
    for k in 0..4 {
        let y = 1.6 + k as f32 * 5.5;
        let open = hovered && k == 0;
        let (dx, dw) = if open { (-0.5, 11.0) } else { (0.6, 9.8) };
        if open {
            px.fill(0.4, y + 4.6, 11.2, 0.6, Color32::from_black_alpha(90));
        }
        px.boxed(dx, y, dw, 4.8, if open { Color32::from_rgb(0x95, 0xa3, 0xb8) } else { Color32::from_rgb(0x7e, 0x8c, 0xa2) }, outline);
        px.fill(dx + dw / 2.0 - 2.0, y + 2.9, 4.0, 0.8, Color32::from_rgb(0xd6, 0xdb, 0xe4));
        px.fill(dx + dw / 2.0 - 1.8, y + 0.8, 3.6, 1.6, Color32::from_rgb(0xf4, 0xf1, 0xe6));
    }
    // Kind code on the top drawer label.
    let lab = px.rect(4.2, 2.4, 3.6, 1.6);
    px.p.text(lab.center(), Align2::CENTER_CENTER, d.cfg.kind.code(), FontId::monospace((px.u * 1.5).max(8.0)), Color32::from_rgb(0x2a, 0x2a, 0x40));
    cabinet_led(px, d, now, false);
}

fn cabinet_led(px: Px, d: &DbState, now: f64, glow: bool) {
    let c = match d.status {
        Status::Idle => Color32::from_rgb(0x70, 0x76, 0x80),
        Status::Busy => {
            if (now * 6.0) as i64 % 2 == 0 {
                theme::WARM
            } else {
                Color32::from_rgb(0x70, 0x5a, 0x20)
            }
        }
        Status::Connected => theme::OK,
        Status::Error => theme::DANGER,
    };
    if glow && d.status != Status::Idle {
        px.fill(8.6, -0.8, 3.0, 2.4, c.gamma_multiply(0.3));
    }
    px.fill(9.4, -0.1, 1.4, 0.8, c);
}

fn draw_ghost_cabinet(px: Px, hovered: bool) {
    let c = if hovered { Color32::from_white_alpha(200) } else { Color32::from_white_alpha(110) };
    px.fill(0.0, 0.0, 12.0, 24.0, Color32::from_white_alpha(if hovered { 40 } else { 18 }));
    let mut y = 0.0;
    while y < 24.0 {
        px.fill(0.0, y, 0.4, 1.0, c);
        px.fill(11.6, y, 0.4, 1.0, c);
        y += 2.0;
    }
    let mut x = 0.0;
    while x < 12.0 {
        px.fill(x, 0.0, 1.0, 0.4, c);
        px.fill(x, 23.6, 1.0, 0.4, c);
        x += 2.0;
    }
    px.fill(5.4, 9.0, 1.2, 6.0, c);
    px.fill(3.0, 11.4, 6.0, 1.2, c);
}

// ---------------------------------------------------------------- labels

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n - 1).collect();
        format!("{t}…")
    }
}

fn layout_text(painter: &Painter, text: &str, font: FontId, color: Color32) -> Arc<Galley> {
    painter.layout_no_wrap(text.to_string(), font, color)
}

fn name_tag(painter: &Painter, top_center: Pos2, title: &str, sub: &str, hovered: bool, working: bool, size: f32) {
    let g1 = layout_text(painter, title, FontId::proportional(size), Color32::WHITE);
    let g2 = layout_text(painter, sub, FontId::proportional(size - 2.0), Color32::from_gray(205));
    let w = g1.size().x.max(g2.size().x) + 14.0;
    let h = g1.size().y + g2.size().y + 6.0;
    let r = Rect::from_min_size(pos2(top_center.x - w / 2.0, top_center.y), vec2(w, h));
    let fill = if working {
        Color32::from_rgba_unmultiplied(15, 60, 85, 230)
    } else if hovered {
        Color32::from_rgba_unmultiplied(60, 40, 110, 235)
    } else {
        Color32::from_rgba_unmultiplied(20, 16, 28, 205)
    };
    painter.rect_filled(r, 3.0, fill);
    painter.galley(pos2(r.center().x - g1.size().x / 2.0, r.top() + 3.0), g1, Color32::WHITE);
    painter.galley(pos2(r.center().x - g2.size().x / 2.0, r.bottom() - g2.size().y - 3.0), g2, Color32::WHITE);
}

fn speech_bubble(painter: &Painter, bottom_center: Pos2, text: &str, accent: Color32) {
    let ink = Color32::from_rgb(0x1a, 0x16, 0x26);
    let g = layout_text(painter, text, FontId::monospace(12.0), ink);
    let w = g.size().x + 16.0;
    let h = g.size().y + 10.0;
    let r = Rect::from_min_size(pos2(bottom_center.x - w / 2.0, bottom_center.y - h - 6.0), vec2(w, h));
    painter.rect_filled(r.expand(2.0), 0.0, ink);
    painter.rect_filled(r, 0.0, Color32::from_rgb(0xf6, 0xf3, 0xff));
    painter.rect_filled(Rect::from_min_size(r.left_top(), vec2(4.0, h)), 0.0, accent);
    for k in 0..3 {
        let kw = (3 - k) as f32 * 4.0;
        let tr = Rect::from_min_size(pos2(bottom_center.x - kw / 2.0, r.bottom() + 2.0 + k as f32 * 2.0), vec2(kw, 2.0));
        painter.rect_filled(tr.expand(1.0), 0.0, ink);
    }
    painter.galley(pos2(r.left() + 9.0, r.top() + 5.0), g, Color32::BLACK);
}

fn zzz(painter: &Painter, from: Pos2, u: f32, now: f64, seed: usize) {
    for k in 0..3 {
        let t = ((now * 0.45 + k as f64 / 3.0 + seed as f64 * 0.17) % 1.0) as f32;
        let pos = from + vec2(t * u * 4.0 + (t * 6.0).sin() * u * 0.6, -t * u * 9.0);
        let size = 10.0 + u * 0.9 + t * u * 1.6;
        painter.text(
            pos,
            Align2::CENTER_CENTER,
            if k % 2 == 0 { "Z" } else { "z" },
            FontId::monospace(size),
            Color32::from_rgba_unmultiplied(235, 240, 255, ((1.0 - t) * 230.0) as u8),
        );
    }
}

fn empty_state(ui: &mut Ui, painter: &Painter, floor: Rect, actions: &mut Vec<Action>) {
    let c = floor.center();
    let card = Rect::from_center_size(c, vec2(520.0, 170.0));
    painter.rect_filled(card, 6.0, Color32::from_rgba_unmultiplied(20, 16, 30, 225));
    painter.text(card.center_top() + vec2(0.0, 22.0), Align2::CENTER_TOP, "Kantor ini masih kosong…", FontId::proportional(22.0), Color32::WHITE);
    painter.text(
        card.center_top() + vec2(0.0, 58.0),
        Align2::CENTER_TOP,
        "Rekrut karyawan pertama: setiap folder project mendapat satu meja.",
        FontId::proportional(14.0),
        Color32::from_white_alpha(210),
    );
    let b1 = Rect::from_center_size(card.center_bottom() + vec2(-95.0, -38.0), vec2(170.0, 34.0));
    let b2 = Rect::from_center_size(card.center_bottom() + vec2(95.0, -38.0), vec2(170.0, 34.0));
    if ui.put(b1, IconButton::new(Icon::FolderPlus, "Tambah Folder")).clicked() {
        actions.push(Action::AddProject);
    }
    if ui.put(b2, IconButton::new(Icon::Sparkle, "Project Baru")).clicked() {
        actions.push(Action::NewProject);
    }
}

fn prompt_bar(ui: &mut Ui, projects: &mut [Project], w: &mut OfficeUi, settings: &mut Settings, actions: &mut Vec<Action>) {
    if w.target >= projects.len() {
        w.target = 0;
    }
    let mut send = false;
    ui.horizontal(|ui| {
        if chat::provider_picker(ui, settings, "office_provider") {
            actions.push(Action::SaveConfig);
        }
        ui.label(RichText::new("→").weak());
        egui::ComboBox::from_id_salt("office_target")
            .selected_text(projects[w.target].entry.name.clone())
            .width(150.0)
            .show_ui(ui, |ui| {
                for (i, p) in projects.iter().enumerate() {
                    let label = if p.agent.is_some() { format!("⚡ {}", p.entry.name) } else { p.entry.name.clone() };
                    ui.selectable_value(&mut w.target, i, label);
                }
            });
        let target = &projects[w.target];
        let busy = target.agent.is_some();
        let width = (ui.available_width() - 110.0).max(120.0);
        let r = ui.add(
            TextEdit::singleline(&mut w.prompt)
                .desired_width(width)
                .hint_text(format!("Beri tugas ke {}… (Enter untuk kirim)", target.entry.name)),
        );
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            send = true;
            r.request_focus();
        }
        if busy {
            if ui.add(IconButton::new(Icon::Stop, "Stop").kind(Kind::Danger)).clicked() {
                actions.push(Action::Stop(w.target));
            }
        } else if ui
            .add(IconButton::new(Icon::Send, "Kirim").kind(Kind::Primary))
            .clicked()
        {
            send = true;
        }
    });
    let target = &projects[w.target];
    if target.agent.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new(format!("{}: {}", target.entry.name, target.status)).color(theme::WARM).small());
            ui.label(RichText::new("— klik mejanya untuk melihat kode berubah live").small().weak());
        });
    }
    if send && !w.prompt.trim().is_empty() && target.agent.is_none() {
        actions.push(Action::Send {
            project: w.target,
            prompt: std::mem::take(&mut w.prompt).trim().to_string(),
        });
    }
}
