//! Compact in-memory lists for large folders. Not wired into the program yet.
//!
//! A `Scope` is what exists in an opened folder, built from catalog rows. A `View`
//! is what is shown, in display order. Images are referred to by their number in
//! the scope; sets of images are one bit per image. Nothing here stores one
//! separately allocated path per image.
use crate::catalog::{join, key, path_of};
use anyhow::{Context, Result};
use std::{
    cmp::Ordering,
    collections::HashMap,
    ffi::OsStr,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

#[cfg(test)]
mod scale_bench;

/// The images of one opened folder (and, if asked for, its subfolders).
///
/// Images are numbered `0..len()` in a canonical order: folder path bytes, then
/// file-name bytes. A number is valid for this `Scope` only; use `translate` to
/// carry it to another.
pub struct Scope {
    /// Folder path bytes, ascending.
    folders: Vec<Box<[u8]>>,
    /// Number of the first image of each folder, plus the total at the end.
    folder_start: Vec<u32>,
    folder_index: HashMap<Box<[u8]>, u32>,
    /// All file names back to back; `name_end[n]` is where image n's name ends.
    names: Vec<u8>,
    name_end: Vec<u32>,
    /// Catalog id of each image.
    ids: Vec<u32>,
    /// Image numbers ordered by catalog id.
    by_id: Vec<u32>,
}
impl Scope {
    /// Build from `Catalog::listing`.
    pub fn build(mut listing: crate::catalog::Listing) -> Result<Self> {
        listing.retain(|(_, images)| !images.is_empty());
        listing.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let total: usize = listing.iter().map(|(_, images)| images.len()).sum();
        u32::try_from(total).context("Too many images for one scope")?;
        let mut scope = Self {
            folders: Vec::with_capacity(listing.len()),
            folder_start: Vec::with_capacity(listing.len() + 1),
            folder_index: HashMap::with_capacity(listing.len()),
            names: Vec::new(),
            name_end: Vec::with_capacity(total),
            ids: Vec::with_capacity(total),
            by_id: Vec::new(),
        };
        for (folder, mut images) in listing {
            images.sort_unstable_by(|a, b| a.1.cmp(&b.1));
            let folder: Box<[u8]> = folder.into();
            scope
                .folder_index
                .insert(folder.clone(), scope.folders.len() as u32);
            scope.folders.push(folder);
            scope.folder_start.push(scope.ids.len() as u32);
            for (id, name) in images {
                scope.names.extend_from_slice(&name);
                scope
                    .name_end
                    .push(u32::try_from(scope.names.len()).context("File names exceed 4 GiB")?);
                scope.ids.push(id);
            }
        }
        scope.folder_start.push(scope.ids.len() as u32);
        scope.by_id = (0..total as u32).collect();
        scope.by_id.sort_unstable_by_key(|n| scope.ids[*n as usize]);
        Ok(scope)
    }
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    pub fn id(&self, number: u32) -> u32 {
        self.ids[number as usize]
    }
    fn name(&self, number: u32) -> &[u8] {
        let n = number as usize;
        let start = if n == 0 { 0 } else { self.name_end[n - 1] };
        &self.names[start as usize..self.name_end[n] as usize]
    }
    fn folder(&self, number: u32) -> u32 {
        // The last folder whose first image is at or before `number`.
        (self.folder_start.partition_point(|start| *start <= number) - 1) as u32
    }
    pub fn path(&self, number: u32) -> PathBuf {
        path_of(join(
            &self.folders[self.folder(number) as usize],
            self.name(number),
        ))
    }
    pub fn number_of_path(&self, path: &Path) -> Option<u32> {
        let folder = *self.folder_index.get(key(path.parent()?).as_slice())?;
        let name = path.file_name()?.as_encoded_bytes();
        let (start, end) = (
            self.folder_start[folder as usize],
            self.folder_start[folder as usize + 1],
        );
        let mut low = start;
        let mut high = end;
        while low < high {
            let middle = low + (high - low) / 2;
            match self.name(middle).cmp(name) {
                Ordering::Less => low = middle + 1,
                Ordering::Greater => high = middle,
                Ordering::Equal => return Some(middle),
            }
        }
        None
    }
    pub fn number_of_id(&self, id: u32) -> Option<u32> {
        self.by_id
            .binary_search_by_key(&id, |n| self.ids[*n as usize])
            .ok()
            .map(|at| self.by_id[at])
    }
    /// The set of images whose catalog ids are in `ids`; ids outside the scope are ignored.
    pub fn set_of_ids(&self, ids: &[u32]) -> ImageSet {
        let mut set = ImageSet::empty(self.len());
        for id in ids {
            if let Some(number) = self.number_of_id(*id) {
                set.insert(number);
            }
        }
        set
    }
    /// The same order `Path::cmp` gives the two full paths, without building them.
    fn compare_paths(&self, a: u32, b: u32) -> Ordering {
        let parts = |number: u32| {
            // SAFETY: both slices hold bytes produced by `as_encoded_bytes`.
            let (folder, name) = unsafe {
                (
                    OsStr::from_encoded_bytes_unchecked(
                        &self.folders[self.folder(number) as usize],
                    ),
                    OsStr::from_encoded_bytes_unchecked(self.name(number)),
                )
            };
            Path::new(folder)
                .components()
                .chain(std::iter::once(Component::Normal(name)))
        };
        parts(a).cmp(parts(b))
    }
}
/// The number in `new` of the image that has `number` in `old`, if it is still there.
pub fn translate(old: &Scope, new: &Scope, number: u32) -> Option<u32> {
    new.number_of_id(old.id(number))
}

/// A set of image numbers of one scope: one bit per image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageSet {
    bits: Vec<u64>,
    universe: usize,
}
impl ImageSet {
    pub fn empty(universe: usize) -> Self {
        Self {
            bits: vec![0; universe.div_ceil(64)],
            universe,
        }
    }
    pub fn full(universe: usize) -> Self {
        let mut set = Self::empty(universe);
        set.complement();
        set
    }
    pub fn insert(&mut self, number: u32) {
        assert!((number as usize) < self.universe);
        self.bits[number as usize / 64] |= 1 << (number % 64);
    }
    pub fn contains(&self, number: u32) -> bool {
        (number as usize) < self.universe
            && self.bits[number as usize / 64] >> (number % 64) & 1 == 1
    }
    pub fn count(&self) -> usize {
        self.bits
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }
    pub fn intersect(&mut self, other: &Self) {
        assert_eq!(self.universe, other.universe);
        for (word, theirs) in self.bits.iter_mut().zip(&other.bits) {
            *word &= theirs;
        }
    }
    pub fn unite(&mut self, other: &Self) {
        assert_eq!(self.universe, other.universe);
        for (word, theirs) in self.bits.iter_mut().zip(&other.bits) {
            *word |= theirs;
        }
    }
    pub fn complement(&mut self) {
        for word in &mut self.bits {
            *word = !*word;
        }
        let tail = self.universe % 64;
        if tail != 0 {
            *self.bits.last_mut().unwrap() &= (1 << tail) - 1;
        }
    }
    /// Combine per-keyword sets the way `model::Filter` combines keywords.
    pub fn filter(universe: usize, mode: crate::model::FilterMode, keywords: &[ImageSet]) -> Self {
        use crate::model::FilterMode;
        if keywords.is_empty() {
            return Self::full(universe);
        }
        match mode {
            FilterMode::All => {
                let mut set = keywords[0].clone();
                keywords[1..].iter().for_each(|k| set.intersect(k));
                set
            }
            FilterMode::Any | FilterMode::Exclude => {
                let mut set = Self::empty(universe);
                keywords.iter().for_each(|k| set.unite(k));
                if mode == FilterMode::Exclude {
                    set.complement();
                }
                set
            }
        }
    }
}

/// The shown images of a scope in display order. Replaces `model::VisibleFiles`.
pub struct View {
    scope: Arc<Scope>,
    /// Image numbers in display order.
    order: Vec<u32>,
    /// Position of each image number in `order`, or `HIDDEN`.
    place: Vec<u32>,
}
const HIDDEN: u32 = u32::MAX;
impl View {
    /// Images of `shown` (all when `None`) ordered by file name exactly as
    /// `sorting::sort` orders paths with `SortKey::Name`.
    pub fn by_name(scope: Arc<Scope>, shown: Option<&ImageSet>, descending: bool) -> Self {
        // Natural comparison is the expensive part and many folders repeat the same
        // names, so compare each distinct name once and sort images by its rank.
        let mut distinct: HashMap<&[u8], u32> = HashMap::new();
        let mut name_of: Vec<u32> = Vec::with_capacity(scope.len());
        let mut texts: Vec<String> = Vec::new();
        for number in 0..scope.len() as u32 {
            let name = scope.name(number);
            let next = distinct.len() as u32;
            let index = *distinct.entry(name).or_insert_with(|| {
                texts.push(String::from_utf8_lossy(name).into_owned());
                next
            });
            name_of.push(index);
        }
        let mut sorted: Vec<u32> = (0..texts.len() as u32).collect();
        sorted.sort_unstable_by(|a, b| {
            crate::sorting::natural(&texts[*a as usize], &texts[*b as usize]).then(a.cmp(b))
        });
        // Names that compare equal (identical after lossy conversion) share a rank.
        let mut rank = vec![0u32; texts.len()];
        let mut current = 0;
        for (at, name) in sorted.iter().enumerate() {
            if at > 0
                && crate::sorting::natural(&texts[sorted[at - 1] as usize], &texts[*name as usize])
                    != Ordering::Equal
            {
                current += 1;
            }
            rank[*name as usize] = current;
        }
        let mut order: Vec<u32> = (0..scope.len() as u32)
            .filter(|number| shown.is_none_or(|set| set.contains(*number)))
            .collect();
        order.sort_unstable_by(|a, b| {
            rank[name_of[*a as usize] as usize]
                .cmp(&rank[name_of[*b as usize] as usize])
                .then_with(|| scope.compare_paths(*a, *b))
        });
        if descending {
            order.reverse();
        }
        let mut place = vec![HIDDEN; scope.len()];
        for (position, number) in order.iter().enumerate() {
            place[*number as usize] = position as u32;
        }
        Self {
            scope,
            order,
            place,
        }
    }
    pub fn len(&self) -> usize {
        self.order.len()
    }
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
    pub fn number_at(&self, position: usize) -> Option<u32> {
        self.order.get(position).copied()
    }
    pub fn path_at(&self, position: usize) -> Option<PathBuf> {
        self.number_at(position)
            .map(|number| self.scope.path(number))
    }
    pub fn position_of_number(&self, number: u32) -> Option<usize> {
        self.place
            .get(number as usize)
            .filter(|place| **place != HIDDEN)
            .map(|place| *place as usize)
    }
    pub fn position_of_path(&self, path: &Path) -> Option<usize> {
        self.position_of_number(self.scope.number_of_path(path)?)
    }
    pub fn first(&self) -> Option<PathBuf> {
        self.path_at(0)
    }
    pub fn last(&self) -> Option<PathBuf> {
        self.len().checked_sub(1).and_then(|at| self.path_at(at))
    }
    /// Paths of one page, as the grid needs them.
    pub fn page(&self, start: usize, count: usize) -> Vec<PathBuf> {
        self.order
            .iter()
            .skip(start)
            .take(count)
            .map(|number| self.scope.path(*number))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog::{Catalog, Depth, Entry, Stamp},
        model::{Filter, FilterMode, PhotoMeta},
        sorting::{sort, SortKey},
    };

    fn entry(tags: &[&str]) -> Entry {
        Entry {
            image: Stamp {
                size: 1,
                modified_ns: Some(1),
            },
            sidecar: None,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
        }
    }
    /// A folder tree with repeated names, digits, case and non-ASCII names, and
    /// one folder that is a byte-wise but not component-wise neighbour of another.
    fn fixture(root: &Path) -> Vec<(PathBuf, Entry)> {
        let names = [
            ("img2.jpg", &["a"][..]),
            ("img10.jpg", &["a", "b"]),
            ("IMG3.JPG", &["b"]),
            ("img002.jpg", &[]),
            ("äiti.png", &["c"]),
            ("Z.png", &["a", "c"]),
            ("a.jpg", &[]),
        ];
        let mut rows = Vec::new();
        for folder in ["", "trip", "trip-2", "trip/day 1", "Ärla"] {
            let folder = folder
                .split('/')
                .filter(|part| !part.is_empty())
                .fold(root.to_path_buf(), |path, part| path.join(part));
            for (name, tags) in names {
                rows.push((folder.join(name), entry(tags)));
            }
        }
        rows
    }
    fn scope_of(catalog: &Catalog, root: &Path) -> Arc<Scope> {
        Arc::new(Scope::build(catalog.listing(root, Depth::Subtree).unwrap()).unwrap())
    }
    #[test]
    fn name_order_matches_the_existing_sort_in_both_directions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        let rows = fixture(&root);
        let mut catalog = Catalog::open(&dir.path().join("catalog.sqlite3")).unwrap();
        catalog.store(&rows).unwrap();
        let scope = scope_of(&catalog, &root);
        assert_eq!(scope.len(), rows.len());
        for descending in [false, true] {
            let mut expected: Vec<PathBuf> = rows.iter().map(|(path, _)| path.clone()).collect();
            sort(&mut expected, SortKey::Name, descending);
            let view = View::by_name(scope.clone(), None, descending);
            assert_eq!(view.page(0, usize::MAX), expected);
            assert_eq!(view.first(), expected.first().cloned());
            assert_eq!(view.last(), expected.last().cloned());
            for (position, path) in expected.iter().enumerate() {
                assert_eq!(view.position_of_path(path), Some(position));
                assert_eq!(view.path_at(position).as_ref(), Some(path));
            }
            assert_eq!(view.page(3, 4), expected[3..7]);
        }
    }
    #[test]
    fn lookups_round_trip_and_reject_what_is_outside_the_scope() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        let rows = fixture(&root);
        let mut catalog = Catalog::open(&dir.path().join("catalog.sqlite3")).unwrap();
        catalog.store(&rows).unwrap();
        catalog
            .store(&[(dir.path().join("elsewhere").join("x.jpg"), entry(&[]))])
            .unwrap();
        let scope = scope_of(&catalog, &root);
        for (path, _) in &rows {
            let number = scope.number_of_path(path).unwrap();
            assert_eq!(&scope.path(number), path);
            assert_eq!(scope.number_of_id(scope.id(number)), Some(number));
        }
        assert_eq!(scope.number_of_path(&root.join("missing.jpg")), None);
        assert_eq!(
            scope.number_of_path(&dir.path().join("elsewhere").join("x.jpg")),
            None
        );
        let outside = catalog
            .ids(&dir.path().join("elsewhere"), Depth::Children)
            .unwrap()[0]
            .0;
        assert_eq!(scope.number_of_id(outside), None);
        // A subfolder scope numbers its images independently.
        let trip = scope_of(&catalog, &root.join("trip"));
        assert_eq!(trip.len(), 14);
    }
    #[test]
    fn keyword_sets_combine_like_the_existing_filter() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        let rows = fixture(&root);
        let mut catalog = Catalog::open(&dir.path().join("catalog.sqlite3")).unwrap();
        catalog.store(&rows).unwrap();
        let scope = scope_of(&catalog, &root);
        for mode in [FilterMode::All, FilterMode::Any, FilterMode::Exclude] {
            for wanted in [&["a"][..], &["a", "b"], &["c", "missing"], &[]] {
                let sets: Vec<ImageSet> = wanted
                    .iter()
                    .map(|tag| scope.set_of_ids(&catalog.ids_with_keyword(tag).unwrap()))
                    .collect();
                let shown = ImageSet::filter(scope.len(), mode, &sets);
                let filter = Filter {
                    tags: wanted.iter().map(|tag| tag.to_string()).collect(),
                    mode,
                    ..Default::default()
                };
                let mut expected = 0;
                for (path, entry) in &rows {
                    let matches = filter.matches(&PhotoMeta {
                        tags: entry.tags.clone(),
                        ..Default::default()
                    });
                    expected += usize::from(matches);
                    assert_eq!(
                        shown.contains(scope.number_of_path(path).unwrap()),
                        matches,
                        "{mode:?} {wanted:?} {path:?}"
                    );
                }
                assert_eq!(shown.count(), expected);
                let view = View::by_name(scope.clone(), Some(&shown), false);
                assert_eq!(view.len(), expected);
                assert!(view
                    .page(0, usize::MAX)
                    .iter()
                    .all(|path| view.position_of_path(path).is_some()));
            }
        }
    }
    #[test]
    fn numbers_translate_between_scopes_through_catalog_ids() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("photos");
        let rows = fixture(&root);
        let mut catalog = Catalog::open(&dir.path().join("catalog.sqlite3")).unwrap();
        catalog.store(&rows).unwrap();
        let old = scope_of(&catalog, &root);
        let removed = rows[5].0.clone();
        catalog.remove(&removed).unwrap();
        let added = root.join("000 first.jpg");
        catalog.store(&[(added.clone(), entry(&[]))]).unwrap();
        let new = scope_of(&catalog, &root);
        for (path, _) in &rows {
            let before = old.number_of_path(path).unwrap();
            let after = translate(&old, &new, before);
            if path == &removed {
                assert_eq!(after, None);
            } else {
                assert_eq!(new.path(after.unwrap()), *path);
            }
        }
        assert!(new.number_of_path(&added).is_some());
        let mut all = ImageSet::full(new.len());
        assert_eq!(all.count(), rows.len());
        all.complement();
        assert_eq!(all.count(), 0);
    }
}
