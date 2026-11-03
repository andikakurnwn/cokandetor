//! Wall-clock time in an Indonesian time zone, and the day/night lighting
//! derived from it. No timezone database needed: Indonesia has no DST.

use egui::Color32;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug)]
pub struct LocalTime {
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// Hours since midnight as a float (e.g. 13.5 for 13:30).
    pub hours: f32,
    pub weekday: usize,
    pub day: u32,
    pub month: u32,
    pub year: i64,
}

pub fn zone_label(offset: i32) -> &'static str {
    match offset {
        8 => "WITA",
        9 => "WIT",
        _ => "WIB",
    }
}

const DAYS: [&str; 7] = ["Minggu", "Senin", "Selasa", "Rabu", "Kamis", "Jumat", "Sabtu"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "Mei", "Jun", "Jul", "Agu", "Sep", "Okt", "Nov", "Des"];

impl LocalTime {
    pub fn date_label(&self) -> String {
        format!(
            "{}, {} {} {}",
            DAYS[self.weekday],
            self.day,
            MONTHS[(self.month as usize).saturating_sub(1).min(11)],
            self.year
        )
    }

    pub fn hm(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }
}

/// Current time at UTC+`offset`; `override_hour` pins the hour (preview mode).
pub fn now(offset: i32, override_hour: Option<f32>) -> LocalTime {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let local = secs + offset as f64 * 3600.0;
    let days = (local / 86_400.0).floor() as i64;
    let mut sod = local - days as f64 * 86_400.0;
    if let Some(h) = override_hour {
        sod = (h.rem_euclid(24.0) as f64) * 3600.0;
    }
    let (year, month, day) = civil_from_days(days);
    LocalTime {
        hour: (sod / 3600.0) as u32,
        minute: ((sod % 3600.0) / 60.0) as u32,
        second: (sod % 60.0) as u32,
        hours: (sod / 3600.0) as f32,
        weekday: (days + 4).rem_euclid(7) as usize,
        day,
        month,
        year,
    }
}

/// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 0 at night, 1 in full daylight. Near the equator sunrise is ~05:45 and
/// sunset ~18:00 all year round.
pub fn daylight(h: f32) -> f32 {
    smooth(5.0, 6.6, h) * (1.0 - smooth(17.4, 18.7, h))
}

/// Sky gradient (top, horizon) for the hour.
pub fn sky(h: f32) -> (Color32, Color32) {
    let night = (Color32::from_rgb(0x0b, 0x10, 0x26), Color32::from_rgb(0x1d, 0x2b, 0x4f));
    let dawn = (Color32::from_rgb(0x3f, 0x56, 0x95), Color32::from_rgb(0xf7, 0xa8, 0x6b));
    let day = (Color32::from_rgb(0x3d, 0x9b, 0xe0), Color32::from_rgb(0xbf, 0xe6, 0xff));
    let dusk = (Color32::from_rgb(0x3b, 0x37, 0x7a), Color32::from_rgb(0xf2, 0x86, 0x5a));
    let keys: [(f32, (Color32, Color32)); 8] = [
        (0.0, night),
        (4.6, night),
        (5.6, dawn),
        (6.8, day),
        (16.9, day),
        (17.9, dusk),
        (18.9, night),
        (24.0, night),
    ];
    for w in keys.windows(2) {
        let (h0, c0) = w[0];
        let (h1, c1) = w[1];
        if h >= h0 && h <= h1 {
            let t = (h - h0) / (h1 - h0);
            return (lerp(c0.0, c1.0, t), lerp(c0.1, c1.1, t));
        }
    }
    night
}

/// Greeting for the office sign.
pub fn greeting(h: f32) -> &'static str {
    match h as u32 {
        4..=10 => "Selamat pagi",
        11..=14 => "Selamat siang",
        15..=17 => "Selamat sore",
        _ => "Selamat malam",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_760), (2026, 11, 3));
    }

    #[test]
    fn day_and_night() {
        assert!(daylight(12.0) > 0.99);
        assert!(daylight(23.0) < 0.01);
        assert!(daylight(2.0) < 0.01);
    }
}
