# NoirView's changes to eframe 0.31.1

eframe 0.31.1 from crates.io (MIT OR Apache-2.0, see `LICENSE-MIT` and `LICENSE-APACHE`), changed as follows. To update eframe, copy the new version here and re-apply the changes (search for "NoirView patch").

## Surface size check (`src/native/wgpu_integration.rs`)

- `surface_sizes()`: the size each viewport's wgpu surface was last configured to.
- Before `paint_and_update_textures`, the window's `inner_size()` is compared with it; if they differ, `painter.on_window_resized` reconfigures the surface first. eframe does a similar check only on Linux (Wayland).

## Window diagnostics (`src/noirview_diag.rs`)

- `pub mod noirview_diag` in `lib.rs`: `window_diagnostics()` returns what the root window's events and paints showed (sizes, scale, cursor, and on Windows `GetClientRect` / `GetWindowRect` / `GetDpiForWindow`). `record_event` is called at the start of `on_window_event` and `record_paint` just before painting, in both `wgpu_integration.rs` and `glow_integration.rs`. `take_paint_stats()` reports paint times with and without texture uploads.
- `Cargo.toml`: windows-sys features `Win32_UI_HiDpi`, `Win32_UI_WindowsAndMessaging`.
