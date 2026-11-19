//! Image order for both the grid and the single-image view.
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, path::Path, path::PathBuf, time::SystemTime};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortKey {
    /// Natural order: "img2" before "img10".
    #[default]
    Name,
    Modified,
    Created,
    Size,
    Type,
}

impl SortKey {
    pub const ALL: [SortKey; 5] = [
        SortKey::Name,
        SortKey::Modified,
        SortKey::Created,
        SortKey::Size,
        SortKey::Type,
    ];
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Modified => "Date modified",
            SortKey::Created => "Date created",
            SortKey::Size => "File size",
            SortKey::Type => "File type",
        }
    }
    /// Keys that need file metadata (read on a background thread).
    pub fn needs_metadata(self) -> bool {
        matches!(self, SortKey::Modified | SortKey::Created | SortKey::Size)
    }
}

/// Case-insensitive comparison where digit runs compare by value.
pub fn natural(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (m, n) = (take(&mut x), take(&mut y));
                let (mt, nt) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let order = mt.len().cmp(&nt.len()).then_with(|| mt.cmp(nt));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(c), Some(d)) => {
                let order = c.to_lowercase().cmp(d.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                x.next();
                y.next();
            }
        }
    }
}

fn name(p: &Path) -> String {
    p.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Sort in place. Metadata keys read each file's metadata once; unreadable
/// files sort last. Ties fall back to the natural name order.
pub fn sort(paths: &mut [PathBuf], key: SortKey, descending: bool) {
    let by_name = |a: &PathBuf, b: &PathBuf| natural(&name(a), &name(b)).then_with(|| a.cmp(b));
    match key {
        SortKey::Name => paths.sort_by(by_name),
        SortKey::Type => paths.sort_by(|a, b| {
            let ext = |p: &PathBuf| {
                p.extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase()
            };
            ext(a).cmp(&ext(b)).then_with(|| by_name(a, b))
        }),
        _ => {
            let value = |p: &PathBuf| -> Option<u128> {
                let m = std::fs::metadata(p).ok()?;
                let time = |t: std::io::Result<SystemTime>| {
                    t.ok()?
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .ok()
                        .map(|d| d.as_nanos())
                };
                match key {
                    SortKey::Modified => time(m.modified()),
                    SortKey::Created => time(m.created()),
                    _ => Some(m.len() as u128),
                }
            };
            let mut keyed: Vec<(Option<u128>, PathBuf)> =
                paths.iter().map(|p| (value(p), p.clone())).collect();
            keyed.sort_by(|(x, a), (y, b)| match (x, y) {
                (Some(x), Some(y)) => {
                    let o = x.cmp(y);
                    if descending { o.reverse() } else { o }.then_with(|| by_name(a, b))
                }
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => by_name(a, b),
            });
            for (slot, (_, p)) in paths.iter_mut().zip(keyed) {
                *slot = p;
            }
            return;
        }
    }
    if descending {
        paths.reverse();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order_compares_numbers_by_value_and_ignores_case() {
        let mut v = vec!["img10.jpg", "IMG2.jpg", "img1.jpg", "img02b.jpg", "a.jpg"];
        v.sort_by(|a, b| natural(a, b));
        assert_eq!(
            v,
            ["a.jpg", "img1.jpg", "IMG2.jpg", "img02b.jpg", "img10.jpg"]
        );
        assert_eq!(natural("00000011.png", "00000002.png"), Ordering::Greater);
    }

    #[test]
    fn metadata_and_type_keys_sort_both_ways() {
        let dir = tempfile::tempdir().unwrap();
        let mk = |n: &str, len: usize| {
            let p = dir.path().join(n);
            std::fs::write(&p, vec![0u8; len]).unwrap();
            p
        };
        let (a, b, c) = (mk("a.png", 30), mk("b.jpg", 10), mk("c.jpg", 20));
        let mut v = vec![a.clone(), b.clone(), c.clone()];
        sort(&mut v, SortKey::Size, false);
        assert_eq!(v, [b.clone(), c.clone(), a.clone()]);
        sort(&mut v, SortKey::Size, true);
        assert_eq!(v, [a.clone(), c.clone(), b.clone()]);
        sort(&mut v, SortKey::Type, false);
        assert_eq!(v, [b.clone(), c.clone(), a.clone()]);
        sort(&mut v, SortKey::Name, true);
        assert_eq!(v, [c.clone(), b.clone(), a.clone()]);
        // Unreadable files go last whichever the direction.
        let gone = dir.path().join("0-missing.jpg");
        let mut v = vec![gone.clone(), a.clone(), b.clone()];
        sort(&mut v, SortKey::Modified, true);
        assert_eq!(v.last(), Some(&gone));
    }
}
