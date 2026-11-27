//! Who runs the watcher, and how the open program and the background process
//! (`noirview --watch`) hand it over. Everything lives in one per-user control
//! folder, shared by all NoirView installations (portable copies included),
//! so a new version can stop an old one's background process before its
//! folder is deleted.
//!
//! - `engine.lock`: held by the one owner (the open program, or the process).
//! - `stop`: a request (with its time) that the background process exits.
//! - `config.json`: what the background process watches, written by the program.
//! - `status.json`: the owner's progress, for Settings.
//! - `watcher.log`: start/stop, checks and errors (bounded).
use super::{source::now_secs, Handle, Setup, Status};
use crate::{model::Settings, model::WatchedFolder, thumbnail_cache::ThumbnailCache};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

/// A stop request older than this is ignored (its writer went away).
const STOP_VALID_SECS: u64 = 60;
const LOG_LIMIT: u64 = 1 << 20;
/// Windows sign-in: wait before starting so the desktop settles first.
pub const SIGN_IN_DELAY_SECS: u64 = 60;
#[cfg_attr(not(windows), allow(dead_code))]
pub const RUN_VALUE: &str = "NoirView folder watcher";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub version: String,
    pub exe: PathBuf,
    pub folders: Vec<WatchedFolder>,
    pub settings: Settings,
    pub cache_root: Option<PathBuf>,
    pub keyword_db: Option<PathBuf>,
    pub state_db: PathBuf,
    /// Run while NoirView is closed.
    pub background: bool,
}

pub fn dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("NOIRVIEW_WATCH_CONTROL") {
        return Some(dir.into());
    }
    directories::ProjectDirs::from("org", "NoirView", "NoirView")
        .map(|d| d.data_local_dir().join("watcher"))
}

/// The owner lock, if free.
pub fn try_own(dir: &Path) -> Option<File> {
    fs::create_dir_all(dir).ok()?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("engine.lock"))
        .ok()?;
    fs2::FileExt::try_lock_exclusive(&file).ok()?;
    Some(file)
}
pub fn request_stop(dir: &Path) {
    let _ = fs::create_dir_all(dir);
    let _ = fs::write(dir.join("stop"), now_secs().to_string());
}
pub fn clear_stop(dir: &Path) {
    let _ = fs::remove_file(dir.join("stop"));
}
pub fn stop_requested(dir: &Path) -> bool {
    fs::read_to_string(dir.join("stop"))
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok())
        .is_some_and(|at| now_secs().saturating_sub(at) <= STOP_VALID_SECS)
}
/// The open program takes the watcher over: asks a background process to
/// stop and waits for the lock. None when another open NoirView keeps it.
pub fn take_over(dir: &Path, timeout: Duration) -> Option<File> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(lock) = try_own(dir) {
            clear_stop(dir);
            return Some(lock);
        }
        if Instant::now() >= deadline {
            clear_stop(dir);
            return None;
        }
        request_stop(dir);
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}
pub fn write_config(dir: &Path, config: &Config) -> Result<()> {
    fs::create_dir_all(dir)?;
    write_atomic(
        &dir.join("config.json"),
        &serde_json::to_vec_pretty(config)?,
    )
}
pub fn read_config(dir: &Path) -> Result<Config> {
    let bytes = fs::read(dir.join("config.json")).context("No watcher configuration")?;
    serde_json::from_slice(&bytes).context("Unreadable watcher configuration")
}
pub fn write_status(dir: &Path, status: &Status) {
    if let Ok(bytes) = serde_json::to_vec_pretty(status) {
        let _ = write_atomic(&dir.join("status.json"), &bytes);
    }
}
/// The background process's last status, if it is recent.
pub fn read_status(dir: &Path) -> Option<Status> {
    let status: Status = serde_json::from_slice(&fs::read(dir.join("status.json")).ok()?).ok()?;
    Some(status)
}
pub fn log(dir: &Path, line: &str) {
    let path = dir.join("watcher.log");
    if fs::metadata(&path).is_ok_and(|m| m.len() > LOG_LIMIT) {
        let _ = fs::rename(&path, dir.join("watcher.old.log"));
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(
            file,
            "{} [{}] {line}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            std::process::id()
        );
    }
}

/// The Run-key command for an executable.
pub fn autostart_command(exe: &Path) -> String {
    format!("\"{}\" --watch --delay {SIGN_IN_DELAY_SECS}", exe.display())
}
/// The executable a Run-key command starts.
pub fn command_exe(command: &str) -> Option<PathBuf> {
    let rest = command.strip_prefix('"')?;
    Some(PathBuf::from(&rest[..rest.find('"')?]))
}
#[cfg(windows)]
mod registry {
    use super::RUN_VALUE;
    use anyhow::{bail, Result};
    use windows_sys::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
    };
    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    pub fn read() -> Option<String> {
        let (key, value) = (wide(RUN), wide(RUN_VALUE));
        let mut size = 0u32;
        // SAFETY: valid NUL-terminated strings; the first call only sizes.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if status != 0 || size == 0 {
            return None;
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
        // SAFETY: the buffer holds `size` bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != 0 {
            return None;
        }
        let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..end]))
    }
    pub fn write(command: Option<&str>) -> Result<()> {
        let (key, value) = (wide(RUN), wide(RUN_VALUE));
        // SAFETY: valid NUL-terminated strings and a data length in bytes.
        let status = unsafe {
            match command {
                Some(command) => {
                    let data = wide(command);
                    RegSetKeyValueW(
                        HKEY_CURRENT_USER,
                        key.as_ptr(),
                        value.as_ptr(),
                        REG_SZ,
                        data.as_ptr().cast(),
                        (data.len() * 2) as u32,
                    )
                }
                None => RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()),
            }
        };
        // 2 = ERROR_FILE_NOT_FOUND: nothing to delete.
        if status != 0 && !(command.is_none() && status == 2) {
            bail!("Windows sign-in setting could not be changed (error {status})");
        }
        Ok(())
    }
}
/// The current sign-in command (Windows only).
pub fn autostart() -> Option<String> {
    #[cfg(windows)]
    {
        registry::read()
    }
    #[cfg(not(windows))]
    {
        None
    }
}
fn set_autostart(command: Option<&str>) -> Result<()> {
    #[cfg(windows)]
    {
        registry::write(command)
    }
    #[cfg(not(windows))]
    {
        let _ = command;
        Ok(())
    }
}
/// Registers this executable for sign-in when enabled. When disabled, removes
/// an entry that points here or to an executable that no longer exists (an
/// older, deleted copy) and leaves another installation's entry alone.
pub fn sync_autostart(exe: &Path, enabled: bool) -> Result<()> {
    let current = autostart();
    if enabled {
        let wanted = autostart_command(exe);
        if current.as_deref() != Some(wanted.as_str()) {
            set_autostart(Some(&wanted))?;
        }
    } else if let Some(command) = current {
        let target = command_exe(&command);
        if target.as_ref().is_none_or(|t| t == exe || !t.exists()) {
            set_autostart(None)?;
        }
    }
    Ok(())
}

/// Starts the background process now (when NoirView closes).
pub fn spawn_background(exe: &Path) -> Result<()> {
    let mut command = std::process::Command::new(exe);
    command
        .arg("--watch")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    command
        .spawn()
        .context("Cannot start the background watcher")?;
    Ok(())
}

/// `noirview --watch [--delay SECONDS]`: the background process.
pub fn headless(args: &[String]) -> i32 {
    let delay = args
        .iter()
        .position(|a| a == "--delay")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let Some(dir) = dir() else {
        return 1;
    };
    if fs::create_dir_all(&dir).is_err() {
        return 1;
    }
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(delay) {
        if stop_requested(&dir) {
            return 0;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    if stop_requested(&dir) {
        return 0;
    }
    let Some(_lock) = try_own(&dir) else {
        log(&dir, "background: another owner is watching; exiting");
        return 0;
    };
    let config = match read_config(&dir) {
        Ok(c) => c,
        Err(e) => {
            log(&dir, &format!("background: {e:#}"));
            return 1;
        }
    };
    if !config.background || !config.settings.watch_folders || config.folders.is_empty() {
        log(&dir, "background: watching is off; exiting");
        return 0;
    }
    // A crash must leave a trace for the user to report.
    {
        let dir = dir.clone();
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log(&dir, &format!("background: unexpected error: {info}"));
            default(info);
        }));
    }
    crate::sysinfo::enter_background_mode();
    let cache = Arc::new(ThumbnailCache::new(
        config.cache_root.clone(),
        config.settings.thumbnail_cache_mb,
    ));
    // First views of new images too, when they are prepared for all images.
    let proxies = (config.settings.first_views_for_all
        && config.settings.loading_mode == crate::model::LoadingMode::Aggressive)
        .then(|| config.cache_root.as_deref().map(crate::imaging::proxy_root))
        .flatten()
        .map(|root| {
            let proxies = ThumbnailCache::new(Some(root), config.settings.proxy_budget_mb());
            proxies.keep_free(config.settings.proxy_keep_free());
            Arc::new(proxies)
        });
    let stop_dir = dir.clone();
    let setup = Setup {
        folders: config.folders.clone(),
        settings: config.settings.clone(),
        cache,
        proxies,
        keyword_db: config.keyword_db.clone(),
        state_db: config.state_db.clone(),
        owner: "background",
        gate: Arc::new(|| true),
        stop_requested: Arc::new(move || stop_requested(&stop_dir)),
    };
    let (source, warnings) = super::source_for(&config.folders, &config.settings);
    if super::journal_wanted(&config.settings) {
        log(&dir, "background: using the change journal (USN service)");
    }
    let handle = match Handle::start(setup, source) {
        Ok(h) => h,
        Err(e) => {
            log(&dir, &format!("background: cannot start: {e:#}"));
            return 1;
        }
    };
    log(
        &dir,
        &format!(
            "background {} started: {} folder(s)",
            config.version,
            config.folders.len()
        ),
    );
    for w in warnings {
        log(&dir, &w);
    }
    let mut last_check = 0;
    let mut last_batch = None;
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let status = handle.status();
        if status.last_check_at != last_check {
            last_check = status.last_check_at;
            if let Some(c) = status.last_check {
                log(
                    &dir,
                    &format!(
                        "check{}: {} folder(s), {} changed, {} ms",
                        if c.deep { " (full)" } else { "" },
                        c.folders,
                        c.changed,
                        c.ms
                    ),
                );
            }
        }
        if status.last_batch != last_batch {
            last_batch = status.last_batch;
            if let Some(b) = last_batch {
                log(&dir, &format!("batch: {}", b.describe()));
            }
        }
        write_status(&dir, &status);
        if handle.finished() || stop_requested(&dir) {
            break;
        }
    }
    let status = handle.status();
    handle.stop();
    let mut status_final = status;
    status_final.state = "stopped".into();
    status_final.updated = now_secs();
    write_status(&dir, &status_final);
    log(
        &dir,
        &format!(
            "background stopped: thumbnails made {}, keywords read {}",
            status_final.thumbnails, status_final.keywords
        ),
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_owner_at_a_time_and_a_stop_request_hands_over() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let first = try_own(dir).expect("free");
        assert!(try_own(dir).is_none(), "second owner");
        assert!(!stop_requested(dir));
        // The owner releases on request; take_over then gets the lock.
        let releaser = {
            let dir = dir.to_path_buf();
            std::thread::spawn(move || {
                while !stop_requested(&dir) {
                    std::thread::sleep(Duration::from_millis(10));
                }
                drop(first);
            })
        };
        let lock = take_over(dir, Duration::from_secs(5)).expect("handed over");
        releaser.join().unwrap();
        assert!(!stop_requested(dir), "request cleared after the hand-over");
        // A held lock and no response: gives up without leaving a request.
        assert!(take_over(dir, Duration::from_millis(300)).is_none());
        assert!(!stop_requested(dir));
        drop(lock);
        // Old requests are ignored.
        fs::write(
            dir.join("stop"),
            (now_secs() - STOP_VALID_SECS - 5).to_string(),
        )
        .unwrap();
        assert!(!stop_requested(dir));
    }

    #[test]
    fn sign_in_command_names_the_executable() {
        let exe = Path::new(r"C:\Apps\NoirView 0.1.38\noirview.exe");
        let command = autostart_command(exe);
        assert_eq!(command_exe(&command).as_deref(), Some(exe));
        assert!(command.ends_with("--watch --delay 60"));
        assert_eq!(command_exe("noirview.exe --watch"), None);
    }

    #[cfg(windows)]
    #[test]
    fn sign_in_entry_is_written_and_removed() {
        let saved = autostart();
        let exe = std::env::current_exe().unwrap();
        sync_autostart(&exe, true).unwrap();
        assert_eq!(autostart(), Some(autostart_command(&exe)));
        sync_autostart(&exe, false).unwrap();
        assert_eq!(autostart(), None);
        // A stale entry of a deleted copy is cleaned up too.
        set_autostart(Some(&autostart_command(Path::new(r"C:\gone\noirview.exe")))).unwrap();
        sync_autostart(&exe, false).unwrap();
        assert_eq!(autostart(), None);
        if let Some(saved) = saved {
            set_autostart(Some(&saved)).unwrap();
        }
    }
}
