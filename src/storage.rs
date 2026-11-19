use crate::model::Library;
use anyhow::{bail, Context, Result};
use chrono::{Datelike, Local, Timelike};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Display options read before the window is created (`display.json` in
/// the data folder, written by the program when they change).
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    pub low_latency: bool,
    /// Graphics backend: "dx12", "vulkan", "opengl" or empty for automatic.
    pub backend: String,
    /// Restore the window's size and position from the last run (eframe's
    /// `app.ron`). Missing in older files: on.
    #[serde(default = "yes")]
    pub remember_window: bool,
}
fn yes() -> bool {
    true
}
/// The data folder (next to the library), for files read before the window
/// exists.
pub fn data_dir() -> Option<PathBuf> {
    Store::discover()
        .ok()
        .and_then(|s| s.path.parent().map(Path::to_path_buf))
}
pub fn write_display(data: &Path, display: &Display) -> Result<()> {
    fs::write(data.join("display.json"), serde_json::to_vec(display)?)?;
    Ok(())
}
pub fn read_display() -> Display {
    data_dir()
        .and_then(|d| fs::read(d.join("display.json")).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Display {
            remember_window: true,
            low_latency: true,
            ..Default::default()
        })
}

#[derive(Clone)]
pub struct Store {
    pub path: PathBuf,
    pub exe_dir: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct Backup {
    format: String,
    version: u32,
    created: String,
    library: Library,
}
impl Store {
    pub fn discover() -> Result<Self> {
        let exe_dir = std::env::current_exe()?
            .parent()
            .context("Executable has no parent")?
            .to_path_buf();
        let data = if exe_dir.join("portable.marker").exists() {
            exe_dir.join("data")
        } else {
            directories::ProjectDirs::from("org", "NoirView", "NoirView")
                .context("Cannot locate application data")?
                .data_local_dir()
                .to_path_buf()
        };
        fs::create_dir_all(&data)?;
        Ok(Self {
            path: data.join("library.json"),
            exe_dir,
        })
    }
    pub fn load(&self) -> Result<Library> {
        let database = crate::state::database_path(self);
        if database.exists() {
            return crate::state::load(self);
        }
        self.load_legacy()
    }
    pub(crate) fn load_legacy(&self) -> Result<Library> {
        if !self.path.exists() {
            return Ok(Library {
                schema: 1,
                ..Default::default()
            });
        }
        let library: Library = serde_json::from_slice(&fs::read(&self.path)?)
            .context("Cannot read library; the original file has been preserved")?;
        if library.schema > 1 {
            bail!("This library was created by a newer version of NoirView");
        }
        Ok(library)
    }
    pub fn save(&self, library: &Library) -> Result<()> {
        crate::state::replace(self, library)
    }
    pub fn backup(&self, library: &Library) -> Result<PathBuf> {
        let bytes = serde_json::to_vec_pretty(&Backup {
            format: "noirview-backup".into(),
            version: 1,
            created: Local::now().to_rfc3339(),
            library: library.clone(),
        })?;
        let name = format!("noirview-{}.json", Local::now().format("%Y%m%d-%H%M%S-%3f"));
        let primary = self.exe_dir.join("backups");
        fs::create_dir_all(&primary).context(
            "Cannot write backups beside the executable. Use a writable portable installation",
        )?;
        let path = primary.join(&name);
        atomic_write(&path, &bytes)?;
        prune(&primary, library.settings.backup_keep)?;
        if !library.settings.backup_extra.as_os_str().is_empty() {
            fs::create_dir_all(&library.settings.backup_extra)?;
            atomic_write(&library.settings.backup_extra.join(&name), &bytes)?;
            prune(&library.settings.backup_extra, library.settings.backup_keep)?;
        }
        Ok(path)
    }
    pub fn restore(&self, path: &Path, current: &Library) -> Result<Library> {
        let backup: Backup =
            serde_json::from_slice(&fs::read(path)?).context("Invalid NoirView backup")?;
        if backup.format != "noirview-backup" || backup.version != 1 || backup.library.schema > 1 {
            bail!("Unsupported backup format");
        }
        // A verified safety copy is written before changing any active state.
        let safety = self.path.with_file_name(format!(
            "before-restore-{}.json",
            Local::now().format("%Y%m%d-%H%M%S-%f")
        ));
        atomic_write(&safety, &serde_json::to_vec_pretty(current)?)?;
        self.save(&backup.library)?;
        Ok(backup.library)
    }
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing parent directory")?;
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
fn prune(folder: &Path, keep: usize) -> Result<()> {
    let mut backups: Vec<_> = fs::read_dir(folder)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.starts_with("noirview-") && s.ends_with(".json"))
                .unwrap_or(false)
        })
        .collect();
    backups.sort();
    let remove = backups.len().saturating_sub(keep.max(1));
    for path in backups.into_iter().take(remove) {
        fs::remove_file(path)?;
    }
    Ok(())
}
/// Latest due local-time slot, including slots missed while the app was closed.
pub fn due_slot(lib: &Library, now: chrono::DateTime<Local>) -> Option<String> {
    let s = &lib.settings;
    if !s.backup_schedule || !s.backup_days.iter().any(|v| *v) {
        return None;
    }
    for back in 0..8 {
        let day = now.date_naive() - chrono::Duration::days(back);
        if !s.backup_days[day.weekday().num_days_from_monday() as usize] {
            continue;
        }
        if back == 0 && (now.hour(), now.minute()) < (s.backup_hour, s.backup_minute) {
            continue;
        }
        let slot = format!("{}T{:02}:{:02}", day, s.backup_hour, s.backup_minute);
        return if lib
            .last_backup_slot
            .as_ref()
            .map(|s| s < &slot)
            .unwrap_or(true)
        {
            Some(slot)
        } else {
            None
        };
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_provenance_and_legacy_mixed_tags_survive_backup_restore() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store {
            path: dir.path().join("data/library.json"),
            exe_dir: dir.path().into(),
        };
        fs::create_dir_all(store.path.parent().unwrap()).unwrap();
        let mut lib = Library {
            schema: 1,
            ..Default::default()
        };
        lib.tags.insert("old mixed".into());
        let legacy_created = store.backup(&lib).unwrap();
        let legacy = dir.path().join("legacy-backup.json");
        fs::rename(legacy_created, &legacy).unwrap();
        lib.create_manual_tag("oma järvi".into());
        lib.settings.tags_max_width = Some(900.);
        lib.learn_tags(&std::collections::BTreeSet::from(["camera".into()]));
        lib.photos.entry("photo.jpg".into()).or_default().elite = true;
        let modern = store.backup(&lib).unwrap();
        let restored = store.restore(&modern, &Library::default()).unwrap();
        assert!(restored.is_manual_tag("oma järvi"));
        assert_eq!(restored.settings.tag_width(), 900.);
        assert!(!restored.is_manual_tag("camera"));
        assert!(!restored.is_manual_tag("old mixed"));
        assert_eq!(restored.tags, lib.tags);
        assert!(restored.photos[std::path::Path::new("photo.jpg")].elite);
        let restored = store.load().unwrap();
        assert!(restored.is_manual_tag("oma järvi"));
        // Old envelopes remain compatible; no origin is invented for the mixed palette.
        let legacy_restore = store.restore(&legacy, &lib).unwrap();
        assert_eq!(
            legacy_restore.tags,
            std::collections::BTreeSet::from(["old mixed".into()])
        );
        assert!(!legacy_restore.is_manual_tag("old mixed"));
        assert!(!store.load().unwrap().is_manual_tag("old mixed"));
    }
    #[test]
    fn backup_roundtrip_preserves_favorites_tags_and_settings() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store {
            path: dir.path().join("data/library.json"),
            exe_dir: dir.path().to_path_buf(),
        };
        let mut lib = Library {
            schema: 1,
            ..Default::default()
        };
        lib.tags.insert("Test & 日本語".into());
        lib.settings.hover = true;
        let path = store.backup(&lib).unwrap();
        let restored = store.restore(&path, &Library::default()).unwrap();
        assert_eq!(restored.tags, lib.tags);
        assert!(restored.settings.hover);
    }
    #[test]
    fn malformed_restore_never_replaces_library() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store {
            path: dir.path().join("library.json"),
            exe_dir: dir.path().into(),
        };
        store.save(&Library::default()).unwrap();
        let old = serde_json::to_value(store.load().unwrap()).unwrap();
        let bad = dir.path().join("bad.json");
        fs::write(&bad, b"not json").unwrap();
        assert!(store.restore(&bad, &Library::default()).is_err());
        assert_eq!(old, serde_json::to_value(store.load().unwrap()).unwrap());
    }
    #[test]
    fn missed_schedule_runs_once_after_restart() {
        use chrono::TimeZone;
        let now = Local.with_ymd_and_hms(2026, 11, 6, 21, 30, 0).unwrap();
        let mut lib = Library::default();
        lib.settings.backup_schedule = true;
        lib.settings.backup_hour = 20;
        lib.settings.backup_days = [false; 7];
        lib.settings.backup_days[4] = true;
        let slot = due_slot(&lib, now).unwrap();
        assert_eq!(slot, "2026-11-06T20:00");
        lib.last_backup_slot = Some(slot);
        assert!(due_slot(&lib, now).is_none());
        let next = now + chrono::Duration::days(10);
        assert!(due_slot(&lib, next).unwrap() > lib.last_backup_slot.unwrap());
    }
}
