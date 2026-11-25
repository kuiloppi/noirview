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

/// Case-insensitive comparison where digit runs compare by value. Ties fall
/// back to the plain string order. Allocates nothing.
pub fn natural(a: &str, b: &str) -> Ordering {
    let (mut i, mut j) = (0, 0);
    loop {
        match (a[i..].chars().next(), b[j..].chars().next()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let end = |s: &str, from: usize| {
                    from + s.as_bytes()[from..]
                        .iter()
                        .take_while(|b| b.is_ascii_digit())
                        .count()
                };
                let (ie, je) = (end(a, i), end(b, j));
                let (m, n) = (
                    a[i..ie].trim_start_matches('0'),
                    b[j..je].trim_start_matches('0'),
                );
                let order = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if order != Ordering::Equal {
                    return order;
                }
                (i, j) = (ie, je);
            }
            (Some(c), Some(d)) => {
                let order = c.to_lowercase().cmp(d.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                i += c.len_utf8();
                j += d.len_utf8();
            }
        }
    }
}

fn name(p: &Path) -> std::borrow::Cow<'_, str> {
    p.file_name().unwrap_or_default().to_string_lossy()
}
fn extension(p: &Path) -> String {
    p.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}
/// Name order: natural file name, then the whole path.
fn name_order(a: (&str, &Path), b: (&str, &Path)) -> Ordering {
    natural(a.0, b.0).then_with(|| a.1.cmp(b.1))
}

/// Whether `paths` are already in the order `sort` gives for Name or Type
/// (one pass; other keys answer false).
pub fn is_sorted(paths: &[PathBuf], key: SortKey, descending: bool) -> bool {
    if !matches!(key, SortKey::Name | SortKey::Type) {
        return false;
    }
    paths.windows(2).all(|w| {
        let (a, b) = if descending {
            (&w[1], &w[0])
        } else {
            (&w[0], &w[1])
        };
        let names = name_order((&name(a), a), (&name(b), b));
        let order = if key == SortKey::Type {
            extension(a).cmp(&extension(b)).then(names)
        } else {
            names
        };
        order != Ordering::Greater
    })
}

/// Sort in place. Metadata keys read each file's metadata once; unreadable
/// files sort last. Ties fall back to the natural name order.
pub fn sort(paths: &mut [PathBuf], key: SortKey, descending: bool) {
    let by_name = |a: &PathBuf, b: &PathBuf| name_order((&name(a), a), (&name(b), b));
    // Name and Type: keys made once per path, not on every comparison.
    let decorated = |paths: &mut [PathBuf], with_type: bool| {
        let mut keyed: Vec<(String, String, PathBuf)> = paths
            .iter_mut()
            .map(|p| {
                let ext = if with_type {
                    extension(p)
                } else {
                    String::new()
                };
                (ext, name(p).into_owned(), std::mem::take(p))
            })
            .collect();
        keyed.sort_by(|(xa, na, a), (xb, nb, b)| {
            xa.cmp(xb).then_with(|| name_order((na, a), (nb, b)))
        });
        for (slot, (_, _, p)) in paths.iter_mut().zip(keyed) {
            *slot = p;
        }
    };
    match key {
        SortKey::Name => decorated(paths, false),
        SortKey::Type => decorated(paths, true),
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

    /// The comparator before keys were made once (reference for the order).
    fn old_natural(a: &str, b: &str) -> Ordering {
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
    fn old_sort(paths: &mut [PathBuf], key: SortKey, descending: bool) {
        let name = |p: &PathBuf| p.file_name().unwrap().to_string_lossy().into_owned();
        let by_name =
            |a: &PathBuf, b: &PathBuf| old_natural(&name(a), &name(b)).then_with(|| a.cmp(b));
        let ext = |p: &PathBuf| {
            p.extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase()
        };
        match key {
            SortKey::Name => paths.sort_by(by_name),
            _ => paths.sort_by(|a, b| ext(a).cmp(&ext(b)).then_with(|| by_name(a, b))),
        }
        if descending {
            paths.reverse();
        }
    }

    #[test]
    fn keyed_name_and_type_sorting_gives_the_same_order_as_before() {
        use rand::{seq::SliceRandom, Rng, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let parts = [
            "img", "IMG", "Img", "a", "Z", "äiti", "Ärla", "ß", "photo_", "x-", " ", ".", "é",
            "日本",
        ];
        let exts = ["jpg", "JPG", "png", "Png", "webp", "gif", ""];
        let folders = ["/p", "/p/sub", "/P", "/q/2"];
        for round in 0..40 {
            let mut paths: Vec<PathBuf> = (0..200)
                .map(|_| {
                    let mut name = String::new();
                    for _ in 0..rng.gen_range(1..4) {
                        name.push_str(parts.choose(&mut rng).unwrap());
                        if rng.gen_bool(0.7) {
                            let zeros = "0".repeat(rng.gen_range(0..3));
                            name.push_str(&format!("{zeros}{}", rng.gen_range(0..120)));
                        }
                    }
                    let ext = exts.choose(&mut rng).unwrap();
                    if !ext.is_empty() {
                        name = format!("{name}.{ext}");
                    }
                    Path::new(folders.choose(&mut rng).unwrap()).join(name)
                })
                .collect();
            // Equal names in different folders.
            paths.push(PathBuf::from("/p/same.jpg"));
            paths.push(PathBuf::from("/q/2/same.jpg"));
            for key in [SortKey::Name, SortKey::Type] {
                for descending in [false, true] {
                    let mut new = paths.clone();
                    let mut old = paths.clone();
                    sort(&mut new, key, descending);
                    old_sort(&mut old, key, descending);
                    assert_eq!(new, old, "round {round}, {key:?}, descending {descending}");
                    assert!(is_sorted(&new, key, descending));
                }
            }
            // Pairwise as well (every outcome, not only the sorted result).
            for w in paths.windows(2) {
                let (a, b) = (name(&w[0]), name(&w[1]));
                assert_eq!(natural(&a, &b), old_natural(&a, &b), "{a} / {b}");
            }
        }
        let mut shuffled = vec![PathBuf::from("/b2.jpg"), PathBuf::from("/b10.jpg")];
        assert!(is_sorted(&shuffled, SortKey::Name, false));
        shuffled.reverse();
        assert!(!is_sorted(&shuffled, SortKey::Name, false));
        assert!(is_sorted(&shuffled, SortKey::Name, true));
        assert!(!is_sorted(&shuffled, SortKey::Size, false));
    }

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
