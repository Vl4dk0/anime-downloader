#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod ui;

/// Downscaled copy of `assets/icon.png`: window, Dock and taskbar icons never
/// need the full 1024 px master, so the binary stays small.
const WINDOW_ICON_PNG: &[u8] = include_bytes!("../assets/icon-256.png");

use anime_downloader::site::http::build_client;
use anyhow::{Context, Result};
use eframe::egui;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("start background runtime")?;
    let http = build_client()?;
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Anime Downloader")
        .with_inner_size([900.0, 680.0])
        .with_min_inner_size([580.0, 440.0]);
    match eframe::icon_data::from_png_bytes(WINDOW_ICON_PNG) {
        Ok(icon) => viewport = viewport.with_icon(icon),
        Err(err) => tracing::warn!("window icon unavailable: {err}"),
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Anime Downloader",
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, runtime, http)))),
    )
    .map_err(|error| anyhow::anyhow!("open application window: {error}"))
}
