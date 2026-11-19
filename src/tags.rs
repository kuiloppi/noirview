//! Palette provenance is user state; learning image keywords never makes them manual.
use crate::model::Library;
use std::collections::BTreeSet;

impl Library {
    pub fn is_manual_tag(&self, tag: &str) -> bool {
        self.manual_tags
            .as_ref()
            .is_some_and(|tags| tags.contains(tag))
    }
    pub fn create_manual_tag(&mut self, tag: String) -> bool {
        let mut changed = self.hidden_tags.remove(&tag);
        changed |= self.tags.insert(tag.clone());
        changed |= self.manual_tags.get_or_insert_default().insert(tag);
        if changed {
            self.tag_revision = self.tag_revision.wrapping_add(1);
        }
        changed
    }
    pub fn learn_tags(&mut self, tags: &BTreeSet<String>) -> bool {
        let before = self.tags.len();
        self.tags.extend(
            tags.iter()
                .filter(|tag| !self.hidden_tags.contains(*tag))
                .cloned(),
        );
        let changed = self.tags.len() != before;
        if changed {
            self.tag_revision = self.tag_revision.wrapping_add(1);
        }
        changed
    }
    pub fn hide_palette_tags(&mut self, selected: &BTreeSet<String>) -> bool {
        let mut changed = false;
        for tag in selected {
            changed |= self.tags.remove(tag);
            changed |= self.hidden_tags.insert(tag.clone());
            if let Some(manual) = &mut self.manual_tags {
                changed |= manual.remove(tag);
            }
        }
        if changed {
            self.tag_revision = self.tag_revision.wrapping_add(1);
        }
        changed
    }
}

pub const CONTROLS: [&str; 6] = [
    "+",
    "Fav./Elite",
    "Additional",
    "Lock",
    "Remove tags",
    "Save keywords",
];
#[derive(Clone, Debug)]
pub struct Choice {
    pub name: String,
    pub additional: bool,
}
#[derive(Clone, PartialEq)]
pub struct RowStyle {
    pub font: eframe::egui::FontId,
    pub padding: eframe::egui::Vec2,
    pub minimum: eframe::egui::Vec2,
    pub gap: f32,
    pub gap_y: f32,
    pub height: f32,
    pub scrollbar: f32,
    pub pixels_per_point: f32,
    pub controls: bool,
}
#[derive(Default)]
pub struct Rows {
    pub ranges: Vec<std::ops::Range<usize>>,
    pub widths: Vec<f32>,
    pub used_width: f32,
    pub fixed: usize,
    pub wrap_width: f32,
    width: f32,
    style: Option<RowStyle>,
}
fn wrap(widths: &[f32], limit: f32, gap: f32) -> (Vec<std::ops::Range<usize>>, f32) {
    let mut rows = vec![];
    let mut start = 0;
    let mut used = 0.0_f32;
    let mut maximum = 0.0_f32;
    for (i, width) in widths.iter().enumerate() {
        let width = width.min(limit);
        let next = if i == start {
            width
        } else {
            used + gap + width
        };
        if i > start && next > limit {
            rows.push(start..i);
            maximum = maximum.max(used);
            start = i;
            used = width;
        } else {
            used = next;
        }
    }
    if start < widths.len() {
        rows.push(start..widths.len());
        maximum = maximum.max(used);
    }
    (rows, maximum)
}
#[derive(Default)]
pub struct TagPicker {
    pub show_additional: bool,
    choices: Vec<Choice>,
    revision: Option<u64>,
    cached_additional: bool,
    pub rows: Rows,
}
impl TagPicker {
    pub fn all() -> Self {
        Self {
            show_additional: true,
            ..Self::default()
        }
    }
    pub fn refresh(&mut self, library: &Library) -> bool {
        if self.revision == Some(library.tag_revision)
            && self.cached_additional == self.show_additional
        {
            return false;
        }
        self.choices = library
            .tags
            .iter()
            .filter_map(|tag| {
                let additional = !library.is_manual_tag(tag);
                (!additional || self.show_additional).then(|| Choice {
                    name: tag.clone(),
                    additional,
                })
            })
            .collect();
        self.revision = Some(library.tag_revision);
        self.cached_additional = self.show_additional;
        self.rows.style = None;
        true
    }
    pub fn arrange(
        &mut self,
        width: f32,
        style: RowStyle,
        mut measure: impl FnMut(&str) -> f32,
    ) -> bool {
        let remeasure = self.rows.style.as_ref().is_none_or(|old| {
            old.font != style.font
                || old.padding.x != style.padding.x
                || old.minimum.x != style.minimum.x
                || old.pixels_per_point != style.pixels_per_point
                || old.controls != style.controls
        });
        if self.rows.style.as_ref() == Some(&style) && self.rows.width == width {
            return false;
        }
        if remeasure {
            self.rows.widths = CONTROLS
                .iter()
                .take(if style.controls { CONTROLS.len() } else { 0 })
                .copied()
                .chain(self.choices.iter().map(|c| c.name.as_str()))
                .map(|s| {
                    (measure(s) + style.padding.x * 2.)
                        .ceil()
                        .max(style.minimum.x)
                })
                .collect();
        }
        let (mut ranges, mut used_width) = wrap(&self.rows.widths, width, style.gap);
        let required_height = ranges.len() as f32 * (style.minimum.y + style.gap_y) - style.gap_y;
        self.rows.wrap_width = if required_height > style.height {
            (width - style.scrollbar).max(1.)
        } else {
            width
        };
        if self.rows.wrap_width != width {
            (ranges, used_width) = wrap(&self.rows.widths, self.rows.wrap_width, style.gap);
        }
        self.rows.fixed = if style.controls {
            ranges
                .iter()
                .take_while(|r| r.start < CONTROLS.len())
                .count()
        } else {
            0
        };
        self.rows.ranges = ranges;
        self.rows.used_width = used_width;
        self.rows.width = width;
        self.rows.style = Some(style);
        true
    }
    pub fn len(&self) -> usize {
        self.choices.len()
    }
    pub fn choices(&self) -> &[Choice] {
        &self.choices
    }
    pub fn restrict_selection(&self, library: &Library, selected: &mut BTreeSet<String>) {
        selected.retain(|tag| {
            library.tags.contains(tag) && (self.show_additional || library.is_manual_tag(tag))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_discovery_manual_creation_promotion_and_hiding_have_distinct_origins() {
        let mut lib = Library::default();
        let tags = BTreeSet::from(["camera".into(), "järvi".into()]);
        assert!(lib.learn_tags(&tags));
        assert!(!lib.is_manual_tag("camera"));
        assert!(lib.create_manual_tag("järvi".into()));
        assert!(!lib.learn_tags(&tags));
        assert!(lib.is_manual_tag("järvi"));
        lib.photos.entry("photo.jpg".into()).or_default().tags = tags.clone();
        lib.hide_palette_tags(&BTreeSet::from(["järvi".into(), "camera".into()]));
        assert!(lib.tags.is_empty());
        assert!(!lib.is_manual_tag("järvi"));
        assert!(!lib.learn_tags(&tags));
        assert_eq!(lib.photos[std::path::Path::new("photo.jpg")].tags, tags);
        lib.create_manual_tag("camera".into());
        assert!(lib.is_manual_tag("camera"));
        assert!(!lib.hidden_tags.contains("camera"));
    }
    #[test]
    fn legacy_palette_stays_available_without_guessing_manual_origins() {
        let lib: Library = serde_json::from_str(
            r#"{"schema":1,"tags":["old imported","old manual"],"hidden_tags":["hidden"]}"#,
        )
        .unwrap();
        assert!(lib.manual_tags.is_none());
        let mut picker = TagPicker::default();
        picker.refresh(&lib);
        assert_eq!(picker.len(), 0);
        picker.show_additional = true;
        picker.refresh(&lib);
        assert_eq!(picker.len(), 2);
        assert!(picker.choices().iter().all(|c| c.additional));
        let restored: Library =
            serde_json::from_str(&serde_json::to_string(&lib.header()).unwrap()).unwrap();
        assert!(restored.manual_tags.as_ref().unwrap().is_empty());
        assert_eq!(restored.tags, lib.tags);
        assert_eq!(restored.hidden_tags, lib.hidden_tags);
    }
    #[test]
    fn complete_unpaged_palette_preserves_origins_and_hidden_selection_safety() {
        let mut lib = Library::default();
        lib.create_manual_tag("own crème brûlée".into());
        lib.learn_tags(&(0..5000).map(|n| format!("imported {n:04}")).collect());
        let mut picker = TagPicker::default();
        assert!(picker.refresh(&lib));
        assert!(!picker.refresh(&lib));
        assert_eq!(picker.len(), 1);
        picker.show_additional = true;
        picker.refresh(&lib);
        assert_eq!(picker.choices().len(), 5001);
        assert!(
            picker
                .choices()
                .iter()
                .find(|c| c.name == "imported 0042")
                .unwrap()
                .additional
        );
        lib.create_manual_tag("imported 0042".into());
        picker.refresh(&lib);
        assert!(
            !picker
                .choices()
                .iter()
                .find(|c| c.name == "imported 0042")
                .unwrap()
                .additional
        );
        let mut selected = BTreeSet::from([
            "imported 0042".into(),
            "imported 0010".into(),
            "stale".into(),
        ]);
        picker.show_additional = false;
        picker.restrict_selection(&lib, &mut selected);
        assert_eq!(selected, BTreeSet::from(["imported 0042".into()]));
        picker.refresh(&lib);
        assert_eq!(picker.len(), 2);
        assert!(picker.choices().iter().all(|c| !c.additional));
    }
    #[test]
    fn natural_width_rows_cover_every_item_and_clamp_oversized_labels() {
        for limit in [1., 100., 312., 652., 1572.] {
            let widths: Vec<_> = (0..5005)
                .map(|i| {
                    if i % 17 == 0 {
                        2000.
                    } else {
                        30. + (i % 9) as f32 * 7.
                    }
                })
                .collect();
            let (rows, used) = wrap(&widths, limit, 8.);
            assert!(used <= limit);
            assert_eq!(
                rows.iter().flat_map(|r| r.clone()).collect::<Vec<_>>(),
                (0..widths.len()).collect::<Vec<_>>()
            );
            for r in rows {
                let sum = r.clone().map(|i| widths[i].min(limit)).sum::<f32>()
                    + (r.len() - 1) as f32 * 8.;
                assert!(sum <= limit);
            }
        }
        assert_eq!(wrap(&[50., 50., 50.], 108., 8.).0, vec![0..2, 2..3]);
    }
    #[test]
    fn changing_row_width_reuses_measurements_and_keeps_controls_in_fixed_bottom_rows() {
        use eframe::egui::{vec2, FontId};
        let mut lib = Library::default();
        lib.learn_tags(&(0..5000).map(|n| format!("word {n}")).collect());
        let mut picker = TagPicker::all();
        picker.refresh(&lib);
        let style = RowStyle {
            font: FontId::proportional(13.),
            padding: vec2(12., 8.),
            minimum: vec2(40., 32.),
            gap: 8.,
            gap_y: 8.,
            height: 400.,
            scrollbar: 16.,
            pixels_per_point: 1.,
            controls: true,
        };
        let mut measured = 0;
        picker.arrange(652., style.clone(), |s| {
            measured += 1;
            s.len() as f32 * 7.
        });
        assert_eq!(measured, 5000 + CONTROLS.len());
        assert!(picker.rows.fixed > 0);
        assert!(picker.rows.ranges[picker.rows.fixed - 1].contains(&(CONTROLS.len() - 1)));
        picker.arrange(312., style.clone(), |_| {
            panic!("width changes must not remeasure all labels")
        });
        assert!(picker.rows.fixed >= 2);
        assert!(picker.rows.used_width <= 312.);
        assert!(!picker.arrange(312., style, |_| panic!("unchanged rows must remain cached")));
    }
}
