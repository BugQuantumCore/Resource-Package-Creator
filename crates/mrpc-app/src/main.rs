//! MTR Resource Package Creator — desktop entry point.
//!
//! A standalone (no Minecraft required) creator for Minecraft Transit Railway
//! 4.0.x custom resource packs: 3D model preview, door animation preview, VVVF
//! audition and JCM (Yanyang) train-script support. Rust + C++, egui + wgpu.
//!
//! Startup strategy: because release builds run without a console
//! (`windows_subsystem = "windows"`), all diagnostics go to the file logger in
//! [`crate::logging`] first. The app then tries to start with a hardware GPU;
//! if that fails (broken/locked driver, remote session without GPU access…)
//! it retries once with a software adapter (WARP on Windows, llvmpipe on
//! Linux) before giving up with a native error dialog. eframe keeps the
//! window hidden until the first frame has been presented, so the first
//! present uses a non-blocking present mode (`AutoNoVsync`, see
//! [`native_options`]) and a startup watchdog reports (and ends) a hung
//! first frame instead of leaving a windowless process behind.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod logging;
mod panels;
mod state;
mod tabs;
mod viewport;

use std::sync::Arc;

use egui_wgpu::wgpu;
use egui_wgpu::WgpuSetup;

use mrpc_core::i18n::Locale;

use crate::app::MrpcApp;
use crate::logging::Stage;
use crate::state::AppConfig;

const APP_NAME: &str = "MTR Resource Package Creator";

/// Seconds without a first frame before the startup watchdog reports a stall
/// and gives up (a healthy start completes in well under one second, even on
/// slow disks, so this only trips on real hangs).
const STARTUP_WATCHDOG_TIMEOUT_SECS: u64 = 20;

/// Which kind of wgpu adapter to prefer for this launch attempt.
#[derive(Clone, Copy, Debug)]
enum RendererPlan {
    /// Hardware GPU first; a software adapter is accepted only as a last
    /// resort (so machines with a broken-but-present driver still start).
    HardwareFirst,
    /// Software adapter forced: WARP (Windows/DX12), llvmpipe (Linux/GL).
    ForceSoftware,
}

impl RendererPlan {
    fn describe(self) -> &'static str {
        match self {
            RendererPlan::HardwareFirst => "hardware GPU (software as last resort)",
            RendererPlan::ForceSoftware => "software renderer (WARP / llvmpipe)",
        }
    }
}

fn main() -> eframe::Result<()> {
    logging::init();
    logging::install_panic_hook();
    logging::set_stage(Stage::LoggerReady);
    logging::spawn_startup_watchdog(STARTUP_WATCHDOG_TIMEOUT_SECS);

    let config = AppConfig::load();
    let locale = config
        .locale
        .as_deref()
        .and_then(Locale::from_tag)
        .unwrap_or(Locale::ZhCn);

    let mut last_err: Option<String> = None;
    for plan in [RendererPlan::HardwareFirst, RendererPlan::ForceSoftware] {
        log::info!("launch attempt: {}", plan.describe());
        logging::set_stage(Stage::RunNative);
        let options = native_options(plan);
        let cfg = config.clone();
        let result = eframe::run_native(
            APP_NAME,
            options,
            Box::new(move |cc| {
                logging::set_stage(Stage::AppCreator);
                log::info!("app creator: building the application…");
                let app = MrpcApp::new(cc, locale, cfg);
                logging::set_stage(Stage::AppCreated);
                log::info!("app creator: done");
                Ok(Box::new(app))
            }),
        );
        match result {
            Ok(res) => {
                log::info!("application exited normally");
                return Ok(res);
            }
            Err(err) => {
                log::error!("launch attempt failed ({plan:?}): {err}");
                last_err = Some(err.to_string());
            }
        }
    }

    // Both attempts failed: make the failure visible instead of exiting
    // silently behind the windows_subsystem attribute.
    let log_hint = match logging::log_path() {
        Some(p) => format!("A full log was written to:\n{}", p.display()),
        None => "No log file could be written.".to_string(),
    };
    logging::alert(
        "MTR Resource Package Creator failed to start",
        &format!(
            "{} could not create a window or graphics device.\n\n\
             Reason: {}\n\n\
             Both the hardware GPU and the software renderer (WARP) were tried.\n\n\
             {log_hint}\n\n\
             Tips:\n\
             \u{2022} Update your graphics driver and try again.\n\
             \u{2022} Try setting the environment variable WGPU_BACKEND=gl.\n\
             \u{2022} Try setting MRPC_NO_AUDIO=1 to rule out audio-driver issues.\n\n\
             Please report this at:\n\
             https://github.com/BugQuantumCore/Resource-Package-Creator/issues",
            APP_NAME,
            last_err.as_deref().unwrap_or("unknown error")
        ),
    );
    std::process::exit(1);
}

fn native_options(plan: RendererPlan) -> eframe::NativeOptions {
    let mut wgpu_options = eframe::NativeOptions::default().wgpu_options;
    // Present mode: eframe keeps the window hidden until the first frame has
    // been painted, and a FIFO (`AutoVsync`) first present on a still-hidden
    // window has been observed to block indefinitely with some Windows +
    // NVIDIA driver combinations — leaving "a process, but no window". Mailbox
    // / Immediate (`AutoNoVsync`) avoids that waiting path, and since egui
    // repaints on demand there is no real GPU cost. Set MRPC_VSYNC=1 to opt
    // back into FIFO vsync.
    let vsync = std::env::var("MRPC_VSYNC")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);
    wgpu_options.surface.present_mode = if vsync {
        log::info!("present mode: AutoVsync (FIFO; forced by MRPC_VSYNC=1)");
        wgpu::PresentMode::AutoVsync
    } else {
        log::info!("present mode: AutoNoVsync (mailbox/immediate; MRPC_VSYNC=1 opts back into FIFO)");
        wgpu::PresentMode::AutoNoVsync
    };
    if let WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        // The default backend set already covers WARP (DX12) and llvmpipe (GL);
        // `Backends::all()` would additionally enumerate the Noop stub backend,
        // so it is deliberately NOT used.
        let selector: egui_wgpu::NativeAdapterSelectorMethod = match plan {
            RendererPlan::HardwareFirst => Arc::new(select_adapter_hardware_first),
            RendererPlan::ForceSoftware => Arc::new(select_adapter_software_first),
        };
        setup.native_adapter_selector = Some(selector);
    }

    eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1024.0, 640.0])
            .with_title(APP_NAME),
        wgpu_options,
        ..Default::default()
    }
}

/// Adapter score: bigger = better for a normal hardware launch.
fn adapter_score(adapter: &wgpu::Adapter) -> i64 {
    let info = adapter.get_info();
    device_backend_score(info.device_type, info.backend)
}

/// Adapter score from the adapter's device type and backend, kept pure so it
/// can be unit-tested without a GPU.
fn device_backend_score(device_type: wgpu::DeviceType, backend: wgpu::Backend) -> i64 {
    let device = match device_type {
        wgpu::DeviceType::DiscreteGpu => 40,
        wgpu::DeviceType::IntegratedGpu => 30,
        // Apple Silicon Metal adapters report `Other`.
        wgpu::DeviceType::Other => 20,
        wgpu::DeviceType::VirtualGpu => 10,
        wgpu::DeviceType::Cpu => 0, // WARP / llvmpipe / lavapipe
    };
    // On Windows, prefer Vulkan over DX12 when both are available: the
    // DX12 backend has been the one hanging at startup with some NVIDIA
    // driver releases (2025-2026), while the same machines run Vulkan fine.
    // DX12 is still preferred over GL and remains the software-renderer
    // (WARP) path. `max_by_key` keeps the LAST maximum on ties, so these
    // scores are deliberately distinct per hardware class.
    let backend = match backend {
        wgpu::Backend::Metal => 5,
        wgpu::Backend::Vulkan => 5,
        wgpu::Backend::Dx12 => 3,
        wgpu::Backend::Gl => 1,
        _ => 0,
    };
    device * 10 + backend
}

/// Prefer hardware adapters, fall back to a software one only if nothing else
/// works. Applied on the first launch attempt.
fn select_adapter_hardware_first(
    adapters: &[wgpu::Adapter],
    surface: Option<&wgpu::Surface<'_>>,
) -> Result<wgpu::Adapter, String> {
    if adapters.is_empty() {
        return Err("no graphics adapter was found on this system".to_string());
    }
    let supports_surface =
        |a: &wgpu::Adapter| surface.map_or(true, |s| a.is_surface_supported(s));
    // Deterministic choice: on equal scores the FIRST adapter wins (the same
    // order stock egui/wgpu uses), instead of `max_by_key`'s last-max quirk.
    let mut best: Option<&wgpu::Adapter> = None;
    for a in adapters.iter().filter(|a| supports_surface(a)) {
        if best.is_none_or(|b| adapter_score(a) > adapter_score(b)) {
            best = Some(a);
        }
    }
    let Some(best) = best else {
        return Err("none of the available adapters can render to a window".to_string());
    };
    let info = best.get_info();
    log::info!(
        "selected adapter: {} ({:?}, {:?})",
        info.name,
        info.device_type,
        info.backend
    );
    Ok(best.clone())
}

/// Prefer software adapters (WARP / llvmpipe). Applied on the retry after a
/// failed hardware launch.
fn select_adapter_software_first(
    adapters: &[wgpu::Adapter],
    surface: Option<&wgpu::Surface<'_>>,
) -> Result<wgpu::Adapter, String> {
    if adapters.is_empty() {
        return Err("no graphics adapter was found on this system".to_string());
    }
    // Adapters that can present to the window are always preferred, even if
    // all of them are hardware ones.
    let mut pool: Vec<&wgpu::Adapter> = adapters
        .iter()
        .filter(|a| surface.map_or(true, |s| a.is_surface_supported(s)))
        .collect();
    if pool.is_empty() {
        log::warn!("no adapter supports the window surface; taking any adapter");
        pool = adapters.iter().collect();
    }
    // Lowest score = most software-like adapter.
    let best = pool
        .into_iter()
        .min_by_key(|a| adapter_score(a))
        .expect("pool is non-empty");
    let info = best.get_info();
    log::info!(
        "selected fallback adapter: {} ({:?}, {:?})",
        info.name,
        info.device_type,
        info.backend
    );
    Ok(best.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vulkan_is_preferred_over_dx12_for_equal_hardware() {
        // On a typical Windows machine wgpu lists the same GPU under both
        // backends; the launch must prefer Vulkan (see device_backend_score).
        let vulkan = device_backend_score(wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Vulkan);
        let dx12 = device_backend_score(wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Dx12);
        assert!(vulkan > dx12);
    }

    #[test]
    fn any_hardware_backend_beats_a_software_adapter() {
        let software = device_backend_score(wgpu::DeviceType::Cpu, wgpu::Backend::Dx12);
        assert!(device_backend_score(wgpu::DeviceType::Cpu, wgpu::Backend::Vulkan) > 0);
        assert!(software == 3); // WARP: backend score only
        assert!(device_backend_score(wgpu::DeviceType::VirtualGpu, wgpu::Backend::Gl) > software);
    }

    #[test]
    fn metal_ranks_with_vulkan_for_apple_silicon() {
        // Apple Silicon reports DeviceType::Other on the Metal backend.
        let metal = device_backend_score(wgpu::DeviceType::Other, wgpu::Backend::Metal);
        assert!(metal > device_backend_score(wgpu::DeviceType::Cpu, wgpu::Backend::Dx12));
    }
}
