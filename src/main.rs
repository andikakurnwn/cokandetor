#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Cokandetor — an offline pixel-office code editor with a built-in AI coding agent.

mod agent;
mod app;
mod chat;
mod clock;
mod config;
mod db;
mod dbview;
mod editor;
mod icons;
mod office;
mod partial_json;
mod project;
mod skill;
mod sprite;
mod theme;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Cokandetor — Pixel Code Studio")
            .with_inner_size([1200.0, 740.0])
            .with_min_inner_size([920.0, 560.0])
            .with_maximized(true)
            .with_icon(sprite::icon(64)),
        ..Default::default()
    };
    eframe::run_native(
        "Cokandetor",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
