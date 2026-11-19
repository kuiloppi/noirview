//! Conservative local-file identity and persistent mark ownership, never content matching.
use crate::model::{Library, PhotoMeta};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};

pub fn capture(path: &Path) -> Option<String> {
    // Do not assign ownership through file symlinks or multiply linked files.
    if fs::symlink_metadata(path).ok()?.file_type().is_symlink() {
        return None;
    }
    let file = fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    let created = match meta.created().ok()?.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_nanos().to_string(),
        Err(e) => format!("-{}", e.duration().as_nanos()),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return None;
        }
        Some(format!("unix:{}:{}:{created}", meta.dev(), meta.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FileIdInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
            BY_HANDLE_FILE_INFORMATION, FILE_ID_INFO,
        };
        let mut links: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
        // Handles belong to this scope and close before any subsequent rename.
        unsafe {
            if GetFileInformationByHandle(file.as_raw_handle(), &mut links) == 0
                || links.nNumberOfLinks != 1
            {
                return None;
            }
            if GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&mut info as *mut FILE_ID_INFO).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            ) == 0
            {
                return None;
            }
        }
        if info.VolumeSerialNumber == 0 || info.FileId.Identifier == [0; 16] {
            return None;
        }
        Some(format!(
            "windows:{}:{:032x}:{created}",
            info.VolumeSerialNumber,
            u128::from_le_bytes(info.FileId.Identifier)
        ))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = created;
        None
    }
}

#[cfg(windows)]
fn case_rename(a: &Path, b: &Path) -> bool {
    a.as_os_str().to_string_lossy().to_lowercase() == b.as_os_str().to_string_lossy().to_lowercase()
        && fs::canonicalize(a)
            .ok()
            .zip(fs::canonicalize(b).ok())
            .is_some_and(|(a, b)| a == b)
}
#[cfg(not(windows))]
fn case_rename(_a: &Path, _b: &Path) -> bool {
    false
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkRecord {
    pub path: PathBuf,
    pub favorite: bool,
    pub elite: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TrackedMarks {
    records: BTreeMap<String, MarkRecord>,
    #[serde(skip)]
    changed: BTreeSet<String>,
}
impl TrackedMarks {
    pub fn get(&self, id: &str) -> Option<&MarkRecord> {
        self.records.get(id)
    }
    pub fn iter(&self) -> impl Iterator<Item = (&String, &MarkRecord)> {
        self.records.iter()
    }
    pub fn set(&mut self, id: String, value: Option<MarkRecord>) {
        self.changed.insert(id.clone());
        if let Some(value) = value {
            self.records.insert(id, value);
        } else {
            self.records.remove(&id);
        }
    }
    pub fn clear_changes(&mut self) {
        self.changed.clear();
    }
    pub fn take_changes(&mut self) -> BTreeMap<String, Option<MarkRecord>> {
        std::mem::take(&mut self.changed)
            .into_iter()
            .map(|id| {
                let value = self.records.get(&id).cloned();
                (id, value)
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct Observation {
    pub identity: Option<String>,
}
#[derive(Debug, Default)]
pub struct Snapshot {
    pub paths: BTreeMap<PathBuf, Observation>,
    pub serial: u64,
}
impl Library {
    pub fn rebind_file(&mut self, path: &Path) {
        if let Some(old) = self.photos.get(path).and_then(|m| m.identity.clone()) {
            if self.tracked_marks.get(&old).is_some_and(|m| m.path == path) {
                self.tracked_marks.set(old, None);
            }
        }
        if self
            .photos
            .get(path)
            .is_some_and(|m| m.favorite || m.elite || m.identity.is_some())
        {
            let meta = self.photos.entry(path.into()).or_default();
            meta.rebound_from = Some(meta.identity.take().unwrap_or_default());
        }
    }
    pub fn sync_marks(&mut self, snapshot: Option<&Snapshot>) {
        for (path, id) in std::mem::take(&mut self.photos.removed_marks) {
            if self.tracked_marks.get(&id).is_some_and(|m| m.path == path) {
                self.tracked_marks.set(id, None);
            }
        }
        for path in std::mem::take(&mut self.photos.marks_dirty) {
            let Some(meta) = self.photos.get(&path) else {
                continue;
            };
            if meta.rebound_from.is_some() {
                continue;
            }
            if meta.identity.is_none() && (meta.favorite || meta.elite) {
                if let Some(id) = snapshot
                    .and_then(|s| s.paths.get(&path))
                    .and_then(|o| o.identity.clone())
                {
                    self.photos.entry(path.clone()).or_default().identity = Some(id);
                    self.photos.marks_dirty.remove(&path);
                }
            }
            let meta = &self.photos[&path];
            let Some(id) = &meta.identity else {
                continue;
            };
            if self.tracked_marks.get(id).is_some_and(|m| m.path != path) {
                continue;
            }
            let record = (meta.favorite || meta.elite).then(|| MarkRecord {
                path: path.clone(),
                favorite: meta.favorite,
                elite: meta.elite,
            });
            self.tracked_marks.set(id.clone(), record);
        }
    }
}

#[derive(Debug)]
pub struct Plan {
    pub guard: BTreeMap<PathBuf, Option<PhotoMeta>>,
    pub photos: BTreeMap<PathBuf, (Option<PhotoMeta>, Option<PhotoMeta>)>,
    pub marks: BTreeMap<String, (Option<MarkRecord>, Option<MarkRecord>)>,
    pub snapshot: Arc<Snapshot>,
    pub relocated: usize,
}

// Called on the state worker after the latest UI delta has committed. Filesystem
// probes and the full library clone never run on the UI thread.
pub fn plan(original: &Library, snapshot: Arc<Snapshot>) -> Plan {
    let mut next = original.clone();
    next.photos.clear_changes();
    next.tracked_marks.clear_changes();
    let mut found: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for (path, o) in &snapshot.paths {
        if let Some(id) = &o.identity {
            found.entry(id.clone()).or_default().push(path.clone());
        }
    }
    let mut owners: BTreeMap<String, usize> = BTreeMap::new();
    for meta in original.photos.values().filter(|m| m.favorite || m.elite) {
        if let Some(id) = &meta.identity {
            *owners.entry(id.clone()).or_default() += 1;
        }
    }
    // Bind legacy path marks only while that exact path can actually be observed.
    for (path, meta) in original.photos.iter() {
        if meta.identity.is_some() || !(meta.favorite || meta.elite) {
            continue;
        }
        let Some(id) = snapshot.paths.get(path).and_then(|o| o.identity.as_ref()) else {
            continue;
        };
        if found[id].len() != 1
            || next.tracked_marks.get(id).is_some()
            || capture(path).as_ref() != Some(id)
        {
            continue;
        }
        next.photos.entry(path.clone()).or_default().identity = Some(id.clone());
        next.tracked_marks.set(
            id.clone(),
            Some(MarkRecord {
                path: path.clone(),
                favorite: meta.favorite,
                elite: meta.elite,
            }),
        );
    }
    let mut relocated = 0;
    let mut guard = BTreeMap::new();
    let records = next
        .tracked_marks
        .iter()
        .map(|(id, m)| (id.clone(), m.clone()))
        .collect::<Vec<_>>();
    for (id, mut record) in records {
        let Some(paths) = found.get(&id).filter(|p| p.len() == 1) else {
            continue;
        };
        if owners.get(&id).copied().unwrap_or(0) > 1 {
            continue;
        }
        let target = &paths[0];
        guard.insert(
            record.path.clone(),
            original.photos.get(&record.path).cloned(),
        );
        guard.insert(target.clone(), original.photos.get(target).cloned());
        if capture(target).as_ref() != Some(&id) {
            continue;
        }
        if target != &record.path {
            // Permission errors and unsupported identity are not proof of absence.
            let mut source = match fs::symlink_metadata(&record.path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(None),
                Ok(_) => capture(&record.path).map(Some),
                Err(_) => None,
            };
            if source.as_ref().is_some_and(|s| s.as_ref() == Some(&id))
                && case_rename(&record.path, target)
            {
                source = Some(None);
            }
            if source.is_none() || source.as_ref().is_some_and(|s| s.as_ref() == Some(&id)) {
                continue;
            }
            if let Some(destination) = next.photos.get(target) {
                if (destination.favorite || destination.elite)
                    && destination.identity.as_ref() != Some(&id)
                {
                    continue;
                }
            }
            // Retire the old projection, retaining durable identity ownership.
            if next
                .photos
                .get(&record.path)
                .is_some_and(|m| m.identity.as_ref() == Some(&id))
            {
                if let Some(Some(other)) = source {
                    let meta = next.photos.entry(record.path.clone()).or_default();
                    meta.favorite = false;
                    meta.elite = false;
                    meta.identity = Some(other);
                } else {
                    next.photos.remove(&record.path);
                }
            }
            record.path = target.clone();
            next.tracked_marks.set(id.clone(), Some(record.clone()));
            relocated += 1;
        }
        let meta = next.photos.entry(target.clone()).or_default();
        meta.identity = Some(id);
        meta.favorite = record.favorite;
        meta.elite = record.elite;
    }
    // A reused path must not display the previous object's marks. Its identity
    // record stays archived even when the original object is outside this scan.
    for (path, o) in &snapshot.paths {
        let Some(meta) = next.photos.get(path) else {
            continue;
        };
        if meta.identity.is_some() && meta.identity != o.identity && capture(path) == o.identity {
            let old = meta.identity.clone().unwrap();
            // Force an ownership delta even when the archived record is unchanged:
            // retiring a reused/unsupported path must not delete its old owner.
            if let Some(record) = next.tracked_marks.get(&old).cloned() {
                next.tracked_marks.set(old, Some(record));
            }
            let own = o
                .identity
                .as_ref()
                .and_then(|id| next.tracked_marks.get(id))
                .filter(|m| m.path == *path)
                .cloned();
            let meta = next.photos.entry(path.clone()).or_default();
            meta.favorite = own.as_ref().is_some_and(|m| m.favorite);
            meta.elite = own.as_ref().is_some_and(|m| m.elite);
            meta.identity = o.identity.clone();
        }
    }
    let photos: BTreeMap<PathBuf, (Option<PhotoMeta>, Option<PhotoMeta>)> = next
        .photos
        .take_changes()
        .into_iter()
        .filter_map(|(p, m)| {
            let old = original.photos.get(&p).cloned();
            (old != m).then_some((p, (old, m)))
        })
        .collect();
    let marks = next
        .tracked_marks
        .take_changes()
        .into_iter()
        .map(|(id, m)| {
            let old = original.tracked_marks.get(&id).cloned();
            (id, (old, m))
        })
        .collect();
    for (path, (expected, _)) in &photos {
        guard.insert(path.clone(), expected.clone());
    }
    Plan {
        guard,
        photos,
        marks,
        snapshot,
        relocated,
    }
}

impl Plan {
    pub fn apply(self, lib: &mut Library) -> Result<usize, ()> {
        for (path, expected) in &self.guard {
            let actual = lib.photos.get(path);
            let matches = match (actual, expected.as_ref()) {
                (None, None) => true,
                (Some(a), Some(e)) => {
                    a.rebound_from.is_none()
                        && a.tags == e.tags
                        && a.favorite == e.favorite
                        && a.elite == e.elite
                        && (a.identity.is_none() || a.identity == e.identity)
                }
                _ => false,
            };
            if !matches {
                return Err(());
            }
        }
        for (id, (expected, _)) in &self.marks {
            if lib.tracked_marks.get(id).is_some() && lib.tracked_marks.get(id) != expected.as_ref()
            {
                return Err(());
            }
        }
        for (path, (_, value)) in self.photos {
            if let Some(meta) = value {
                lib.photos.insert(path, meta);
            } else {
                lib.photos.remove(&path);
            }
        }
        for (id, (_, value)) in self.marks {
            lib.tracked_marks.set(id, value);
        }
        Ok(self.relocated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(path: &Path) {
        fs::write(path, b"independent file").unwrap();
    }
    fn snapshot(paths: &[&Path]) -> Arc<Snapshot> {
        Arc::new(Snapshot {
            paths: paths
                .iter()
                .map(|p| {
                    (
                        p.to_path_buf(),
                        Observation {
                            identity: capture(p),
                        },
                    )
                })
                .collect(),
            serial: 0,
        })
    }
    fn marked(path: &Path) -> Library {
        let id =
            capture(path).expect("Test filesystem must expose stable identity + creation time");
        let mut lib = Library::default();
        lib.photos.insert(
            path.into(),
            PhotoMeta {
                favorite: true,
                elite: true,
                identity: Some(id),
                ..Default::default()
            },
        );
        lib.sync_marks(None);
        lib.photos.clear_changes();
        lib.tracked_marks.clear_changes();
        lib
    }
    #[test]
    fn identity_survives_rename_but_not_copy_or_replacement_and_releases_handles() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jpg");
        let b = dir.path().join("b.jpg");
        let c = dir.path().join("c.jpg");
        file(&a);
        let id = capture(&a).unwrap();
        fs::rename(&a, &b).unwrap();
        assert_eq!(capture(&b), Some(id.clone()));
        fs::copy(&b, &c).unwrap();
        assert_ne!(capture(&c), Some(id));
        fs::remove_file(&b).unwrap();
        assert!(capture(&b).is_none());
    }
    #[test]
    fn case_only_rename_updates_the_canonical_mark_path() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("MixedÉ.JPG");
        let b = dir.path().join("mixedé.jpg");
        file(&a);
        let mut lib = marked(&a);
        fs::rename(&a, &b).unwrap();
        let p = plan(&lib, snapshot(&[&b]));
        assert_eq!(p.relocated, 1);
        p.apply(&mut lib).unwrap();
        lib.sync_marks(None);
        assert!(lib.photos[&b].elite);
        assert!(!lib.photos.contains_key(&a));
    }
    #[test]
    fn rename_move_projection_and_archive_reuse_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jpg");
        let other = dir.path().join("other");
        fs::create_dir(&other).unwrap();
        let b = other.join("renamed.jpg");
        file(&a);
        let mut lib = marked(&a);
        let id = capture(&a).unwrap();
        fs::rename(&a, &b).unwrap();
        let p = plan(&lib, snapshot(&[&b]));
        assert_eq!(p.relocated, 1);
        p.apply(&mut lib).unwrap();
        lib.sync_marks(None);
        assert!(!lib.photos.contains_key(&a));
        assert!(lib.photos[&b].elite);
        assert_eq!(lib.tracked_marks.get(&id).unwrap().path, b);
        assert_eq!(plan(&lib, snapshot(&[&b])).relocated, 0);
    }
    #[test]
    fn copy_and_same_name_different_object_never_receive_original_marks() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jpg");
        let b = dir.path().join("copy.jpg");
        let hidden = dir.path().join("hidden.jpg");
        file(&a);
        let mut lib = marked(&a);
        let id = capture(&a).unwrap();
        fs::copy(&a, &b).unwrap();
        plan(&lib, snapshot(&[&a, &b])).apply(&mut lib).unwrap();
        assert!(!lib.photos.get(&b).is_some_and(|m| m.favorite));
        fs::rename(&a, &hidden).unwrap();
        fs::copy(&b, &a).unwrap();
        plan(&lib, snapshot(&[&a, &b])).apply(&mut lib).unwrap();
        lib.sync_marks(None);
        assert!(!lib.photos[&a].favorite);
        assert!(lib.tracked_marks.get(&id).unwrap().elite);
        plan(&lib, snapshot(&[&hidden])).apply(&mut lib).unwrap();
        lib.sync_marks(None);
        assert!(lib.photos[&hidden].elite);
        assert!(!lib.photos[&a].favorite);
    }
    #[test]
    fn reused_path_and_original_found_together_keep_distinct_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jpg");
        let b = dir.path().join("b.jpg");
        file(&a);
        let mut lib = marked(&a);
        fs::rename(&a, &b).unwrap();
        file(&a);
        plan(&lib, snapshot(&[&a, &b])).apply(&mut lib).unwrap();
        lib.sync_marks(None);
        assert!(!lib.photos[&a].favorite);
        assert!(lib.photos[&b].favorite);
        assert!(lib.photos[&b].elite);
    }
    #[test]
    fn hardlinks_and_conflicting_legacy_marks_remain_unresolved() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jpg");
        let b = dir.path().join("b.jpg");
        file(&a);
        let mut lib = marked(&a);
        let id = capture(&a).unwrap();
        fs::hard_link(&a, &b).unwrap();
        assert!(capture(&a).is_none());
        assert!(capture(&b).is_none());
        plan(&lib, snapshot(&[&a, &b])).apply(&mut lib).unwrap();
        lib.sync_marks(None);
        assert!(!lib.photos.get(&b).is_some_and(|m| m.elite));
        assert!(lib.tracked_marks.get(&id).is_some());
        fs::remove_file(&b).unwrap();
        let mut duplicate = snapshot(&[&a]);
        Arc::get_mut(&mut duplicate)
            .unwrap()
            .paths
            .insert(b.clone(), Observation { identity: Some(id) });
        assert_eq!(plan(&lib, duplicate).relocated, 0);
    }
    #[test]
    fn stale_plan_does_not_overwrite_new_mark_or_tag_edits() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jpg");
        let b = dir.path().join("b.jpg");
        file(&a);
        let mut lib = marked(&a);
        fs::rename(&a, &b).unwrap();
        let p = plan(&lib, snapshot(&[&b]));
        lib.photos.entry(a.clone()).or_default().favorite = false;
        assert!(p.apply(&mut lib).is_err());
        assert!(!lib.photos.contains_key(&b));
        let p = plan(&lib, snapshot(&[&b]));
        lib.photos
            .entry(a.clone())
            .or_default()
            .tags
            .insert("new edit".into());
        assert!(p.apply(&mut lib).is_err());
    }
    #[test]
    fn archives_survive_json_backup_and_small_header_excludes_identity_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jpg");
        file(&a);
        let lib = marked(&a);
        let bytes = serde_json::to_vec(&lib).unwrap();
        let restored: Library = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.tracked_marks.iter().count(), 1);
        let header = lib.header();
        assert_eq!(header.tracked_marks.iter().count(), 0);
        assert!(header.photos.is_empty());
        let legacy: Library = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.tracked_marks.iter().count(), 0);
    }
}
