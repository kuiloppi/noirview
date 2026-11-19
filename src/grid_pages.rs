//! Random / Shuffle page order for the grid. Pages are remembered so Previous
//! goes back through what was seen; the next page is prepared one step ahead so
//! its thumbnails can be preloaded.
use crate::model::Order;
use rand::seq::SliceRandom;
use std::path::PathBuf;

const HISTORY: usize = 200;

#[derive(Default)]
pub struct Pages {
    pages: Vec<Vec<PathBuf>>,
    cursor: usize,
    /// Shuffle: images not shown yet in this round.
    left: Vec<PathBuf>,
    key: Option<(usize, Option<PathBuf>, usize, Order)>,
    /// Pages dropped from the start of the history: the page number keeps
    /// counting after the history is full.
    dropped: usize,
}

impl Pages {
    /// Start over when the list, the page size or the order changes.
    pub fn sync(&mut self, visible: &[PathBuf], count: usize, order: Order) {
        let key = (visible.len(), visible.first().cloned(), count, order);
        if self.key.as_ref() != Some(&key) {
            *self = Self {
                key: Some(key),
                ..Default::default()
            };
        }
    }
    fn draw(&mut self, visible: &[PathBuf], count: usize, order: Order) -> Vec<PathBuf> {
        let mut rng = rand::thread_rng();
        let count = count.min(visible.len());
        match order {
            Order::Shuffle => {
                let mut page = Vec::with_capacity(count);
                while page.len() < count {
                    if self.left.is_empty() {
                        self.left = visible.to_vec();
                        self.left.shuffle(&mut rng);
                        // A new round must not repeat images already on this page.
                        self.left.retain(|p| !page.contains(p));
                        if self.left.is_empty() {
                            break;
                        }
                    }
                    page.push(self.left.pop().unwrap());
                }
                page
            }
            _ => visible.choose_multiple(&mut rng, count).cloned().collect(),
        }
    }
    fn ensure(&mut self, visible: &[PathBuf], count: usize, order: Order) {
        while self.pages.len() < self.cursor + 2 && !visible.is_empty() {
            let page = self.draw(visible, count, order);
            self.pages.push(page);
        }
        if self.pages.len() > HISTORY {
            let drop = self.pages.len() - HISTORY;
            self.pages.drain(..drop);
            self.cursor = self.cursor.saturating_sub(drop);
            self.dropped += drop;
        }
    }
    /// The page on screen and the prepared next one.
    pub fn current(
        &mut self,
        visible: &[PathBuf],
        count: usize,
        order: Order,
    ) -> (Vec<PathBuf>, Vec<PathBuf>) {
        self.ensure(visible, count, order);
        (
            self.pages.get(self.cursor).cloned().unwrap_or_default(),
            self.pages.get(self.cursor + 1).cloned().unwrap_or_default(),
        )
    }
    pub fn step(&mut self, next: bool) {
        if next {
            self.cursor += 1;
        } else {
            self.cursor = self.cursor.saturating_sub(1);
        }
    }
    pub fn number(&self) -> usize {
        self.dropped + self.cursor + 1
    }
    pub fn at_start(&self) -> bool {
        self.cursor == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_page_number_keeps_counting_past_the_history() {
        let v = files(670);
        let mut p = Pages::default();
        p.sync(&v, 10, Order::Shuffle);
        for _ in 0..(3 * 67 + 5) {
            let _ = p.current(&v, 10, Order::Shuffle);
            p.step(true);
        }
        let _ = p.current(&v, 10, Order::Shuffle);
        assert_eq!(p.number(), 3 * 67 + 6);
        // Shown round by round: 6 / 67 on the fourth round.
        assert_eq!((p.number() - 1) % 67 + 1, 6);
    }
    use std::collections::HashSet;

    fn files(n: usize) -> Vec<PathBuf> {
        (0..n).map(|i| PathBuf::from(format!("{i}"))).collect()
    }

    #[test]
    fn shuffle_shows_every_image_once_per_round_and_previous_returns() {
        let v = files(10);
        let mut p = Pages::default();
        p.sync(&v, 4, Order::Shuffle);
        let mut seen = Vec::new();
        let (first, _) = p.current(&v, 4, Order::Shuffle);
        seen.extend(first.clone());
        p.step(true);
        let (second, prepared) = p.current(&v, 4, Order::Shuffle);
        seen.extend(second.clone());
        p.step(true);
        let (third, _) = p.current(&v, 4, Order::Shuffle);
        assert_eq!(third, prepared, "next page was prepared ahead");
        // First 8 images of the round are distinct.
        assert_eq!(seen.iter().collect::<HashSet<_>>().len(), 8);
        assert!(third.iter().collect::<HashSet<_>>().len() == 4);
        p.step(false);
        assert_eq!(p.current(&v, 4, Order::Shuffle).0, second);
        p.step(false);
        p.step(false);
        assert!(p.at_start());
        assert_eq!(p.current(&v, 4, Order::Shuffle).0, first);
    }

    #[test]
    fn random_pages_are_distinct_within_a_page_and_reset_on_change() {
        let v = files(30);
        let mut p = Pages::default();
        p.sync(&v, 12, Order::Random);
        let (page, _) = p.current(&v, 12, Order::Random);
        assert_eq!(page.iter().collect::<HashSet<_>>().len(), 12);
        p.step(true);
        assert_eq!(p.number(), 2);
        p.sync(&v, 6, Order::Random);
        assert_eq!(p.number(), 1);
        let small = files(3);
        p.sync(&small, 6, Order::Random);
        assert_eq!(p.current(&small, 6, Order::Random).0.len(), 3);
    }
}
