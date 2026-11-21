use anyhow::{bail, ensure, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MoveItem {
    pub source: PathBuf,
    pub target: PathBuf,
    pub done: bool,
}
#[derive(Clone, Copy, Debug)]
pub enum Conflict {
    Cancel,
    Skip,
    Rename,
}
#[derive(Default, Debug)]
pub struct MoveReport {
    pub moved: Vec<(PathBuf, PathBuf)>,
    pub skipped: usize,
    pub error: Option<String>,
    pub conflict: Vec<MoveItem>,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    version: u32,
    items: Vec<MoveItem>,
}
#[cfg(test)]
pub fn plan(files: &[PathBuf], dest: &Path) -> Result<Vec<MoveItem>> {
    plan_with(files, dest, false, 8)
}
/// A sequence name's number: a stem of digits only, at least `digits` long.
fn sequence_number(path: &Path, digits: usize) -> Option<u64> {
    let stem = path.file_stem()?.to_str()?;
    (stem.len() >= digits.max(1) && stem.bytes().all(|b| b.is_ascii_digit()))
        .then(|| stem.parse().ok())
        .flatten()
}
/// Targets for moving `files` into `dest`: sequence names of at least
/// `digits` digits continuing after the highest one already there (001.jpg,
/// 002.jpg, …; past 999, 1000), or the original file names (`keep_names`; a
/// name used twice in the batch gets " (2)", " (3)", …).
#[cfg(test)]
pub fn plan_with(
    files: &[PathBuf],
    dest: &Path,
    keep_names: bool,
    digits: usize,
) -> Result<Vec<MoveItem>> {
    plan_names(files, dest, keep_names, digits, false)
}
/// `plan_with`; `fill_gaps`: sequence names take the free numbers first
/// (1.png and 3.png there: 2.png, then 4.png), not only those after the
/// highest.
pub fn plan_names(
    files: &[PathBuf],
    dest: &Path,
    keep_names: bool,
    digits: usize,
    fill_gaps: bool,
) -> Result<Vec<MoveItem>> {
    let digits = digits.clamp(1, 8);
    ensure!(
        !dest.as_os_str().is_empty(),
        "Choose a destination folder (Browse…)"
    );
    ensure!(dest.is_dir(), "The destination must be an existing folder");
    let dest = dest.canonicalize()?;
    let mut max = 0u64;
    let mut taken = std::collections::HashSet::new();
    for e in fs::read_dir(&dest)? {
        let e = e?;
        if !e.file_type()?.is_file() {
            continue;
        }
        if let Some(n) = sequence_number(&e.path(), digits) {
            max = max.max(n);
            taken.insert(n);
        }
    }
    // The numbers to use, in order: with `fill_gaps` the free ones first, then those
    // after the highest.
    let mut next = 0u64;
    let mut number = || -> Result<u64> {
        loop {
            next = next.checked_add(1).context("Sequence overflow")?;
            if !fill_gaps && next <= max {
                next = max;
                continue;
            }
            if taken.insert(next) {
                return Ok(next);
            }
        }
    };
    let mut items = vec![];
    let mut used = std::collections::HashSet::new();
    for source in files {
        ensure!(
            source
                .parent()
                .context("Missing source parent")?
                .canonicalize()?
                != dest,
            "Source and destination folders must differ"
        );
        let ext = source
            .extension()
            .context("Missing image extension")?
            .to_string_lossy();
        let target = if keep_names {
            let name = source.file_name().context("Missing file name")?;
            let mut target = dest.join(name);
            while !used.insert(target.file_name().unwrap_or_default().to_ascii_lowercase()) {
                target = suffixed(&target, 2)?;
            }
            target
        } else {
            let n = number()?;
            dest.join(format!("{n:0digits$}.{ext}"))
        };
        items.push(MoveItem {
            source: source.clone(),
            target,
            done: false,
        });
    }
    Ok(items)
}
pub fn hash(path: &Path) -> Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 128 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_vec())
}
fn safe_copy(source: &Path, target: &Path, verify: bool) -> Result<()> {
    let before = fs::metadata(source)?;
    ensure!(before.is_file(), "Source is not a file");
    let parent = target.parent().context("Target has no parent")?;
    let mut input = fs::File::open(source)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    let bytes = std::io::copy(&mut input, &mut tmp)?;
    ensure!(bytes == before.len(), "Source size changed during copy");
    tmp.as_file().sync_all()?;
    let after = fs::metadata(source)?;
    ensure!(
        before.len() == after.len() && before.modified()? == after.modified()?,
        "Source changed during copy"
    );
    if verify {
        ensure!(
            hash(source)? == hash(tmp.path())?,
            "Checksum mismatch; source was preserved"
        );
    }
    // persist_noclobber protects against another process creating the same name.
    tmp.persist_noclobber(target).map_err(|e| e.error)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
/// "name (k).ext" for the first k ≥ `from` (counting on from an existing
/// " (k)") that is not taken by a file or its sidecar.
fn suffixed(target: &Path, from: u64) -> Result<PathBuf> {
    let parent = target.parent().context("Missing parent")?;
    let ext = target.extension().map(|e| e.to_string_lossy().into_owned());
    let stem = target.file_stem().unwrap_or_default().to_string_lossy();
    let (base, mut k) = match stem.rsplit_once(" (") {
        Some((base, n)) if n.ends_with(')') => match n[..n.len() - 1].parse::<u64>() {
            Ok(n) => (base.to_owned(), (n + 1).max(from)),
            Err(_) => (stem.to_string(), from),
        },
        _ => (stem.to_string(), from),
    };
    loop {
        let name = match &ext {
            Some(ext) => format!("{base} ({k}).{ext}"),
            None => format!("{base} ({k})"),
        };
        let path = parent.join(name);
        if !path.exists() && !crate::metadata::sidecar(&path).exists() {
            return Ok(path);
        }
        k += 1;
        ensure!(k < 100_000, "No free name for {}", target.display());
    }
}
fn next_name(target: &Path) -> Result<PathBuf> {
    let parent = target.parent().context("Missing parent")?;
    // An original name (not a sequence number) gets " (2)" and so on; a
    // sequence name the next free number of the same width.
    let Some(mut n) = sequence_number(target, 1) else {
        return suffixed(target, 2);
    };
    let digits = target.file_stem().map_or(1, |s| s.len());
    let ext = target.extension().unwrap_or_default().to_string_lossy();
    for entry in fs::read_dir(parent)? {
        if let Some(value) = sequence_number(&entry?.path(), digits) {
            n = n.max(value);
        }
    }
    loop {
        n = n.checked_add(1).context("Sequence overflow")?;
        let path = parent.join(format!("{n:0digits$}.{ext}"));
        if !path.exists() && !crate::metadata::sidecar(&path).exists() {
            return Ok(path);
        }
    }
}
pub fn conflicts(items: &[MoveItem]) -> bool {
    items
        .iter()
        .any(|i| i.target.exists() || crate::metadata::sidecar(&i.target).exists())
}
pub fn move_batch(mut items: Vec<MoveItem>, verify: bool, policy: Conflict) -> MoveReport {
    let mut report = MoveReport::default();
    let result = (|| -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let folder = items[0]
            .target
            .parent()
            .context("No destination")?
            .to_path_buf();
        let lock = folder_lock(&folder)?;
        let _guard = lock;
        if matches!(policy, Conflict::Cancel) && conflicts(&items) {
            report.conflict = items.clone();
            bail!(
                "Name conflict: no files were moved. Choose Skip or Rename in the conflict dialog"
            );
        }
        let journal = folder.join(format!(
            ".noirview-move-{}.json",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        if verify {
            crate::storage::atomic_write(
                &journal,
                &serde_json::to_vec(&Journal {
                    version: 1,
                    items: items.clone(),
                })?,
            )?;
        }
        for idx in 0..items.len() {
            if items[idx].target.exists() || crate::metadata::sidecar(&items[idx].target).exists() {
                match policy {
                    Conflict::Skip => {
                        report.skipped += 1;
                        items[idx].done = true;
                        if verify {
                            crate::storage::atomic_write(
                                &journal,
                                &serde_json::to_vec(&Journal {
                                    version: 1,
                                    items: items.clone(),
                                })?,
                            )?;
                        }
                        continue;
                    }
                    Conflict::Rename => {
                        let mut target = next_name(&items[idx].target)?;
                        while items
                            .iter()
                            .enumerate()
                            .any(|(j, i)| j != idx && i.target.file_stem() == target.file_stem())
                        {
                            target = next_name(&target)?;
                        }
                        items[idx].target = target;
                    }
                    Conflict::Cancel => {
                        report.conflict = items[idx..].to_vec();
                        bail!(
                            "A file appeared at the destination during the move. Source preserved"
                        );
                    }
                }
            }
            if verify {
                crate::storage::atomic_write(
                    &journal,
                    &serde_json::to_vec(&Journal {
                        version: 1,
                        items: items.clone(),
                    })?,
                )?;
            }
            let i = &items[idx];
            if let Err(e) = safe_copy(&i.source, &i.target, verify) {
                if e.downcast_ref::<std::io::Error>()
                    .map(|e| e.kind() == std::io::ErrorKind::AlreadyExists)
                    .unwrap_or(false)
                {
                    report.conflict = items[idx..].to_vec();
                }
                return Err(e);
            }
            let source_side = crate::metadata::sidecar(&i.source);
            let target_side = crate::metadata::sidecar(&i.target);
            if source_side.exists() {
                safe_copy(&source_side, &target_side, verify)
                    .context("Image copied; source retained because sidecar copy failed")?;
            }
            fs::remove_file(&i.source)
                .context("Destination verified, but source removal failed; recover the move")?;
            if source_side.exists() {
                fs::remove_file(source_side)?;
            }
            report.moved.push((i.source.clone(), i.target.clone()));
            items[idx].done = true;
            if verify {
                crate::storage::atomic_write(
                    &journal,
                    &serde_json::to_vec(&Journal {
                        version: 1,
                        items: items.clone(),
                    })?,
                )?;
            }
        }
        if verify {
            fs::remove_file(journal)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        report.error = Some(format!("{e:#}"));
    }
    report
}
/// A destination folder's move lock: held while files are moved or
/// recovered there; the lock file is removed afterwards.
struct FolderLock {
    file: Option<fs::File>,
    path: PathBuf,
}
impl Drop for FolderLock {
    fn drop(&mut self) {
        // Closing releases the lock; another instance may have taken it
        // meanwhile, so a failed removal is fine.
        drop(self.file.take());
        let _ = fs::remove_file(&self.path);
    }
}
/// Explicit recovery checks content before completing an interrupted move.
fn folder_lock(folder: &Path) -> Result<FolderLock> {
    let path = folder.join(".noirview-move.lock");
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    file.try_lock_exclusive()
        .context("Another NoirView instance is moving or recovering files in this folder")?;
    Ok(FolderLock {
        file: Some(file),
        path,
    })
}
pub fn recover(folder: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    let _lock = folder_lock(folder)?;
    let mut moved = vec![];
    for entry in fs::read_dir(folder)? {
        let path = entry?.path();
        if !path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with(".noirview-move-") && n.ends_with(".json"))
            .unwrap_or(false)
        {
            continue;
        }
        let mut journal: Journal = serde_json::from_slice(&fs::read(&path)?)?;
        ensure!(journal.version == 1, "Unknown move journal version");
        for i in &mut journal.items {
            if i.done {
                continue;
            }
            if !i.source.exists() {
                ensure!(
                    i.target.exists(),
                    "Both source and target missing: {}",
                    i.source.display()
                );
                let source_side = crate::metadata::sidecar(&i.source);
                let target_side = crate::metadata::sidecar(&i.target);
                if source_side.exists() {
                    if target_side.exists() {
                        ensure!(
                            hash(&source_side)? == hash(&target_side)?,
                            "Recovery sidecar mismatch; files preserved"
                        );
                    } else {
                        safe_copy(&source_side, &target_side, true)?;
                    }
                    fs::remove_file(source_side)?;
                }
                i.done = true;
                moved.push((i.source.clone(), i.target.clone()));
                continue;
            }
            if i.target.exists() {
                ensure!(
                    hash(&i.source)? == hash(&i.target)?,
                    "Recovery found different contents; files preserved: {}",
                    i.source.display()
                );
            } else {
                safe_copy(&i.source, &i.target, true)?;
            }
            let side = crate::metadata::sidecar(&i.source);
            let target_side = crate::metadata::sidecar(&i.target);
            if side.exists() {
                if target_side.exists() {
                    ensure!(
                        hash(&side)? == hash(&target_side)?,
                        "Sidecar contents differ"
                    );
                } else {
                    safe_copy(&side, &target_side, true)?;
                }
            }
            fs::remove_file(&i.source)?;
            if side.exists() {
                fs::remove_file(side)?;
            }
            i.done = true;
            moved.push((i.source.clone(), i.target.clone()));
        }
        fs::remove_file(path)?;
    }
    Ok(moved)
}
pub fn recycle(paths: &[PathBuf]) -> (Vec<PathBuf>, Vec<String>) {
    let mut done = vec![];
    let mut errors = vec![];
    for p in paths {
        let mut batch = vec![p.clone()];
        let side = crate::metadata::sidecar(p);
        if side.exists() {
            batch.push(side);
        }
        match trash::delete_all(&batch) {
            Ok(_) => done.push(p.clone()),
            Err(e) => errors.push(format!("{}: {e}", p.display())),
        }
    }
    (done, errors)
}
/// Moves `paths` (and their sidecars) back from the Recycle Bin: for each,
/// the newest trashed item with that original path deleted at `since`
/// (Unix seconds, a few seconds' slack) or later. Returns the restored
/// images and an error line per image that could not be restored.
pub fn restore_deleted(paths: &[PathBuf], since: i64) -> (Vec<PathBuf>, Vec<String>) {
    // On Windows first straight from the drive's Recycle Bin folder: listing
    // the whole Recycle Bin through the shell (below) can take minutes.
    #[cfg(windows)]
    let (mut fast, paths) = {
        let (done, rest) = restore_from_bins(&recycle_bin_folders(paths), paths, since);
        (done, rest)
    };
    #[cfg(not(windows))]
    let mut fast: Vec<PathBuf> = vec![];
    #[cfg(windows)]
    let paths = &paths[..];
    if paths.is_empty() {
        return (fast, vec![]);
    }
    let (done, errors) = restore_through_trash(paths, since);
    fast.extend(done);
    (fast, errors)
}
/// The original path and deletion time (Unix seconds) in a Windows Recycle
/// Bin `$I` file (format 1: Vista / 7, 2: Windows 8 and later).
#[cfg(any(windows, test))]
fn recycle_info(bytes: &[u8]) -> Option<(PathBuf, i64)> {
    let u64_at = |at: usize| Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?));
    let version = u64_at(0)?;
    let filetime = u64_at(16)?;
    let wide: Vec<u16> = match version {
        1 => bytes.get(24..24 + 520)?,
        2 => {
            let chars = u32::from_le_bytes(bytes.get(24..28)?.try_into().ok()?) as usize;
            bytes.get(28..28 + chars.checked_mul(2)?)?
        }
        _ => return None,
    }
    .as_chunks::<2>()
    .0
    .iter()
    .map(|c| u16::from_le_bytes(*c))
    .take_while(|c| *c != 0)
    .collect();
    let path = String::from_utf16(&wide).ok()?;
    // Windows FILETIME: 100 ns steps since 1601.
    let unix = (filetime as i64 - 116_444_736_000_000_000) / 10_000_000;
    (!path.is_empty()).then(|| (PathBuf::from(path), unix))
}
/// Moves back the newest `$R` file of each path (and its sidecar) deleted at
/// `since` or later, from these Recycle Bin folders, removing its `$I` record.
/// Returns the images restored and those not found there (for the slower way).
#[cfg(any(windows, test))]
fn restore_from_bins(
    bins: &[PathBuf],
    paths: &[PathBuf],
    since: i64,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut records = vec![];
    for bin in bins {
        let Ok(entries) = fs::read_dir(bin) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(rest) = name.strip_prefix("$I") else {
                continue;
            };
            let Some((original, at)) = fs::read(e.path()).ok().and_then(|b| recycle_info(&b))
            else {
                continue;
            };
            if at >= since - 5 {
                records.push((original, at, e.path(), bin.join(format!("$R{rest}"))));
            }
        }
    }
    let newest = |wanted: &Path| {
        records
            .iter()
            .filter(|(original, _, _, data)| same_path(original, wanted) && data.exists())
            .max_by_key(|(_, at, _, _)| *at)
    };
    let mut done = vec![];
    let mut rest = vec![];
    for p in paths {
        let Some((_, _, info, data)) = newest(p).filter(|_| !p.exists()) else {
            rest.push(p.clone());
            continue;
        };
        if fs::rename(data, p).is_err() {
            rest.push(p.clone());
            continue;
        }
        let _ = fs::remove_file(info);
        let side = crate::metadata::sidecar(p);
        if let Some((_, _, info, data)) = newest(&side).filter(|_| !side.exists()) {
            if fs::rename(data, &side).is_ok() {
                let _ = fs::remove_file(info);
            }
        }
        done.push(p.clone());
    }
    (done, rest)
}
/// The Recycle Bin folders (one per user) on the drives of `paths`.
#[cfg(windows)]
fn recycle_bin_folders(paths: &[PathBuf]) -> Vec<PathBuf> {
    use std::path::Component;
    let mut roots: Vec<PathBuf> = paths
        .iter()
        .filter_map(|p| match p.components().next() {
            Some(Component::Prefix(prefix)) => {
                // "D:" + "\" (join would replace the path with the root).
                let mut root = prefix.as_os_str().to_os_string();
                root.push("\\");
                Some(PathBuf::from(root))
            }
            _ => None,
        })
        .collect();
    roots.sort();
    roots.dedup();
    roots
        .into_iter()
        .filter_map(|root| fs::read_dir(root.join("$Recycle.Bin")).ok())
        .flat_map(|dirs| dirs.flatten().map(|d| d.path()))
        .filter(|d| d.is_dir())
        .collect()
}
/// Restores through the trash crate (the shell's view of the Recycle Bin).
fn restore_through_trash(paths: &[PathBuf], since: i64) -> (Vec<PathBuf>, Vec<String>) {
    #[cfg(any(windows, all(unix, not(target_os = "macos"))))]
    {
        let items = match trash::os_limited::list() {
            Ok(items) => items,
            Err(e) => return (vec![], vec![format!("Cannot read the Recycle Bin: {e}")]),
        };
        let mut done = vec![];
        let mut errors = vec![];
        for p in paths {
            let wanted = [p.clone(), crate::metadata::sidecar(p)];
            let picked = newest_trashed(&items, &wanted, since);
            if !picked.iter().any(|i| same_path(&i.original_path(), p)) {
                errors.push(format!("{}: not in the Recycle Bin", p.display()));
                continue;
            }
            if p.exists() {
                errors.push(format!("{}: a file with this name exists", p.display()));
                continue;
            }
            match trash::os_limited::restore_all(picked) {
                Ok(()) => done.push(p.clone()),
                Err(e) => errors.push(format!("{}: {e}", p.display())),
            }
        }
        (done, errors)
    }
    #[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
    {
        let _ = (paths, since);
        (
            vec![],
            vec!["Restoring from the Recycle Bin is not supported here".into()],
        )
    }
}
/// For each wanted path, the newest trashed item with that original path
/// deleted at `since` or later (5 s slack for clock rounding).
#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
fn newest_trashed(
    items: &[trash::TrashItem],
    wanted: &[PathBuf],
    since: i64,
) -> Vec<trash::TrashItem> {
    wanted
        .iter()
        .filter_map(|w| {
            items
                .iter()
                .filter(|i| i.time_deleted >= since - 5 && same_path(&i.original_path(), w))
                .max_by_key(|i| i.time_deleted)
                .cloned()
        })
        .collect()
}
/// Paths compared as Windows does: case-insensitive, without `\\?\`.
fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        let norm = |p: &Path| {
            let s = p.to_string_lossy().replace('/', "\\");
            s.strip_prefix(r"\\?\").unwrap_or(&s).to_lowercase()
        };
        norm(a) == norm(b)
    } else {
        a == b
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    /// Uses the real Recycle Bin / trash: run by hand
    /// (`cargo test -- --ignored a_recycled_image_comes_back`).
    #[test]
    #[ignore]
    fn a_recycled_image_comes_back_from_the_trash() {
        let base = directories::BaseDirs::new().unwrap();
        let d = tempfile::tempdir_in(base.home_dir()).unwrap();
        let p = d.path().join("noirview-restore-test.jpg");
        fs::write(&p, b"jpeg").unwrap();
        let since = chrono::Utc::now().timestamp();
        let (done, errors) = recycle(std::slice::from_ref(&p));
        assert_eq!((done.len(), errors.len()), (1, 0), "{errors:?}");
        assert!(!p.exists());
        let (done, errors) = restore_deleted(std::slice::from_ref(&p), since);
        assert_eq!(done, std::slice::from_ref(&p), "{errors:?}");
        assert_eq!(fs::read(&p).unwrap(), b"jpeg");
        // Already back: nothing left in the trash for it.
        let (done, errors) = restore_deleted(std::slice::from_ref(&p), since);
        assert!(done.is_empty() && errors.len() == 1);
    }
    #[test]
    fn sequence_names_have_the_chosen_width_and_continue_after_the_highest() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("in");
        let dst = d.path().join("out");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        let files: Vec<_> = ["a.jpg", "b.PNG"]
            .iter()
            .map(|n| {
                let p = src.join(n);
                fs::write(&p, b"x").unwrap();
                p
            })
            .collect();
        let names = |items: Vec<MoveItem>| {
            items
                .iter()
                .map(|i| i.target.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(plan_with(&files, &dst, false, 3).unwrap()),
            ["001.jpg", "002.PNG"]
        );
        // Older eight-digit names (and shorter numbers) in the folder: on
        // after the highest; numbers shorter than the width do not count.
        fs::write(dst.join("00000041.jpg"), b"x").unwrap();
        fs::write(dst.join("7.jpg"), b"x").unwrap();
        assert_eq!(
            names(plan_with(&files, &dst, false, 3).unwrap()),
            ["042.jpg", "043.PNG"]
        );
        fs::write(dst.join("999.jpg"), b"x").unwrap();
        assert_eq!(
            names(plan_with(&files, &dst, false, 3).unwrap()),
            ["1000.jpg", "1001.PNG"]
        );
        // A taken sequence name moves on at the same width.
        fs::remove_file(dst.join("999.jpg")).unwrap();
        fs::remove_file(dst.join("00000041.jpg")).unwrap();
        fs::write(dst.join("005.jpg"), b"x").unwrap();
        assert_eq!(
            next_name(&dst.join("005.jpg")).unwrap(),
            dst.join("006.jpg")
        );
    }
    fn info_v2(path: &str, unix: i64) -> Vec<u8> {
        let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
        let mut b = vec![];
        b.extend(2u64.to_le_bytes());
        b.extend(1234u64.to_le_bytes());
        b.extend(((unix * 10_000_000 + 116_444_736_000_000_000) as u64).to_le_bytes());
        b.extend((wide.len() as u32).to_le_bytes());
        for c in wide {
            b.extend(c.to_le_bytes());
        }
        b
    }
    #[test]
    fn recycle_bin_records_are_read_in_both_formats() {
        assert_eq!(
            recycle_info(&info_v2(r"D:\Photos\a.jpg", 1_700_000_000)),
            Some((PathBuf::from(r"D:\Photos\a.jpg"), 1_700_000_000))
        );
        let mut v1 = vec![];
        v1.extend(1u64.to_le_bytes());
        v1.extend(5u64.to_le_bytes());
        v1.extend(((1_600_000_000i64 * 10_000_000 + 116_444_736_000_000_000) as u64).to_le_bytes());
        let mut path: Vec<u8> = "C:\\x.png"
            .encode_utf16()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        path.resize(520, 0);
        v1.extend(path);
        assert_eq!(
            recycle_info(&v1),
            Some((PathBuf::from("C:\\x.png"), 1_600_000_000))
        );
        assert_eq!(recycle_info(&[0; 10]), None);
        assert_eq!(recycle_info(&info_v2("", 1)), None);
    }
    #[test]
    fn images_come_back_from_a_recycle_bin_folder_with_their_sidecars() {
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("bin");
        let photos = d.path().join("photos");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&photos).unwrap();
        let a = photos.join("a.jpg");
        let side = crate::metadata::sidecar(&a);
        let b = photos.join("b.jpg");
        let record = |id: &str, original: &Path, at: i64, data: &[u8]| {
            fs::write(
                bin.join(format!("$I{id}")),
                info_v2(&original.to_string_lossy(), at),
            )
            .unwrap();
            fs::write(bin.join(format!("$R{id}")), data).unwrap();
        };
        record("OLD.jpg", &a, 100, b"old");
        record("NEW.jpg", &a, 1000, b"new");
        record("SIDE.xmp", &side, 1000, b"xmp");
        record("B.jpg", &b, 50, b"b");
        let (done, rest) =
            restore_from_bins(std::slice::from_ref(&bin), &[a.clone(), b.clone()], 990);
        assert_eq!(done, std::slice::from_ref(&a));
        assert_eq!(
            rest,
            std::slice::from_ref(&b),
            "deleted before the deletion: left for the slow way"
        );
        assert_eq!(fs::read(&a).unwrap(), b"new", "the newest copy");
        assert_eq!(fs::read(&side).unwrap(), b"xmp", "with its sidecar");
        assert!(!bin.join("$INEW.jpg").exists() && !bin.join("$RNEW.jpg").exists());
        assert!(
            bin.join("$IOLD.jpg").exists(),
            "older copies stay in the bin"
        );
        // A file of that name exists: not overwritten.
        record("NEWER.jpg", &a, 2000, b"newer");
        let (done, rest) =
            restore_from_bins(std::slice::from_ref(&bin), std::slice::from_ref(&a), 990);
        assert!(done.is_empty() && rest == [a.clone()]);
        assert_eq!(fs::read(&a).unwrap(), b"new");
    }
    #[test]
    fn restoring_picks_the_newest_trashed_copy_since_the_deletion() {
        let item = |dir: &str, name: &str, at: i64| trash::TrashItem {
            id: format!("{dir}/{name}@{at}").into(),
            name: name.into(),
            original_parent: PathBuf::from(dir),
            time_deleted: at,
        };
        let items = vec![
            item("/p", "a.jpg", 100),
            item("/p", "a.jpg", 500),
            item("/p", "a.xmp", 500),
            item("/p", "b.jpg", 200),
            item("/q", "a.jpg", 600),
        ];
        let wanted = [PathBuf::from("/p/a.jpg"), PathBuf::from("/p/a.xmp")];
        let picked = newest_trashed(&items, &wanted, 498);
        let ids: Vec<_> = picked
            .iter()
            .map(|i| i.id.to_string_lossy().into_owned())
            .collect();
        assert_eq!(ids, ["/p/a.jpg@500", "/p/a.xmp@500"]);
        // Deleted before the last deletion: not restored.
        assert!(newest_trashed(&items, &[PathBuf::from("/p/b.jpg")], 498).is_empty());
    }
    #[test]
    fn numbering_replaces_entire_stem_preserves_extension_and_continues_max() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("in");
        let dst = d.path().join("out");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(dst.join("00000010.png"), b"old").unwrap();
        fs::write(dst.join("readme.txt"), b"ignore").unwrap();
        let a = src.join("holiday.photo.JPG");
        let b = src.join("other.png");
        fs::write(&a, b"jpeg data").unwrap();
        fs::write(&b, b"png data").unwrap();
        let items = plan(&[a.clone(), b.clone()], &dst).unwrap();
        assert_eq!(items[0].target.file_name().unwrap(), "00000011.JPG");
        assert_eq!(items[1].target.file_name().unwrap(), "00000012.png");
        let report = move_batch(items, true, Conflict::Cancel);
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(fs::read(dst.join("00000011.JPG")).unwrap(), b"jpeg data");
        assert!(!a.exists());
        assert!(!b.exists());
    }
    #[test]
    fn conflict_never_overwrites_or_removes_source() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("source.jpg");
        let b = d.path().join("00000001.jpg");
        fs::write(&a, b"source").unwrap();
        fs::write(&b, b"existing").unwrap();
        let r = move_batch(
            vec![MoveItem {
                source: a.clone(),
                target: b.clone(),
                done: false,
            }],
            true,
            Conflict::Cancel,
        );
        assert!(r.error.is_some());
        assert_eq!(fs::read(a).unwrap(), b"source");
        assert_eq!(fs::read(b).unwrap(), b"existing");
    }
    #[test]
    fn recovery_finishes_verified_copy_with_sidecar() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("source.bmp");
        let b = d.path().join("00000001.bmp");
        fs::write(&a, b"same").unwrap();
        fs::write(&b, b"same").unwrap();
        fs::write(crate::metadata::sidecar(&a), b"tags").unwrap();
        let journal = Journal {
            version: 1,
            items: vec![MoveItem {
                source: a.clone(),
                target: b.clone(),
                done: false,
            }],
        };
        fs::write(
            d.path().join(".noirview-move-test.json"),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        assert_eq!(recover(d.path()).unwrap().len(), 1);
        assert!(!a.exists());
        assert_eq!(fs::read(crate::metadata::sidecar(&b)).unwrap(), b"tags");
    }
    #[test]
    fn sequence_names_can_fill_the_free_numbers_first() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        for n in [1, 3, 4, 5, 6, 7] {
            fs::write(dst.path().join(format!("{n}.png")), b"x").unwrap();
        }
        let files: Vec<PathBuf> = ["a.png", "b.jpg", "c.png"]
            .iter()
            .map(|n| {
                let p = src.path().join(n);
                fs::write(&p, b"y").unwrap();
                p
            })
            .collect();
        let names = |items: Vec<MoveItem>| -> Vec<String> {
            items
                .iter()
                .map(|i| i.target.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(
            names(plan_names(&files, dst.path(), false, 1, true).unwrap()),
            ["2.png", "8.jpg", "9.png"]
        );
        assert_eq!(
            names(plan_names(&files, dst.path(), false, 1, false).unwrap()),
            ["8.png", "9.jpg", "10.png"]
        );
        // An empty destination starts at 1 either way.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            names(plan_names(&files[..1], empty.path(), false, 3, true).unwrap()),
            ["001.png"]
        );
    }
    #[test]
    fn active_move_lock_blocks_recovery_and_releases_after_drop() {
        let dir = tempfile::tempdir().unwrap();
        let lock = folder_lock(dir.path()).unwrap();
        assert!(recover(dir.path()).is_err());
        drop(lock);
        assert!(recover(dir.path()).unwrap().is_empty());
    }
    #[test]
    fn rename_never_reuses_a_number_from_another_extension() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("original.jpg");
        fs::write(&src, b"source").unwrap();
        let target = d.path().join("00000001.jpg");
        fs::write(&target, b"existing").unwrap();
        fs::write(d.path().join("00000009.png"), b"other").unwrap();
        let report = move_batch(
            vec![MoveItem {
                source: src,
                target: target.clone(),
                done: false,
            }],
            true,
            Conflict::Rename,
        );
        assert!(report.error.is_none());
        assert_eq!(report.moved[0].1.file_name().unwrap(), "00000010.jpg");
        assert!(
            !d.path().join(".noirview-move.lock").exists(),
            "the lock file goes after the move"
        );
        assert_eq!(fs::read(target).unwrap(), b"existing");
    }
    #[test]
    fn original_names_are_kept_and_collisions_get_a_number() {
        let d = tempfile::tempdir().unwrap();
        let (a, b, dest) = (
            d.path().join("a"),
            d.path().join("b"),
            d.path().join("dest"),
        );
        for dir in [&a, &b, &dest] {
            fs::create_dir(dir).unwrap();
        }
        fs::write(a.join("cat.jpg"), b"1").unwrap();
        fs::write(b.join("cat.jpg"), b"2").unwrap();
        fs::write(a.join("dog.png"), b"3").unwrap();
        let files = [a.join("cat.jpg"), b.join("cat.jpg"), a.join("dog.png")];
        let items = plan_with(&files, &dest, true, 3).unwrap();
        let names: Vec<_> = items
            .iter()
            .map(|i| i.target.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["cat.jpg", "cat (2).jpg", "dog.png"]);
        // An existing target: Rename gives the next free " (k)".
        fs::write(dest.join("dog.png"), b"old").unwrap();
        assert!(conflicts(&items));
        let report = move_batch(items, false, Conflict::Rename);
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(fs::read(dest.join("dog (2).png")).unwrap(), b"3");
        assert_eq!(fs::read(dest.join("dog.png")).unwrap(), b"old");
        assert_eq!(fs::read(dest.join("cat (2).jpg")).unwrap(), b"2");
        // Sequence names still work as before.
        let n = plan(&[b.join("x.jpg")], &dest);
        assert!(n.is_ok());
    }
    #[test]
    fn recovery_cleans_sidecar_after_source_image_was_already_removed() {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("gone.bmp");
        let target = d.path().join("00000001.bmp");
        fs::write(&target, b"published").unwrap();
        fs::write(crate::metadata::sidecar(&source), b"keywords").unwrap();
        let journal = Journal {
            version: 1,
            items: vec![MoveItem {
                source: source.clone(),
                target: target.clone(),
                done: false,
            }],
        };
        fs::write(
            d.path().join(".noirview-move-test.json"),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        recover(d.path()).unwrap();
        assert!(!crate::metadata::sidecar(&source).exists());
        assert_eq!(
            fs::read(crate::metadata::sidecar(&target)).unwrap(),
            b"keywords"
        );
    }
}
