#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod ui;

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
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Anime Downloader")
            .with_inner_size([760.0, 620.0])
            .with_min_inner_size([460.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Anime Downloader",
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, runtime, http)))),
    )
    .map_err(|error| anyhow::anyhow!("open application window: {error}"))
}
