//! The order of each bar's buttons, and where each one is: on the bar, in
//! its group bar (opened with the » button) or hidden. Settings keep only
//! what was changed from the defaults' order; buttons added in later
//! versions appear at their default place.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Place {
    #[default]
    Bar,
    Group,
    Hidden,
}

impl Place {
    pub const ALL: [Place; 3] = [Place::Bar, Place::Group, Place::Hidden];
    pub fn label(self) -> &'static str {
        match self {
            Place::Bar => "Bar",
            Place::Group => "Group",
            Place::Hidden => "Hidden",
        }
    }
}

/// One button of a saved arrangement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    pub key: String,
    pub place: Place,
}

/// A bar's buttons in order with their places: the saved order and places
/// of known buttons, then the buttons the saved list does not have, each
/// after the default button before it (or at the start).
pub fn arrange(
    defaults: &[(&'static str, Place)],
    saved: Option<&[Slot]>,
) -> Vec<(&'static str, Place)> {
    let Some(saved) = saved else {
        return defaults.to_vec();
    };
    let mut out: Vec<(&'static str, Place)> = saved
        .iter()
        .filter_map(|s| {
            defaults
                .iter()
                .find(|(k, _)| *k == s.key)
                .map(|(k, _)| (*k, s.place))
        })
        .fold(vec![], |mut v, item| {
            if !v.iter().any(|(k, _)| *k == item.0) {
                v.push(item);
            }
            v
        });
    for (i, (key, place)) in defaults.iter().enumerate() {
        if out.iter().any(|(k, _)| k == key) {
            continue;
        }
        let at = defaults[..i]
            .iter()
            .rev()
            .find_map(|(before, _)| out.iter().position(|(k, _)| k == before))
            .map_or(0, |p| p + 1);
        out.insert(at, (key, *place));
    }
    out
}

fn to_slots(items: &[(&'static str, Place)]) -> Vec<Slot> {
    items
        .iter()
        .map(|(key, place)| Slot {
            key: (*key).to_owned(),
            place: *place,
        })
        .collect()
}

/// The arrangement with `key` moved one step earlier (`up`) or later.
pub fn moved(
    defaults: &[(&'static str, Place)],
    saved: Option<&[Slot]>,
    key: &str,
    up: bool,
) -> Vec<Slot> {
    let mut items = arrange(defaults, saved);
    if let Some(i) = items.iter().position(|(k, _)| *k == key) {
        let j = if up { i.checked_sub(1) } else { Some(i + 1) };
        if let Some(j) = j.filter(|j| *j < items.len()) {
            items.swap(i, j);
        }
    }
    to_slots(&items)
}

/// The arrangement with `key` at `place`.
pub fn placed(
    defaults: &[(&'static str, Place)],
    saved: Option<&[Slot]>,
    key: &str,
    place: Place,
) -> Vec<Slot> {
    let mut items = arrange(defaults, saved);
    for item in items.iter_mut().filter(|(k, _)| *k == key) {
        item.1 = place;
    }
    to_slots(&items)
}

#[cfg(test)]
mod tests {
    use super::*;
    const D: &[(&str, Place)] = &[
        ("a", Place::Bar),
        ("b", Place::Bar),
        ("c", Place::Group),
        ("d", Place::Bar),
    ];
    fn keys(v: &[(&'static str, Place)]) -> Vec<&'static str> {
        v.iter().map(|(k, _)| *k).collect()
    }
    #[test]
    fn without_a_saved_list_the_defaults_apply() {
        assert_eq!(arrange(D, None), D.to_vec());
    }
    #[test]
    fn moving_and_placing_keep_every_button_once() {
        let s = moved(D, None, "d", true);
        assert_eq!(keys(&arrange(D, Some(&s))), ["a", "b", "d", "c"]);
        let s = moved(D, Some(&s), "a", true);
        assert_eq!(
            keys(&arrange(D, Some(&s))),
            ["a", "b", "d", "c"],
            "first stays"
        );
        let s = moved(D, Some(&s), "c", false);
        assert_eq!(
            keys(&arrange(D, Some(&s))),
            ["a", "b", "d", "c"],
            "last stays"
        );
        let s = placed(D, Some(&s), "b", Place::Hidden);
        let a = arrange(D, Some(&s));
        assert_eq!(a[1], ("b", Place::Hidden));
        assert_eq!(a[3], ("c", Place::Group));
    }
    #[test]
    fn unknown_saved_keys_go_and_new_buttons_come_after_their_neighbour() {
        // Saved by a version without "b" and with a button since removed.
        let saved = vec![
            Slot {
                key: "d".into(),
                place: Place::Group,
            },
            Slot {
                key: "gone".into(),
                place: Place::Bar,
            },
            Slot {
                key: "a".into(),
                place: Place::Bar,
            },
            Slot {
                key: "c".into(),
                place: Place::Bar,
            },
            Slot {
                key: "a".into(),
                place: Place::Hidden,
            },
        ];
        let a = arrange(D, Some(&saved));
        assert_eq!(keys(&a), ["d", "a", "b", "c"]);
        assert_eq!(a[0].1, Place::Group);
        assert_eq!(a[1].1, Place::Bar, "the first entry of a duplicate wins");
        assert_eq!(a[2].1, Place::Bar, "a new button at its default place");
    }
}
