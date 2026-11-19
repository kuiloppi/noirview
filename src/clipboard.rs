//! Files on the clipboard (Copy file): Explorer pastes them like files
//! copied there. Text and image copies go through egui.
use anyhow::Result;
use std::path::PathBuf;

#[cfg(windows)]
pub fn copy_files(paths: &[PathBuf]) -> Result<()> {
    use anyhow::{bail, ensure};
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows_sys::Win32::UI::Shell::DROPFILES;
    const CF_HDROP: u32 = 15;
    ensure!(!paths.is_empty(), "nothing to copy");
    // DROPFILES, then the paths as wide strings, each ended by a zero and the
    // list by another.
    let mut wide: Vec<u16> = vec![];
    for p in paths {
        let p = p.canonicalize().unwrap_or_else(|_| p.clone());
        let text = p.as_os_str().to_string_lossy();
        // canonicalize gives \\?\ paths; Explorer wants plain ones.
        let plain = text.strip_prefix(r"\\?\").unwrap_or(&text);
        wide.extend(std::ffi::OsStr::new(plain).encode_wide());
        wide.push(0);
    }
    wide.push(0);
    let header = std::mem::size_of::<DROPFILES>();
    let size = header + wide.len() * 2;
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, size);
        ensure!(!memory.is_null(), "out of memory");
        let base = GlobalLock(memory) as *mut u8;
        if base.is_null() {
            GlobalFree(memory);
            bail!("could not lock memory");
        }
        let files = base as *mut DROPFILES;
        std::ptr::write_bytes(base, 0, size);
        (*files).pFiles = header as u32;
        (*files).fWide = 1;
        std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, base.add(header), wide.len() * 2);
        GlobalUnlock(memory);
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            GlobalFree(memory);
            bail!("the clipboard is in use by another program");
        }
        EmptyClipboard();
        let set = SetClipboardData(CF_HDROP, memory as _);
        CloseClipboard();
        if set.is_null() {
            GlobalFree(memory);
            bail!("could not set the clipboard");
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn copy_files(_paths: &[PathBuf]) -> Result<()> {
    anyhow::bail!("copying files is available on Windows")
}
