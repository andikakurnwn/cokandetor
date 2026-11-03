//! Procedural pixel characters. Every project gets a character whose species
//! and colours come from a hash of its name, and whose size and accessories
//! grow with the project's level.

use egui::ecolor::Hsva;
use egui::{Color32, Painter, Pos2, Rect, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Species {
    Coder,
    Robot,
    Slime,
    Cat,
}

#[derive(Clone, Debug)]
pub struct Look {
    pub species: Species,
    pub hair: Color32,
    pub skin: Color32,
    pub shirt: Color32,
    pub pants: Color32,
    pub accent: Color32,
    pub hat: Color32,
}

const OUTLINE: Color32 = Color32::from_rgb(0x1a, 0x16, 0x26);
const EYE: Color32 = Color32::from_rgb(0x1a, 0x16, 0x26);
const WHITE: Color32 = Color32::from_rgb(0xf4, 0xf4, 0xf4);
const MOUTH: Color32 = Color32::from_rgb(0x7a, 0x2f, 0x3a);
const CHEEK: Color32 = Color32::from_rgb(0xff, 0x8f, 0xa3);
const SHOES: Color32 = Color32::from_rgb(0x2b, 0x2b, 0x36);
const GEM: Color32 = Color32::from_rgb(0xe8, 0x3b, 0x4f);
const LAPTOP: Color32 = Color32::from_rgb(0x5a, 0x60, 0x70);
const LAPTOP_DARK: Color32 = Color32::from_rgb(0x34, 0x38, 0x44);

// Sprite grids are 16x16. Legend:
//  o outline  H hair  S skin  s skin shade  E eye  W white  B shirt  b shirt shade
//  P pants  F shoes  A accent  M mouth  C cheek
const CODER: [&str; 16] = [
    "................",
    "....oooooooo....",
    "...oHHHHHHHHo...",
    "..oHHHHHHHHHHo..",
    "..oHHSSSSSSHHo..",
    "..oHSSSSSSSSHo..",
    "..oSSESSSSESSo..",
    "..oSSESSSSESSo..",
    "..oSCSSMMSSCSo..",
    "...oSSSSSSSSo...",
    "...ooBBBBBBoo...",
    "..oSBBBAABBBSo..",
    "..oSBBBBBBBBSo..",
    "...ooPPPPPPoo...",
    "....oPPooPPo....",
    "....oFFooFFo....",
];

const ROBOT: [&str; 16] = [
    ".......oA.......",
    ".......o........",
    "...oooooooooo...",
    "...oBBBBBBBBo...",
    "...oBWWBBWWBo...",
    "...oBWEBBWEBo...",
    "...oBBBBBBBBo...",
    "...oBBMMMMBBo...",
    "...oooooooooo...",
    ".....oSSSSo.....",
    "..oooooooooooo..",
    ".oSoHHHAAHHHoSo.",
    ".oSoHHHHHHHHoSo.",
    "..oooooooooooo..",
    "....oSSooSSo....",
    "...oFFFooFFFo...",
];

const SLIME: [&str; 16] = [
    "................",
    "................",
    "................",
    "................",
    "......oooo......",
    "....ooBBBBoo....",
    "...oBBWBBBBBo...",
    "..oBBWWBBBBBBo..",
    "..oBBBBBBBBBBo..",
    ".oBBBEBBBBEBBBo.",
    ".oBBBEBBBBEBBBo.",
    ".oBBCBBMMBBCBBo.",
    ".oBBBBBBBBBBBBo.",
    ".obbbbbbbbbbbbo.",
    "..oooooooooooo..",
    "................",
];

const CAT: [&str; 16] = [
    "................",
    "..oo........oo..",
    "..oHo......oHo..",
    "..oHHooooooHHo..",
    "..oSSSSSSSSSSo..",
    ".oSSSSSSSSSSSSo.",
    ".oSSEESSSSEESSo.",
    ".oSSEWSSSSEWSSo.",
    ".oSCSSSMMSSSCSo.",
    "..oSSSSSSSSSSo..",
    "...ooBBBBBBoo...",
    "..oSBBBAABBBSo..",
    "..oSBBBBBBBBSoSo",
    "...oBBBBBBBBooSo",
    "....oSSooSSo....",
    "....osSoosSo....",
];

/// Alternate rows 14 and 15 for the walking frame.
fn walk_rows(species: Species) -> Option<[&'static str; 2]> {
    match species {
        Species::Coder => Some(["...oPPo..oPPo...", "...oFFo..oFFo..."]),
        Species::Robot => Some(["...oSSo..oSSo...", "..oFFFo..oFFFo.."]),
        Species::Cat => Some(["...oSSo..oSSo...", "...osSo..osSo..."]),
        Species::Slime => None,
    }
}

fn head_top(species: Species) -> i32 {
    match species {
        Species::Coder => 1,
        Species::Robot => 2,
        Species::Slime => 4,
        Species::Cat => 3,
    }
}

// Hats. K = hat colour, A = accent, R = gem.
const BEANIE: [&str; 6] = [
    "....oo....",
    "...oAAo...",
    "..oKKKKo..",
    ".oKKKKKKo.",
    "oKKKKKKKKo",
    "oooooooooo",
];
const HEADPHONES: [&str; 6] = [
    "...oooooooo...",
    "..oK......Ko..",
    ".oK........Ko.",
    "oKKo......oKKo",
    "oKKo......oKKo",
    "oooo......oooo",
];
const WIZARD: [&str; 8] = [
    ".......oo...",
    "......oKKo..",
    ".....oKKo...",
    "....oKKKo...",
    "....oKAKKo..",
    "...oKKKKKKo.",
    "..oKKKKKKKo.",
    "oooooooooooo",
];
const CROWN: [&str; 5] = [
    "o..oo..o",
    "oAoAAoAo",
    "oAAAAAAo",
    "oAARRAAo",
    "oooooooo",
];

fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    Hsva::new(h.rem_euclid(1.0), s, v, 1.0).into()
}

fn shade(c: Color32, f: f32) -> Color32 {
    Color32::from_rgb(
        (c.r() as f32 * f) as u8,
        (c.g() as f32 * f) as u8,
        (c.b() as f32 * f) as u8,
    )
}

impl Look {
    pub fn from_name(name: &str) -> Self {
        let h = crate::config::fnv(&name.to_lowercase());
        let f = |shift: u32| ((h >> shift) & 0xff) as f32 / 255.0;
        let species = match h % 4 {
            0 => Species::Coder,
            1 => Species::Robot,
            2 => Species::Slime,
            _ => Species::Cat,
        };
        let skins = [
            Color32::from_rgb(0xff, 0xdb, 0xac),
            Color32::from_rgb(0xf1, 0xc2, 0x7d),
            Color32::from_rgb(0xe0, 0xac, 0x69),
            Color32::from_rgb(0xc6, 0x86, 0x42),
            Color32::from_rgb(0x8d, 0x55, 0x24),
        ];
        let furs = [
            Color32::from_rgb(0xf0, 0xa0, 0x4b),
            Color32::from_rgb(0x9a, 0xa0, 0xa6),
            Color32::from_rgb(0x4a, 0x44, 0x50),
            Color32::from_rgb(0xf2, 0xee, 0xe6),
            Color32::from_rgb(0xa0, 0x6a, 0x3d),
        ];
        let pick = ((h >> 40) % 5) as usize;
        let (hair, skin) = match species {
            Species::Coder => (hsv(f(8), 0.55, 0.45 + 0.4 * f(48)), skins[pick]),
            Species::Robot => (hsv(f(8), 0.45, 0.65), hsv(f(16), 0.06, 0.78)),
            Species::Slime => (hsv(f(8), 0.5, 0.9), hsv(f(8), 0.5, 0.9)),
            Species::Cat => (Color32::from_rgb(0xff, 0x9f, 0xb2), furs[pick]),
        };
        Self {
            species,
            hair,
            skin,
            shirt: hsv(f(16), 0.6, 0.82),
            pants: hsv(f(24), 0.35, 0.38),
            accent: hsv(0.12 + 0.05 * f(32), 0.75, 0.98),
            hat: hsv(f(56), 0.6, 0.7),
        }
    }

    fn color(&self, c: u8) -> Option<Color32> {
        Some(match c {
            b'o' => OUTLINE,
            b'H' => self.hair,
            b'S' => self.skin,
            b's' => shade(self.skin, 0.78),
            b'E' => EYE,
            b'W' => WHITE,
            b'B' => match self.species {
                Species::Robot => shade(self.hair, 1.1),
                _ => self.shirt,
            },
            b'b' => shade(self.shirt, 0.75),
            b'P' => self.pants,
            b'F' => SHOES,
            b'A' => self.accent,
            b'M' => MOUTH,
            b'C' => CHEEK,
            b'K' => self.hat,
            b'R' => GEM,
            b'G' => LAPTOP,
            b'g' => LAPTOP_DARK,
            _ => return None,
        })
    }

    fn template(&self) -> &'static [&'static str; 16] {
        match self.species {
            Species::Coder => &CODER,
            Species::Robot => &ROBOT,
            Species::Slime => &SLIME,
            Species::Cat => &CAT,
        }
    }
}

pub struct DrawOpts {
    pub bottom_center: Pos2,
    /// Screen size of one sprite pixel.
    pub px: f32,
    pub flip: bool,
    /// Squash-and-stretch: >1 is wider and flatter.
    pub squash: f32,
    pub blink: bool,
    pub walk_frame: bool,
    pub working: bool,
    pub level: u32,
    pub time: f64,
    pub alpha: f32,
}

/// Pixel size on screen for a given level.
pub fn pixel_size(level: u32) -> f32 {
    2.4 + level as f32 * 0.5
}

struct Canvas<'a> {
    painter: &'a Painter,
    left: f32,
    top: f32,
    pw: f32,
    ph: f32,
    flip: bool,
    alpha: f32,
}

impl Canvas<'_> {
    /// Fill sprite cells [x0, x1) on row y (sprite coordinates, may be negative).
    fn run(&self, x0: i32, x1: i32, y: i32, c: Color32) {
        let (a, b) = if self.flip { (16 - x1, 16 - x0) } else { (x0, x1) };
        let l = (self.left + a as f32 * self.pw).round();
        let r = (self.left + b as f32 * self.pw).round();
        let t = (self.top + y as f32 * self.ph).round();
        let btm = (self.top + (y + 1) as f32 * self.ph).round();
        let c = if self.alpha < 1.0 { c.gamma_multiply(self.alpha) } else { c };
        self.painter
            .rect_filled(Rect::from_min_max(pos2(l, t), pos2(r, btm)), 0.0, c);
    }

    fn grid(&self, rows: &[&[u8]], ox: i32, oy: i32, look: &Look) {
        for (y, row) in rows.iter().enumerate() {
            let mut x = 0;
            while x < row.len() {
                let Some(c) = look.color(row[x]) else {
                    x += 1;
                    continue;
                };
                let start = x;
                while x < row.len() && row[x] == row[start] {
                    x += 1;
                }
                self.run(ox + start as i32, ox + x as i32, oy + y as i32, c);
            }
        }
    }
}

/// Draws the character and returns its screen bounding box.
pub fn draw(painter: &Painter, look: &Look, o: &DrawOpts) -> Rect {
    let sx = o.squash;
    let sy = 1.0 / o.squash;
    let pw = o.px * sx;
    let ph = o.px * sy;
    let left = o.bottom_center.x - 8.0 * pw;
    let top = o.bottom_center.y - 16.0 * ph;
    let cv = Canvas {
        painter,
        left,
        top,
        pw,
        ph,
        flip: o.flip,
        alpha: o.alpha,
    };

    let mut rows: Vec<Vec<u8>> = look.template().iter().map(|r| r.as_bytes().to_vec()).collect();
    if o.walk_frame
        && let Some(w) = walk_rows(look.species) {
            rows[14] = w[0].as_bytes().to_vec();
            rows[15] = w[1].as_bytes().to_vec();
        }
    if o.blink {
        for r in rows.iter_mut() {
            for c in r.iter_mut() {
                if *c == b'E' || (*c == b'W' && look.species == Species::Cat) {
                    *c = if look.species == Species::Slime { b'B' } else { b's' };
                }
            }
        }
    }
    let refs: Vec<&[u8]> = rows.iter().map(|r| r.as_slice()).collect();
    cv.grid(&refs, 0, 0, look);

    // Hat by level.
    let ht = head_top(look.species);
    let hat: Option<&[&str]> = match o.level {
        0..=2 => None,
        3..=4 => Some(&BEANIE),
        5..=6 => Some(&HEADPHONES),
        7..=8 => Some(&WIZARD),
        _ => Some(&CROWN),
    };
    let mut top_row = 0;
    if let Some(hat) = hat {
        let w = hat.iter().map(|r| r.len()).max().unwrap_or(0) as i32;
        let h = hat.len() as i32;
        let is_phones = (5..=6).contains(&o.level);
        let oy = if is_phones { ht - 1 } else { ht + 1 - h };
        let ox = 8 - w / 2;
        let hat_rows: Vec<&[u8]> = hat.iter().map(|r| r.as_bytes()).collect();
        cv.grid(&hat_rows, ox, oy, look);
        top_row = top_row.min(oy);
    }

    if o.working {
        draw_laptop(&cv, look, o.time);
    }

    let bbox = Rect::from_min_max(
        pos2(left, top + top_row as f32 * ph),
        pos2(left + 16.0 * pw, top + 16.0 * ph),
    );

    if o.level >= 10 {
        // Legendary sparkle aura.
        for i in 0..6 {
            let a = o.time as f32 * 1.4 + i as f32 * std::f32::consts::TAU / 6.0;
            let r = vec2(bbox.width() * 0.62, bbox.height() * 0.45);
            let c = bbox.center() + vec2(a.cos() * r.x, a.sin() * r.y);
            let s = (o.px * 0.8).max(2.0);
            let tw = ((o.time * 3.0 + i as f64).sin() * 0.5 + 0.5) as f32;
            painter.rect_filled(
                Rect::from_center_size(c, vec2(s, s)),
                0.0,
                look.accent.gamma_multiply((0.4 + 0.6 * tw) * o.alpha),
            );
        }
    }
    bbox
}

fn draw_laptop(cv: &Canvas, look: &Look, time: f64) {
    let lid: [&[u8]; 5] = [
        b"gggggggggg",
        b"gGGGGGGGGg",
        b"gGGGAAGGGg",
        b"gGGGGGGGGg",
        b"gggggggggg",
    ];
    // The slime's face sits low, so its laptop does too.
    let y = if look.species == Species::Slime { 11 } else { 9 };
    cv.grid(&lid, 3, y, look);
    let base: [&[u8]; 1] = [b"oggggggggggggo"];
    cv.grid(&base, 1, y + 5, look);
    // Tapping hands.
    let tap = (time * 8.0) as i64 % 2 == 0;
    let hand = if look.species == Species::Slime { look.skin } else { shade(look.skin, 0.95) };
    cv.run(2, 3, y + if tap { 4 } else { 5 }, hand);
    cv.run(13, 14, y + if tap { 5 } else { 4 }, hand);
    // Screen glow on the face.
    let glow = (0.25 + 0.12 * (time * 5.0).sin()) as f32;
    let face = y - 3;
    cv.run(4, 12, face, Color32::from_rgba_unmultiplied(120, 220, 255, (glow * 120.0) as u8));
    cv.run(4, 12, face + 1, Color32::from_rgba_unmultiplied(120, 220, 255, (glow * 90.0) as u8));
}

/// Renders a character into an RGBA buffer (used for the window icon).
pub fn icon(size: u32) -> egui::IconData {
    let look = Look {
        species: Species::Coder,
        hair: Color32::from_rgb(0x6b, 0x3f, 0xa0),
        skin: Color32::from_rgb(0xf1, 0xc2, 0x7d),
        shirt: Color32::from_rgb(0x2e, 0xc4, 0xb6),
        pants: Color32::from_rgb(0x3a, 0x3f, 0x58),
        accent: Color32::from_rgb(0xff, 0xd1, 0x66),
        hat: Color32::from_rgb(0x6b, 0x3f, 0xa0),
    };
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let cell = size / 16;
    let off = (size - cell * 16) / 2;
    for (y, row) in CODER.iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            let Some(col) = look.color(c) else { continue };
            for dy in 0..cell {
                for dx in 0..cell {
                    let px = off + x as u32 * cell + dx;
                    let py = off + y as u32 * cell + dy;
                    let i = ((py * size + px) * 4) as usize;
                    rgba[i..i + 4].copy_from_slice(&[col.r(), col.g(), col.b(), 255]);
                }
            }
        }
    }
    egui::IconData {
        rgba,
        width: size,
        height: size,
    }
}
