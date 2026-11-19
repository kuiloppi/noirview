//! Grid geometry: equally sized 3:2 cells that use as much of the area as possible.
//! Pure functions so the layout and the "full grid" rounding can be tested.

/// Image box aspect ratio (width / height). Fixed so cells never jump between pages.
pub const ASPECT: f32 = 1.5;
pub const MAX_COUNT: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub columns: usize,
    pub rows: usize,
    /// Image box size; the caption strip is added below it.
    pub image_w: f32,
    pub image_h: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub width: f32,
    pub height: f32,
    pub gap: f32,
    /// Height of the fixed caption strip under each image box.
    pub caption: f32,
}

impl Area {
    fn cell_width(&self, columns: usize, rows: usize) -> f32 {
        let by_width = (self.width - (columns - 1) as f32 * self.gap) / columns as f32;
        let by_height =
            ((self.height - (rows - 1) as f32 * self.gap) / rows as f32 - self.caption) * ASPECT;
        by_width.min(by_height).max(1.)
    }
}

/// Column count that gives the largest equal cells for `count` images.
pub fn layout(count: usize, area: Area) -> Layout {
    let count = count.max(1);
    let mut best = (1, count, 0.);
    for columns in 1..=count {
        let rows = count.div_ceil(columns);
        let w = area.cell_width(columns, rows);
        // Prefer fewer rows on ties so the page reads left to right.
        if w > best.2 + 0.01 {
            best = (columns, rows, w);
        }
    }
    let (columns, rows, w) = best;
    Layout {
        columns,
        rows,
        image_w: w,
        image_h: w / ASPECT,
    }
}

/// A count is "full" when its best layout has no empty slots.
pub fn is_full(count: usize, area: Area) -> bool {
    let l = layout(count, area);
    l.columns * l.rows == count.max(1)
}

/// Smallest full count >= `count`. When the folder (or filter) does not have
/// enough images for that page, the requested count is kept unchanged.
pub fn round_up(count: usize, available: usize, area: Area) -> usize {
    let count = count.clamp(1, MAX_COUNT);
    (count..=MAX_COUNT)
        .find(|&n| is_full(n, area))
        .filter(|&n| n <= available.max(count))
        .unwrap_or(count)
}

/// Page size actually shown. With rounding on, a folder (or filter) with no
/// more images than the configured size shows all of them on one page, and
/// otherwise the size is rounded up to the next full grid. The configured
/// value itself is kept, so larger folders return to it.
pub fn effective(count: usize, round: bool, available: usize, area: Option<Area>) -> usize {
    let count = count.clamp(1, MAX_COUNT);
    if !round {
        return count;
    }
    if available > 0 && available <= count {
        return available;
    }
    area.map_or(count, |area| round_up(count, available, area))
}

/// Next value for the − / + buttons. With rounding on they step between full
/// counts; otherwise by one.
pub fn step(count: usize, up: bool, round: bool, available: usize, area: Area) -> usize {
    let count = count.clamp(1, MAX_COUNT);
    if !round {
        return if up {
            (count + 1).min(MAX_COUNT)
        } else {
            count.saturating_sub(1).max(1)
        };
    }
    if up {
        let next = ((count + 1)..=MAX_COUNT)
            .find(|&n| is_full(n, area))
            .unwrap_or(count);
        // Beyond the available images a non-full value is pointless; stay.
        if next > available.max(1) && count >= available.max(1) {
            count
        } else {
            next
        }
    } else {
        (1..count).rev().find(|&n| is_full(n, area)).unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: Area = Area {
        width: 1904.,
        height: 980.,
        gap: 12.,
        caption: 36.,
    };

    #[test]
    fn four_images_fill_a_wide_window_instead_of_small_capped_tiles() {
        let l = layout(4, WIDE);
        assert_eq!((l.columns, l.rows), (2, 2));
        // The old layout capped cells at 300 px tall; the new one uses the space.
        assert!(l.image_h > 400., "{l:?}");
        assert!(l.columns as f32 * l.image_w + WIDE.gap <= WIDE.width + 0.5);
        let height = l.rows as f32 * (l.image_h + WIDE.caption) + (l.rows - 1) as f32 * WIDE.gap;
        assert!(height <= WIDE.height + 0.5);
    }

    #[test]
    fn cells_keep_three_by_two_and_equal_size() {
        for n in 1..=MAX_COUNT {
            let l = layout(n, WIDE);
            assert!((l.image_w / l.image_h - ASPECT).abs() < 1e-3);
            assert!(l.columns * l.rows >= n);
            assert!(l.columns * (l.rows - 1) < n, "no fully empty row for {n}");
        }
    }

    #[test]
    fn round_up_picks_the_next_full_grid_but_respects_available_images() {
        assert!(!is_full(5, WIDE));
        let r = round_up(5, 100, WIDE);
        assert!(r > 5 && is_full(r, WIDE), "{r}");
        // Only five images in the folder: keep what was asked.
        assert_eq!(round_up(5, 5, WIDE), 5);
        // Full values are kept as is.
        assert_eq!(round_up(r, 100, WIDE), r);
        assert_eq!(round_up(1, 0, WIDE), 1);
    }

    #[test]
    fn plus_minus_step_between_full_values_or_by_one() {
        assert_eq!(step(5, true, false, 100, WIDE), 6);
        assert_eq!(step(1, false, false, 100, WIDE), 1);
        assert_eq!(step(MAX_COUNT, true, false, 100, WIDE), MAX_COUNT);
        let mut n = 1;
        let mut seen = vec![n];
        while let Some(next) = Some(step(n, true, true, 1000, WIDE)).filter(|&x| x != n) {
            assert!(is_full(next, WIDE));
            n = next;
            seen.push(n);
        }
        assert!(seen.len() > 4, "{seen:?}");
        for w in seen.windows(2).rev() {
            assert_eq!(step(w[1], false, true, 1000, WIDE), w[0]);
        }
        // Not enough images for a bigger page: + stays.
        assert_eq!(step(6, true, true, 6, WIDE), 6);
    }

    #[test]
    fn effective_size_shows_small_folders_whole_and_rounds_without_editing() {
        // Eight images, default 12: all eight on one page.
        assert_eq!(effective(12, true, 8, Some(WIDE)), 8);
        assert_eq!(effective(12, true, 8, None), 8);
        // Stored 5 with rounding switched on later: rounded immediately.
        let r = effective(5, true, 100, Some(WIDE));
        assert!(r > 5 && is_full(r, WIDE));
        // Rounding off keeps the configured value, also for small folders.
        assert_eq!(effective(5, false, 100, Some(WIDE)), 5);
        assert_eq!(effective(12, false, 8, Some(WIDE)), 12);
        // Empty list and unknown area keep the configured value.
        assert_eq!(effective(12, true, 0, Some(WIDE)), 12);
        assert_eq!(effective(5, true, 100, None), 5);
    }

    #[test]
    fn tall_window_prefers_more_rows() {
        let tall = Area {
            width: 700.,
            height: 1800.,
            gap: 12.,
            caption: 36.,
        };
        let l = layout(6, tall);
        assert!(l.rows > l.columns, "{l:?}");
    }
}
