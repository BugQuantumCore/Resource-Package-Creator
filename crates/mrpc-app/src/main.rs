//! MTR Resource Package Creator — desktop entry point.
//!
//! A standalone (no Minecraft required) creator for Minecraft Transit Railway
//! 4.0.x custom resource packs: 3D model preview, door animation preview, VVVF
//! audition and JCM (Yanyang) train-script support. Rust + C++, egui + wgpu.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod panels;
mod state;
mod tabs;
mod viewport;

use mrpc_core::i18n::Locale;

use crate::app::MrpcApp;
use crate::state::AppConfig;

fn main() -> eframe::Result<()> {
    let config = AppConfig::load();
    let locale = config
        .locale
        .as_deref()
        .and_then(Locale::from_tag)
        .unwrap_or(Locale::ZhCn);

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1024.0, 640.0])
            .with_title("MTR Resource Package Creator"),
        ..Default::default()
    };

    eframe::run_native(
        "MTR Resource Package Creator",
        options,
        Box::new(|cc| Ok(Box::new(MrpcApp::new(cc, locale, config)))),
    )
}
