# NoirView

> **⚠ Early development — use at your own risk.**
> NoirView is a work in progress. It writes keywords into image files, moves, renames and deletes files, and can run an optional background service for folder watching. A bug could damage, corrupt or lose images, or have other unexpected effects on your system. Keep backups of your images before using it, try it first on copies, and do not rely on it for files you cannot replace. The software is provided "as is", without warranty of any kind (see `LICENSE`).

A quiet, dark image viewer for Windows 10/11 (64-bit), written in Rust with egui/eframe. Images are decoded, scaled and cached off the UI thread; the window shows only the image until you ask for more.

## Features

- **Viewing:** fit, fill or shrink-only; full-window zoom and a magnifying glass on hold; animated GIFs; fullscreen with an optional image-only mode; keep the previous image on screen while the next one loads.
- **Browsing:** in order, random or shuffle (with or without history), slideshow, mouse wheel and edge strips, go to an image or page by number, a notice when every image has been shown, recently viewed images, favorite folders.
- **Thumbnail grid:** page sizes and size buttons, page order, wrap-around pages, hover preview, a big image view that can be browsed, selection for deleting, tagging and moving.
- **Tags:** IPTC, XMP and Windows XP keywords read and written losslessly (sidecars supported), Favorite / Elite marks, a tag palette, filters (all / any / exclude), a keyword index in SQLite.
- **Files:** move to the Recycle Bin with confirmation and *Restore deleted* (Ctrl+Z); verified moves to a destination folder with sequence or original names and a recovery journal; copy file / image; prompt and seed from ComfyUI and Automatic1111 images.
- **Bars:** a floating toolbar, the grid's icon bar or text buttons and the big image's bar; every button can be reordered, hidden or put in a group behind ».
- **Folder watching:** changes in the open and watched folders are picked up; on NTFS an optional service reads the USN journal (`noirview-usn.exe`).
- **Rendering:** OpenGL by default on Windows; Vulkan or DirectX 12 (wgpu) can be chosen. Optional AI upscaling through ONNX Runtime with DirectML (bring your own model).
- **Portable:** settings and caches live in a `data` folder next to the program.

## Build

Requirements: Rust (stable; the MSVC toolchain on Windows) and Python 3 for the license notices. On Windows:

```powershell
./build-windows.ps1                 # fmt, Clippy, tests, release build and the package
./build-windows.ps1 -IncludeAiRuntime  # also ONNX Runtime + DirectML from NuGet
```

The package (`dist/NoirView-<version>-windows-x64.zip`) contains `NoirView.exe`, `noirview-usn.exe`, `README.md`, `LICENSE` and `THIRD-PARTY-NOTICES.txt` (written by `tools/third_party_notices.py`).

Development checks on any platform:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

`vendor/eframe` is eframe 0.31.1 with small changes, described in `vendor/eframe/NOIRVIEW_PATCH.md`.

## License

MIT (see `LICENSE`). Third-party licenses: `THIRD-PARTY-NOTICES.txt` in the package, `vendor/eframe/LICENSE-*`.
