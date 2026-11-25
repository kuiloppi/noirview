#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod animation;
mod app;
mod audio;
mod bar_layout;
mod clipboard;
// Catalog for large collections; the keyword cache stores its rows here.
#[allow(dead_code)]
mod catalog;
mod comfy;
mod decode_stats;
mod dialog_place;
mod disk;
mod folder;
mod gesture;
mod grid_layout;
mod grid_pages;
mod icons;
mod identity;
mod image_queue;
mod imaging;
mod input;
mod iptc;
mod jpeg_preview;
mod jpeg_scaled;
mod keyword_index;
mod magnifier;
mod memory;
mod metadata;
mod model;
mod operations;
mod pregen;
mod repaint;
mod resize;
// Compact lists for large folders; not wired in yet.
#[cfg(test)]
mod scale_bench;
#[allow(dead_code)]
mod scope;
mod sorting;
mod startup;
mod state;
mod storage;
mod sysinfo;
mod tags;
mod thumbnail_cache;
// Parts of the USN client are used on Windows only.
#[cfg_attr(not(windows), allow(dead_code))]
mod usn;
mod view;
mod watch;
mod xpkeywords;

fn main() -> eframe::Result {
    // `noirview --watch`: the background folder watcher, without a window.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--watch") {
        std::process::exit(watch::control::headless(&args));
    }
    let mut display = storage::read_display();
    let data = storage::data_dir();
    // Automatic on Windows is OpenGL: through wgpu (Vulkan, DirectX 12) the
    // window could show its frame off the client area after resizing (NVIDIA's
    // default present method). Unless OpenGL did not start before.
    let gl_unavailable = data.as_ref().map(|d| d.join("opengl-unavailable"));
    let mut auto_gl = cfg!(windows)
        && display.backend.is_empty()
        && !gl_unavailable.as_ref().is_some_and(|p| p.exists());
    // A graphics choice that did not get as far as the first frame last time:
    // start with the (next) automatic choice; the window says so.
    let trying = data.as_ref().map(|d| d.join("backend-trying"));
    let chosen = if auto_gl {
        "OpenGL (automatic)".to_owned()
    } else {
        display.backend.clone()
    };
    if !chosen.is_empty() {
        if let Some(t) = &trying {
            if t.exists() {
                let _ = std::fs::write(t.with_file_name("backend-failed"), chosen.as_bytes());
                let _ = std::fs::remove_file(t);
                if auto_gl || display.backend == "opengl" {
                    if let Some(u) = &gl_unavailable {
                        let _ = std::fs::write(u, b"");
                    }
                }
                auto_gl = false;
                display.backend.clear();
            } else {
                let _ = std::fs::write(t, chosen.as_bytes());
            }
        }
    }
    // A panic or a failure to start is written next to the library.
    if let Some(d) = data.clone() {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = std::fs::write(d.join("noirview-error.txt"), info.to_string());
            previous(info);
        }));
    }
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    if display.low_latency {
        // One queued frame: input shows up a frame sooner.
        wgpu_options.desired_maximum_frame_latency = Some(1);
    }
    // A chosen graphics backend (Settings → Display); otherwise wgpu picks.
    let backend = match display.backend.as_str() {
        "dx12" => Some(eframe::egui_wgpu::wgpu::Backends::DX12),
        "vulkan" => Some(eframe::egui_wgpu::wgpu::Backends::VULKAN),
        _ => None,
    };
    if let Some(backends) = backend {
        let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::default();
        setup.instance_descriptor.backends = backends;
        wgpu_options.wgpu_setup = eframe::egui_wgpu::WgpuSetup::CreateNew(setup);
    }
    // OpenGL draws through eframe's glow renderer instead of wgpu.
    let renderer = if auto_gl || display.backend == "opengl" {
        eframe::Renderer::Glow
    } else {
        eframe::Renderer::Wgpu
    };
    let options = eframe::NativeOptions {
        renderer,
        wgpu_options,
        persist_window: display.remember_window,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(concat!("NoirView ", env!("CARGO_PKG_VERSION")))
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([800.0, 540.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "NoirView",
        options,
        Box::new(|cc| Ok(Box::new(app::Viewer::new(cc)))),
    );
    if let (Err(e), Some(d)) = (&result, &data) {
        let _ = std::fs::write(d.join("noirview-error.txt"), format!("{e}"));
    }
    result
}
