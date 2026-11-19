//! Animated GIF playback for the big image (the single image view and the
//! image opened from the grid). The normal image pipeline still shows the
//! first frame at once; a background thread decodes the frames, uploads them
//! as textures and the player switches to them once a second frame exists.
//! Grid thumbnails keep the first frame.
use eframe::egui;
use image::AnimationDecoder;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// Browsers show frames with a delay of 10 ms or less (often written by
/// tools that meant "as fast as possible") for 100 ms.
pub fn frame_delay(numer: u32, denom: u32) -> Duration {
    let ms = numer.checked_div(denom).unwrap_or(0);
    if ms <= 10 {
        Duration::from_millis(100)
    } else {
        Duration::from_millis(ms.into())
    }
}

pub fn is_gif(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
}

/// Decodes the composited frames of a GIF in order, giving each with its
/// delay to `frame` (which returns false to stop). Stops with an error before
/// the frames would need more than `budget` bytes of RGBA.
pub fn decode_frames(
    path: &Path,
    budget: usize,
    cancel: &AtomicBool,
    mut frame: impl FnMut(egui::ColorImage, Duration) -> bool,
) -> anyhow::Result<usize> {
    let reader = std::io::BufReader::new(std::fs::File::open(path)?);
    let decoder = image::codecs::gif::GifDecoder::new(reader)?;
    let mut bytes = 0usize;
    let mut count = 0;
    for next in decoder.into_frames() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let next = next?;
        let (numer, denom) = next.delay().numer_denom_ms();
        let buffer = next.into_buffer();
        let size = [buffer.width() as usize, buffer.height() as usize];
        bytes = bytes.saturating_add(size[0] * size[1] * 4);
        anyhow::ensure!(
            bytes <= budget,
            "the animation needs more than {} MB (limit in Settings); showing the first frame",
            budget >> 20
        );
        let color = egui::ColorImage::from_rgba_unmultiplied(size, buffer.as_raw());
        count += 1;
        if !frame(color, frame_delay(numer, denom)) {
            break;
        }
    }
    Ok(count)
}

enum Message {
    Frame(egui::TextureHandle, Duration),
    Done,
    Stopped(String),
}

struct Playback {
    path: PathBuf,
    frames: Vec<(egui::TextureHandle, Duration)>,
    rx: mpsc::Receiver<Message>,
    cancel: Arc<AtomicBool>,
    /// Every frame has arrived.
    complete: bool,
    /// Decoding stopped (too large or an error): the first frame stays.
    stopped: bool,
    index: usize,
    due: Option<Instant>,
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Playback {
    fn receive(&mut self) -> Option<String> {
        let mut note = None;
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::Frame(texture, delay) => self.frames.push((texture, delay)),
                Message::Done => self.complete = true,
                Message::Stopped(error) => {
                    self.stopped = true;
                    self.frames.clear();
                    note = Some(error);
                }
            }
        }
        note
    }
    /// Moves to the frame due at `now`. Returns the time until the next
    /// frame, or `None` while there is nothing to animate.
    fn advance(&mut self, now: Instant) -> Option<Duration> {
        if self.stopped || self.frames.len() < 2 {
            return None;
        }
        let due = self.due.get_or_insert(now + self.frames[0].1);
        // After a long stall (hidden window) start again from now rather
        // than racing through the missed frames.
        if now.saturating_duration_since(*due) > Duration::from_secs(1) {
            *due = now;
        }
        while now >= *due {
            let next = self.index + 1;
            if next < self.frames.len() {
                self.index = next;
            } else if self.complete {
                self.index = 0;
            } else {
                // Waiting for the next frame: the decoder wakes the UI when
                // it arrives; this is only a fallback.
                return Some(Duration::from_millis(100));
            }
            *due += self.frames[self.index].1;
        }
        Some(due.saturating_duration_since(now))
    }
    fn texture(&self) -> Option<egui::TextureId> {
        (!self.stopped && self.frames.len() >= 2).then(|| self.frames[self.index].0.id())
    }
}

#[derive(Default)]
pub struct Player {
    playing: Option<Playback>,
    used: bool,
}
impl Player {
    /// The frame to show now for `path`, or `None` when the normal (first
    /// frame) texture should be shown: not a GIF, a single frame, still
    /// waiting for the second frame, or decoding stopped. The returned note
    /// explains a stop once.
    pub fn frame(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        budget_mb: usize,
        filter: egui::TextureOptions,
        now: Instant,
    ) -> (Option<egui::TextureId>, Option<String>) {
        self.used = true;
        if !is_gif(path) {
            self.playing = None;
            return (None, None);
        }
        if self.playing.as_ref().is_none_or(|p| p.path != path) {
            self.playing = Some(start(ctx, path, budget_mb << 20, filter));
        }
        let playback = self.playing.as_mut().unwrap();
        let note = playback.receive();
        if let Some(wait) = playback.advance(now) {
            ctx.request_repaint_after(wait);
        }
        (playback.texture(), note)
    }
    /// Frees the frames when no big image asked for them this frame.
    pub fn end_frame(&mut self) {
        if !std::mem::take(&mut self.used) {
            self.playing = None;
        }
    }
}

fn start(
    ctx: &egui::Context,
    path: &Path,
    budget: usize,
    filter: egui::TextureOptions,
) -> Playback {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let (thread_ctx, thread_path, thread_cancel) = (ctx.clone(), path.to_owned(), cancel.clone());
    let _ = std::thread::Builder::new()
        .name("gif frames".into())
        .spawn(move || {
            let name = thread_path.to_string_lossy().into_owned();
            let mut index = 0;
            let result = decode_frames(&thread_path, budget, &thread_cancel, |color, delay| {
                let texture = thread_ctx.load_texture(format!("{name}#{index}"), color, filter);
                index += 1;
                // Wake the UI once the second frame makes playback possible.
                if index >= 2 {
                    thread_ctx.request_repaint();
                }
                tx.send(Message::Frame(texture, delay)).is_ok()
            });
            let _ = tx.send(match result {
                Ok(_) => Message::Done,
                Err(error) => Message::Stopped(format!("{error:#}")),
            });
            thread_ctx.request_repaint();
        });
    Playback {
        path: path.to_owned(),
        frames: Vec::new(),
        rx,
        cancel,
        complete: false,
        stopped: false,
        index: 0,
        due: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{codecs::gif::GifEncoder, Delay, Frame, Rgba, RgbaImage};

    fn write_gif(path: &Path, frames: &[(u8, u32)], size: u32) {
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = GifEncoder::new(file);
        encoder
            .set_repeat(image::codecs::gif::Repeat::Infinite)
            .unwrap();
        for (gray, ms) in frames {
            let buffer = RgbaImage::from_pixel(size, size, Rgba([*gray, *gray, *gray, 255]));
            encoder
                .encode_frame(Frame::from_parts(
                    buffer,
                    0,
                    0,
                    Delay::from_numer_denom_ms(*ms, 1),
                ))
                .unwrap();
        }
    }

    #[test]
    fn short_delays_play_like_in_browsers() {
        assert_eq!(frame_delay(0, 1), Duration::from_millis(100));
        assert_eq!(frame_delay(10, 1), Duration::from_millis(100));
        assert_eq!(frame_delay(20, 1), Duration::from_millis(20));
        assert_eq!(frame_delay(70, 1), Duration::from_millis(70));
        assert_eq!(frame_delay(5, 0), Duration::from_millis(100));
        assert!(is_gif(Path::new("a/B.GIF")) && !is_gif(Path::new("a.png")));
    }

    #[test]
    fn frames_decode_in_order_with_their_delays_and_stop_at_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.gif");
        write_gif(&path, &[(0, 50), (128, 120), (255, 0)], 16);
        let cancel = AtomicBool::new(false);
        let mut seen = vec![];
        let n = decode_frames(&path, 1 << 20, &cancel, |color, delay| {
            seen.push((color.pixels[0].r(), delay.as_millis()));
            true
        })
        .unwrap();
        assert_eq!(n, 3);
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0], (0, 50));
        assert_eq!(seen[1].1, 120);
        assert!(seen[1].0 > 100 && seen[1].0 < 160, "{seen:?}");
        assert_eq!(seen[2], (255, 100));
        // Two 16×16 frames fit in 2 KiB, the third does not.
        let mut count = 0;
        let error = decode_frames(&path, 2 * 16 * 16 * 4, &cancel, |_, _| {
            count += 1;
            true
        })
        .unwrap_err();
        assert_eq!(count, 2);
        assert!(error.to_string().contains("showing the first frame"));
    }

    fn playback(ctx: &egui::Context, delays: &[u64], complete: bool) -> Playback {
        let (_, rx) = mpsc::channel();
        let frames = delays
            .iter()
            .map(|ms| {
                let texture = ctx.load_texture(
                    "frame",
                    egui::ColorImage::new([1, 1], egui::Color32::BLACK),
                    egui::TextureOptions::LINEAR,
                );
                (texture, Duration::from_millis(*ms))
            })
            .collect();
        Playback {
            path: "a.gif".into(),
            frames,
            rx,
            cancel: Arc::default(),
            complete,
            stopped: false,
            index: 0,
            due: None,
        }
    }

    #[test]
    fn playback_follows_frame_delays_loops_and_waits_for_missing_frames() {
        let ctx = egui::Context::default();
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut p = playback(&ctx, &[100, 50, 200], true);
        assert_eq!(p.advance(t0), Some(Duration::from_millis(100)));
        assert_eq!(p.index, 0);
        p.advance(ms(99));
        assert_eq!(p.index, 0);
        assert_eq!(p.advance(ms(100)), Some(Duration::from_millis(50)));
        assert_eq!(p.index, 1);
        p.advance(ms(160));
        assert_eq!(p.index, 2);
        // Loops back to the first frame.
        p.advance(ms(350));
        assert_eq!(p.index, 0);
        let first = p.texture();
        p.advance(ms(450));
        assert_ne!(p.texture(), first);
        // Not complete yet: the last arrived frame stays until the next one.
        let mut p = playback(&ctx, &[100, 100], false);
        p.advance(t0);
        p.advance(ms(250));
        assert_eq!(p.index, 1);
        assert!(p.advance(ms(400)).is_some());
        assert_eq!(p.index, 1);
        // One frame, or stopped: nothing to animate.
        let mut single = playback(&ctx, &[100], true);
        assert_eq!(single.advance(t0), None);
        assert_eq!(single.texture(), None);
        let mut stopped = playback(&ctx, &[100, 100], false);
        stopped.stopped = true;
        assert_eq!(stopped.advance(t0), None);
        assert_eq!(stopped.texture(), None);
    }

    #[test]
    fn player_plays_a_real_gif_and_frees_it_when_not_shown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.gif");
        write_gif(&path, &[(0, 40), (255, 40), (90, 40)], 8);
        let ctx = egui::Context::default();
        let mut player = Player::default();
        let filter = egui::TextureOptions::LINEAR;
        let start = Instant::now();
        let end = start + Duration::from_secs(10);
        let mut shown = std::collections::HashSet::new();
        let mut now = start;
        while shown.len() < 3 {
            assert!(Instant::now() < end, "frames never arrived");
            if let (Some(id), _) = player.frame(&ctx, &path, 64, filter, now) {
                shown.insert(id);
            }
            player.end_frame();
            now += Duration::from_millis(20);
            std::thread::yield_now();
        }
        // A still image clears it; so does a frame where nothing asked.
        assert_eq!(
            player.frame(&ctx, Path::new("b.png"), 64, filter, now).0,
            None
        );
        assert!(player.playing.is_none());
        player.frame(&ctx, &path, 64, filter, now);
        player.end_frame();
        assert!(player.playing.is_some());
        player.end_frame();
        assert!(player.playing.is_none());
        // Over the budget: a note and the first frame (no animation texture).
        let big = dir.path().join("big.gif");
        write_gif(&big, &[(0, 40), (255, 40)], 600);
        let mut note = None;
        while note.is_none() {
            assert!(Instant::now() < end, "no note");
            let (id, n) = player.frame(&ctx, &big, 1, filter, Instant::now());
            assert_eq!(id, None);
            note = n;
            std::thread::yield_now();
        }
        assert!(note.unwrap().contains("1 MB"));
    }
}
