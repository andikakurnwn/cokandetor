//! Line icons in the style of VS Code's codicons, and the buttons that use
//! them. Painted as vector shapes so every icon sits exactly centred next to
//! its label (emoji glyphs have their own metrics and drift off-centre).

use egui::{
    Color32, FontId, Galley, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, TextStyle, Ui, Widget,
    pos2, vec2,
};
use std::f32::consts::{PI, TAU};
use std::sync::Arc;

use crate::theme::{self, vs};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Icon {
    Files,
    File,
    Search,
    Database,
    Chat,
    Home,
    Gear,
    Plus,
    Refresh,
    Collapse,
    Edit,
    Play,
    Stop,
    Send,
    Trash,
    Folder,
    FolderPlus,
    Sparkle,
    Bell,
    Clock,
    Branch,
    Close,
    Target,
    Robot,
    ChevronRight,
    ChevronDown,
}

/// Paints `icon` centred on `c`, scaled to fit a `size`-point box.
pub fn paint(painter: &Painter, icon: Icon, c: Pos2, size: f32, color: Color32) {
    let s = size / 22.0;
    let p = |x: f32, y: f32| c + vec2(x * s, y * s);
    let w = (1.6 * s).max(1.2);
    let st = Stroke::new(w, color);
    let line = |pts: &[(f32, f32)]| painter.add(Shape::line(pts.iter().map(|&(x, y)| p(x, y)).collect(), st));
    let closed = |pts: &[(f32, f32)]| painter.add(Shape::closed_line(pts.iter().map(|&(x, y)| p(x, y)).collect(), st));
    let fill = |pts: &[(f32, f32)]| {
        painter.add(Shape::convex_polygon(pts.iter().map(|&(x, y)| p(x, y)).collect(), color, Stroke::NONE))
    };
    let arc = |cx: f32, cy: f32, r: f32, a0: f32, a1: f32| {
        let n = 24;
        let pts: Vec<Pos2> = (0..=n)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / n as f32;
                p(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        painter.add(Shape::line(pts, st));
    };
    match icon {
        Icon::Files => {
            line(&[(-5.0, -10.0), (4.0, -10.0), (9.0, -5.0), (9.0, 6.0)]);
            closed(&[(-9.0, -6.0), (0.0, -6.0), (5.0, -1.0), (5.0, 11.0), (-9.0, 11.0)]);
        }
        Icon::File => {
            closed(&[(-6.0, -9.0), (2.0, -9.0), (6.0, -5.0), (6.0, 9.0), (-6.0, 9.0)]);
            line(&[(2.0, -9.0), (2.0, -5.0), (6.0, -5.0)]);
        }
        Icon::Search => {
            painter.circle_stroke(p(2.0, -2.0), 7.0 * s, st);
            painter.line_segment([p(-3.0, 3.0), p(-9.0, 9.0)], Stroke::new(w * 1.4, color));
        }
        Icon::Database => {
            painter.add(Shape::ellipse_stroke(p(0.0, -7.0), vec2(8.0 * s, 3.0 * s), st));
            line(&[(-8.0, -7.0), (-8.0, 7.0)]);
            line(&[(8.0, -7.0), (8.0, 7.0)]);
            for dy in [0.0, 7.0] {
                let pts: Vec<Pos2> = (0..=12)
                    .map(|i| {
                        let a = PI * i as f32 / 12.0;
                        p(-8.0 * a.cos(), dy + 3.0 * a.sin())
                    })
                    .collect();
                painter.add(Shape::line(pts, st));
            }
        }
        Icon::Chat => {
            closed(&[(-10.0, -8.0), (10.0, -8.0), (10.0, 5.0), (-1.0, 5.0), (-6.0, 10.0), (-6.0, 5.0), (-10.0, 5.0)]);
            for dx in [-5.0, 0.0, 5.0] {
                painter.circle_filled(p(dx, -1.5), 1.4 * s, color);
            }
        }
        Icon::Home => {
            closed(&[(-9.0, -1.0), (0.0, -10.0), (9.0, -1.0), (7.0, -1.0), (7.0, 9.0), (-7.0, 9.0), (-7.0, -1.0)]);
            closed(&[(-2.5, 3.0), (2.5, 3.0), (2.5, 9.0), (-2.5, 9.0)]);
        }
        Icon::Gear => {
            painter.circle_stroke(c, 6.5 * s, st);
            painter.circle_stroke(c, 2.5 * s, st);
            for i in 0..8 {
                let a = i as f32 / 8.0 * TAU;
                let d = vec2(a.cos(), a.sin());
                painter.line_segment([c + d * 6.5 * s, c + d * 9.5 * s], Stroke::new(w * 1.5, color));
            }
        }
        Icon::Plus => {
            line(&[(-7.0, 0.0), (7.0, 0.0)]);
            line(&[(0.0, -7.0), (0.0, 7.0)]);
        }
        Icon::Refresh => {
            let (a0, a1) = (-0.25 * PI, 1.35 * PI);
            arc(0.0, 0.0, 7.0, a0, a1);
            // Arrow head at the start of the arc, pointing along it.
            let e = p(7.0 * a0.cos(), 7.0 * a0.sin());
            let d = vec2(a0.sin(), -a0.cos()); // against the direction of travel
            let n = vec2(a0.cos(), a0.sin());
            painter.add(Shape::convex_polygon(
                vec![e + d * 3.5 * s, e + n * 3.0 * s, e - n * 3.0 * s],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Collapse => {
            painter.rect_stroke(Rect::from_min_max(p(-7.0, -7.0), p(7.0, 7.0)), 1.5 * s, st, StrokeKind::Middle);
            line(&[(-3.5, 0.0), (3.5, 0.0)]);
        }
        Icon::Edit => {
            closed(&[(3.5, -6.5), (6.5, -3.5), (-3.5, 6.5), (-7.0, 7.0), (-6.5, 3.5)]);
            line(&[(1.0, -4.0), (4.0, -1.0)]);
        }
        Icon::Play => {
            fill(&[(-4.5, -6.5), (6.5, 0.0), (-4.5, 6.5)]);
        }
        Icon::Stop => {
            painter.rect_filled(Rect::from_min_max(p(-5.0, -5.0), p(5.0, 5.0)), 1.0 * s, color);
        }
        Icon::Send => {
            fill(&[(-8.0, -7.0), (8.0, 0.0), (-5.0, 0.0)]);
            fill(&[(-5.0, 0.0), (8.0, 0.0), (-8.0, 7.0)]);
        }
        Icon::Trash => {
            line(&[(-7.5, -5.0), (7.5, -5.0)]);
            line(&[(-2.5, -5.0), (-2.5, -7.5), (2.5, -7.5), (2.5, -5.0)]);
            closed(&[(-5.5, -5.0), (5.5, -5.0), (4.5, 8.0), (-4.5, 8.0)]);
            line(&[(-1.8, -1.5), (-1.8, 5.0)]);
            line(&[(1.8, -1.5), (1.8, 5.0)]);
        }
        Icon::Folder | Icon::FolderPlus => {
            closed(&[(-9.0, -6.0), (-3.0, -6.0), (-1.0, -4.0), (9.0, -4.0), (9.0, 7.0), (-9.0, 7.0)]);
            if icon == Icon::FolderPlus {
                line(&[(-3.0, 1.5), (3.0, 1.5)]);
                line(&[(0.0, -1.5), (0.0, 4.5)]);
            }
        }
        Icon::Sparkle => {
            let star = |cx: f32, cy: f32, r: f32, inner: f32| {
                for k in 0..4 {
                    let a = k as f32 * PI / 2.0 - PI / 2.0;
                    let tip = (cx + r * a.cos(), cy + r * a.sin());
                    let l = (cx + inner * (a - PI / 4.0).cos(), cy + inner * (a - PI / 4.0).sin());
                    let rr = (cx + inner * (a + PI / 4.0).cos(), cy + inner * (a + PI / 4.0).sin());
                    fill(&[l, tip, rr]);
                }
                fill(&[(cx - inner, cy), (cx, cy - inner), (cx + inner, cy), (cx, cy + inner)]);
            };
            star(-1.5, 1.5, 7.5, 2.2);
            star(5.5, -5.5, 3.5, 1.0);
        }
        Icon::Bell => {
            arc(0.0, -1.0, 6.0, PI, TAU);
            line(&[(-6.0, -1.0), (-6.0, 4.0), (-8.0, 6.0), (8.0, 6.0), (6.0, 4.0), (6.0, -1.0)]);
            painter.circle_filled(p(0.0, 8.5), 1.7 * s, color);
            painter.circle_filled(p(0.0, -8.0), 1.1 * s, color);
        }
        Icon::Clock => {
            painter.circle_stroke(c, 8.0 * s, st);
            line(&[(0.0, -5.0), (0.0, 0.0), (4.0, 2.5)]);
        }
        Icon::Branch => {
            for (x, y) in [(-4.0, -6.5), (-4.0, 6.5), (5.0, -3.5)] {
                painter.circle_stroke(p(x, y), 2.2 * s, st);
            }
            line(&[(-4.0, -4.3), (-4.0, 4.3)]);
            line(&[(5.0, -1.3), (5.0, 0.5), (-2.5, 4.8)]);
        }
        Icon::Close => {
            line(&[(-6.0, -6.0), (6.0, 6.0)]);
            line(&[(6.0, -6.0), (-6.0, 6.0)]);
        }
        Icon::Target => {
            painter.circle_stroke(c, 7.5 * s, st);
            painter.circle_filled(c, 2.5 * s, color);
        }
        Icon::ChevronRight => {
            line(&[(-3.0, -7.0), (4.0, 0.0), (-3.0, 7.0)]);
        }
        Icon::ChevronDown => {
            line(&[(-7.0, -3.0), (0.0, 4.0), (7.0, -3.0)]);
        }
        Icon::Robot => {
            painter.rect_stroke(Rect::from_min_max(p(-7.5, -4.0), p(7.5, 8.0)), 2.5 * s, st, StrokeKind::Middle);
            painter.circle_filled(p(-3.2, 2.0), 1.5 * s, color);
            painter.circle_filled(p(3.2, 2.0), 1.5 * s, color);
            line(&[(0.0, -4.0), (0.0, -7.5)]);
            painter.circle_filled(p(0.0, -8.5), 1.4 * s, color);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// Regular egui-styled button.
    Normal,
    /// Filled blue, like VS Code's primary button.
    Primary,
    /// Filled red.
    Danger,
}

fn lighten(c: Color32, t: f32) -> Color32 {
    let f = |v: u8| (v as f32 + (255.0 - v as f32) * t).round() as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

/// A button with a vector icon and an optional label, vertically centred.
pub struct IconButton {
    icon: Icon,
    text: String,
    kind: Kind,
}

impl IconButton {
    pub fn new(icon: Icon, text: impl Into<String>) -> Self {
        Self {
            icon,
            text: text.into(),
            kind: Kind::Normal,
        }
    }

    pub fn kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        self
    }
}

const ICON: f32 = 15.0;
const GAP: f32 = 6.0;

fn label(ui: &Ui, text: &str) -> Arc<Galley> {
    let font = TextStyle::Button.resolve(ui.style());
    ui.painter().layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER)
}

impl Widget for IconButton {
    fn ui(self, ui: &mut Ui) -> Response {
        let galley = (!self.text.is_empty()).then(|| label(ui, &self.text));
        let pad = ui.spacing().button_padding;
        let text_w = galley.as_ref().map(|g| g.size().x + GAP).unwrap_or(0.0);
        let text_h = galley.as_ref().map(|g| g.size().y).unwrap_or(ICON);
        let size = vec2(
            pad.x * 2.0 + ICON + text_w,
            (text_h + pad.y * 2.0).max(ui.spacing().interact_size.y),
        );
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        if ui.is_rect_visible(rect) {
            let v = ui.style().interact(&resp);
            let hot = resp.hovered() || resp.has_focus();
            let (fill, fg, stroke) = match self.kind {
                Kind::Normal => (v.weak_bg_fill, v.text_color(), v.bg_stroke),
                Kind::Primary => {
                    let f = if hot { lighten(vs::FOCUS_BLUE, 0.12) } else { vs::FOCUS_BLUE };
                    (f, Color32::WHITE, Stroke::NONE)
                }
                Kind::Danger => {
                    let f = if hot { lighten(theme::DANGER, 0.12) } else { theme::DANGER };
                    (f, Color32::WHITE, Stroke::NONE)
                }
            };
            let (fill, fg) = if ui.is_enabled() { (fill, fg) } else { (fill.gamma_multiply(0.5), fg.gamma_multiply(0.45)) };
            let painter = ui.painter();
            painter.rect(rect.expand(v.expansion), v.corner_radius, fill, stroke, StrokeKind::Inside);
            // Centre icon + label as one group (they may be wider than needed when justified).
            let content_w = ICON + text_w;
            let left = rect.center().x - content_w / 2.0;
            paint(painter, self.icon, pos2(left + ICON / 2.0, rect.center().y), ICON, fg);
            if let Some(g) = galley {
                let pos = pos2(left + ICON + GAP, rect.center().y - g.size().y / 2.0);
                painter.galley(pos, g, fg);
            }
        }
        resp
    }
}

pub fn button(ui: &mut Ui, icon: Icon, text: &str) -> Response {
    ui.add(IconButton::new(icon, text))
}

/// A square, frameless toolbar button (Explorer/Database headers, title bar).
pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::click());
    if ui.is_rect_visible(rect) {
        if resp.hovered() {
            ui.painter().rect_filled(rect, 4.0, Color32::from_white_alpha(22));
        }
        let color = if resp.hovered() { vs::TEXT_BRIGHT } else { vs::TEXT };
        paint(ui.painter(), icon, rect.center(), 15.0, color);
    }
    resp.on_hover_text(tip)
}

/// A context-menu entry with an icon column, highlighted on hover.
pub fn menu_item(ui: &mut Ui, icon: Icon, text: &str) -> Response {
    let g = label(ui, text);
    // A fixed minimum keeps every row (and its hover highlight) the same width;
    // a menu's available width is the whole screen.
    let w = (g.size().x + 44.0).max(210.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hot = resp.hovered();
        if hot {
            ui.painter().rect_filled(rect, 3.0, vs::LIST_FOCUS);
        }
        let fg = if hot { Color32::WHITE } else { vs::TEXT };
        paint(ui.painter(), icon, pos2(rect.left() + 15.0, rect.center().y), 15.0, fg);
        ui.painter().galley(pos2(rect.left() + 32.0, rect.center().y - g.size().y / 2.0), g, fg);
    }
    resp
}

/// A status-bar entry: optional icon plus white text.
pub fn status_item(ui: &mut Ui, icon: Option<Icon>, text: &str) -> Response {
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(12.0), Color32::WHITE);
    let icon_w = if icon.is_some() { 14.0 + if text.is_empty() { 0.0 } else { 5.0 } } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(icon_w + g.size().x, 20.0), Sense::hover());
    if let Some(icon) = icon {
        paint(ui.painter(), icon, pos2(rect.left() + 7.0, rect.center().y), 13.0, Color32::WHITE);
    }
    ui.painter().galley(pos2(rect.left() + icon_w, rect.center().y - g.size().y / 2.0), g, Color32::WHITE);
    resp
}
