pub use crate::image_queue::Key;
use crate::image_queue::{PrefetchDeferred, Prepared, Request, WorkQueue};
use crate::model::{Settings, Upscale};
use crate::thumbnail_cache::{Kind, ThumbnailCache};
use anyhow::{ensure, Context, Result};
use eframe::egui;
use image::{DynamicImage, GenericImageView, ImageDecoder, RgbaImage};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

/// Thumbnail decoders: half the hardware threads, between 2 and 6. Each one may
/// hold a reduced JPEG decode's working memory (bounded per job).
/// `NOIRVIEW_THUMBNAIL_WORKERS` overrides the count for measurements.
fn thumbnail_workers() -> usize {
    if let Some(n) = std::env::var("NOIRVIEW_THUMBNAIL_WORKERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        return n.clamp(1, 32);
    }
    std::thread::available_parallelism()
        .map_or(2, |n| n.get() / 2)
        .clamp(2, 6)
}
/// Background decoders for full-size images: a quarter of the hardware
/// threads, between 1 and 3 (prefetch fills in parallel; the visible image
/// keeps its own foreground worker). `NOIRVIEW_IMAGE_WORKERS` overrides it.
fn image_workers() -> usize {
    if let Some(n) = std::env::var("NOIRVIEW_IMAGE_WORKERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        return n.clamp(1, 16);
    }
    if cfg!(test) {
        return 1;
    }
    std::thread::available_parallelism()
        .map_or(1, |n| n.get() / 4)
        .clamp(1, 3)
}
/// Work a decoder leaves for its worker to run on the decoded pixels after
/// the result has been delivered (saving a thumbnail of a full image).
type After = Box<dyn FnOnce(&DynamicImage) + Send>;
/// A decoder's result in whatever layout the source produced; the worker
/// turns it into texture pixels in a single pass.
pub struct Decoded {
    pub image: DynamicImage,
    pub dimensions: [u32; 2],
    pub warning: Option<String>,
    after: Option<After>,
}
impl From<(RgbaImage, [u32; 2], Option<String>)> for Decoded {
    fn from((pixels, dimensions, warning): (RgbaImage, [u32; 2], Option<String>)) -> Self {
        Self {
            image: DynamicImage::ImageRgba8(pixels),
            dimensions,
            warning,
            after: None,
        }
    }
}
/// Texture pixels straight from the decoded layout: opaque RGB and gray need
/// no intermediate RGBA copy, and only real alpha is premultiplied.
fn color_image(image: &DynamicImage) -> egui::ColorImage {
    use egui::Color32;
    let size = [image.width() as usize, image.height() as usize];
    let pixels: Vec<Color32> = match image {
        DynamicImage::ImageRgb8(i) => i
            .as_raw()
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| Color32::from_rgb(p[0], p[1], p[2]))
            .collect(),
        DynamicImage::ImageRgba8(i) => i
            .as_raw()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
            .collect(),
        DynamicImage::ImageLuma8(i) => i.as_raw().iter().map(|g| Color32::from_gray(*g)).collect(),
        other => {
            return color_image(&DynamicImage::ImageRgba8(other.to_rgba8()));
        }
    };
    egui::ColorImage { size, pixels }
}
pub struct Texture {
    pub handle: egui::TextureHandle,
    pub dimensions: [u32; 2],
    pub used: u64,
    bytes: usize,
    /// From the worker starting the image until its texture was uploaded.
    work: (Instant, Instant),
}
#[derive(PartialEq, Eq)]
struct Processing {
    upscale: Upscale,
    factor: u32,
    sharpen: u32,
    model: PathBuf,
    runtime: PathBuf,
    tile: u32,
    max_side: u32,
    cache_mb: usize,
}
pub struct Images {
    queue: Arc<WorkQueue>,
    /// Grid thumbnails: small, independent decodes on a wider worker pool so a
    /// page fills in parallel without competing with full-size images.
    thumbs: Arc<WorkQueue>,
    /// Background pass that fills the disk cache for the open folder.
    pregen: Option<Arc<crate::pregen::Pregen>>,
    processing: Option<Processing>,
    pub textures: HashMap<Key, Texture>,
    failures: HashMap<Key, String>,
    pub revision: u64,
    tick: u64,
    pub warning: Option<String>,
    prefetch: Vec<Key>,
    pub disk_cache: Arc<ThumbnailCache>,
    /// Screen-sized first views of large images, in their own database and
    /// budget so they never evict thumbnails.
    pub proxy_cache: Arc<ThumbnailCache>,
}
/// Proxies live next to the thumbnail cache: `<thumbnails>-proxies`.
fn proxy_root(thumbnails: &std::path::Path) -> PathBuf {
    let mut name = thumbnails
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push("-proxies");
    thumbnails.with_file_name(name)
}
/// Makes and stores the grid thumbnail of one image unless the disk cache
/// already has it (background pass and folder watcher; no window needed).
pub fn generate_thumbnail(
    cache: &ThumbnailCache,
    path: &std::path::Path,
    settings: &Settings,
) -> crate::pregen::Outcome {
    use crate::pregen::Outcome;
    let Some(token) = cache.token(path, Kind::Thumbnail) else {
        return Outcome::Failed;
    };
    if cache.contains(&token) {
        return Outcome::Cached;
    }
    // Leave headroom so a pass never evicts its own earlier tiles.
    if cache.stats().bytes >= cache.budget() / 10 * 9 {
        return Outcome::Full;
    }
    let request = Request {
        key: Key {
            path: path.into(),
            thumb: true,
            preview: false,
            revision: 0,
        },
        settings: settings.clone(),
        max_side: 8192,
        cancelled: AtomicBool::new(false),
    };
    let no_ai = Mutex::new(None::<(PathBuf, ort::session::Session)>);
    match decode(&request, &no_ai, None) {
        Ok((pixels, dimensions, _)) => {
            cache.store(token, &pixels, dimensions);
            Outcome::Generated
        }
        Err(_) => Outcome::Failed,
    }
}
impl Images {
    pub fn new(ctx: egui::Context, cache_root: Option<PathBuf>, cache_mb: usize) -> Self {
        let ai = Mutex::new(None::<(PathBuf, ort::session::Session)>);
        let wake = ctx.clone();
        // The proxy budget arrives with the settings (`configure`).
        let proxy_cache = Arc::new(ThumbnailCache::new(
            cache_root.as_deref().map(proxy_root),
            0,
        ));
        let disk_cache = Arc::new(ThumbnailCache::with_wake(cache_root, cache_mb, move || {
            wake.request_repaint()
        }));
        let cache = disk_cache.clone();
        let proxies = proxy_cache.clone();
        let mut images = Self::with_decoder(ctx, move |request, queue| {
            decode_cached(request, &ai, queue, &cache, &proxies)
        });
        let (queue, thumbs, cache) = (
            images.queue.clone(),
            images.thumbs.clone(),
            disk_cache.clone(),
        );
        let idle = move || {
            !queue.busy()
                && !thumbs.busy()
                && crate::disk::grid_demand() == 0
                && cache.stats().pending < 32
                && crate::pregen::quiet()
        };
        let cache = disk_cache.clone();
        let work = move |path: &std::path::Path, settings: &Settings| {
            generate_thumbnail(&cache, path, settings)
        };
        // A full-speed pass can saturate a hard disk and slow the whole system.
        // It runs only in the Aggressive loading mode, as one worker in Windows
        // background mode that rests between images and while the user is
        // active, the window is unfocused or keywords are being indexed.
        images.pregen = Some(crate::pregen::Pregen::start(1, idle, work));
        images.disk_cache = disk_cache;
        images.proxy_cache = proxy_cache;
        images
    }
    fn with_decoder<D: Into<Decoded>>(
        ctx: egui::Context,
        decoder: impl Fn(&Request, &WorkQueue) -> Result<D> + Send + Sync + 'static,
    ) -> Self {
        let background = image_workers();
        // Foreground + background results may run or wait for upload at once.
        let queue = Arc::new(WorkQueue::sized(background + 3));
        let workers = thumbnail_workers();
        let thumbs = Arc::new(WorkQueue::with_slots(workers + 8));
        let decoder = Arc::new(decoder);
        let trace = std::env::var_os("NOIRVIEW_IMAGE_TRACE").is_some();
        let spawn = |queue: &Arc<WorkQueue>, background: bool| {
            let queue = queue.clone();
            let ctx = ctx.clone();
            let decoder = decoder.clone();
            std::thread::spawn(move || {
                // Background decoders yield to the UI thread and to Windows.
                if background {
                    crate::sysinfo::lower_current_thread_priority();
                }
                while let Some(request) = queue.take(background) {
                    let started = Instant::now();
                    if trace {
                        eprintln!("image start: {:?}, thumb={}, preview={}, max_side={}, revision={}, background={background}",request.key.path,request.key.thumb,request.key.preview,request.max_side,request.key.revision);
                    }
                    let small = request.key.thumb || request.key.preview;
                    let job = small.then(crate::decode_stats::Job::begin);
                    let mut after = None;
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let decoded: Decoded = decoder(&request, &queue)?.into();
                        ensure!(!request.cancelled.load(Ordering::Relaxed), "cancelled");
                        // Alpha conversion and the full pixel walk happen on this worker.
                        let convert = Instant::now();
                        let color = color_image(&decoded.image);
                        if small {
                            crate::decode_stats::add(crate::decode_stats::Phase::Convert, convert);
                        }
                        let prepared = Prepared {
                            started,
                            color,
                            dimensions: decoded.dimensions,
                            warning: decoded.warning,
                        };
                        if let Some(work) = decoded.after {
                            after = Some((work, decoded.image));
                        }
                        Ok(prepared)
                    }));
                    let prepared = match result {
                        Ok(Err(e)) if e.is::<PrefetchDeferred>() => {
                            if trace {
                                eprintln!(
                                    "image deferred for output budget: {:?}",
                                    request.key.path
                                );
                            }
                            queue.defer(request, background);
                            ctx.request_repaint();
                            continue;
                        }
                        Ok(result) => result.map_err(|e: anyhow::Error| format!("{e:#}")),
                        Err(_) => Err(
                            "The image processor failed. Check the model and ONNX Runtime version"
                                .into(),
                        ),
                    };
                    if trace {
                        eprintln!(
                            "image prepared: {:?}, preview={}, seconds={:.3}, cancelled={}, ok={}",
                            request.key.path,
                            request.key.preview,
                            started.elapsed().as_secs_f64(),
                            request.cancelled.load(Ordering::Relaxed),
                            prepared.is_ok()
                        );
                    }
                    drop(job);
                    queue.complete(request, prepared, background);
                    ctx.request_repaint();
                    // The image is already on its way to the screen.
                    if let Some((work, image)) = after {
                        let _ =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&image)));
                    }
                }
            });
        };
        spawn(&queue, false);
        for _ in 0..background {
            spawn(&queue, true);
        }
        // One worker keeps the first visible tile moving; the rest share the page.
        for i in 0..workers {
            spawn(&thumbs, i > 0);
        }
        Self {
            queue,
            thumbs,
            pregen: None,
            processing: None,
            textures: HashMap::new(),
            failures: HashMap::new(),
            revision: 0,
            tick: 0,
            warning: None,
            prefetch: Vec::new(),
            disk_cache: Arc::new(ThumbnailCache::new(None, 0)),
            proxy_cache: Arc::new(ThumbnailCache::new(None, 0)),
        }
    }
    pub fn invalidate(&mut self) {
        self.revision += 1;
        self.queue.clear();
        self.thumbs.clear();
        self.textures.clear();
        self.failures.clear();
        self.warning = None;
        self.prefetch.clear();
    }
    pub fn invalidate_paths(&mut self, paths: &[PathBuf]) {
        let paths: HashSet<_> = paths.iter().cloned().collect();
        self.queue.invalidate_paths(&paths);
        self.thumbs.invalidate_paths(&paths);
        self.textures.retain(|k, _| !paths.contains(&k.path));
        self.failures.retain(|k, _| !paths.contains(&k.path));
    }
    /// Start (or replace) the background pass over `paths`, in that order.
    /// Whether the folder watcher inside the open program may work now: the
    /// viewer's own decoders and disk cache are idle, and its own activity
    /// settings allow it (`pregen::watcher_quiet`).
    pub fn background_gate(&self) -> Arc<dyn Fn() -> bool + Send + Sync> {
        let (queue, thumbs, cache) = (
            self.queue.clone(),
            self.thumbs.clone(),
            self.disk_cache.clone(),
        );
        Arc::new(move || {
            !queue.busy()
                && !thumbs.busy()
                && crate::disk::grid_demand() == 0
                && cache.stats().pending < 32
                && crate::pregen::watcher_quiet()
        })
    }
    pub fn pregenerate(&self, paths: Vec<PathBuf>, s: &Settings) {
        if let Some(p) = &self.pregen {
            let enabled = s.loading_mode == crate::model::LoadingMode::Aggressive
                && self.disk_cache.budget() > 0;
            p.set(if enabled { paths } else { Vec::new() }, s, enabled);
        }
    }
    pub fn pregen_progress(&self) -> Option<crate::pregen::Progress> {
        self.pregen.as_ref().and_then(|p| p.progress())
    }
    pub fn stop(&self) {
        if let Some(p) = &self.pregen {
            p.stop();
        }
        self.queue.shutdown();
        self.thumbs.shutdown();
        self.disk_cache.seal();
        self.proxy_cache.seal();
    }
    pub fn configure(&mut self, s: &Settings, max_side: u32) {
        self.disk_cache.configure(s.thumbnail_cache_mb);
        self.proxy_cache
            .configure(if s.loading_mode == crate::model::LoadingMode::Aggressive {
                s.proxy_cache_mb
            } else {
                0
            });
        let processing = Processing {
            upscale: s.upscale,
            factor: s.upscale_factor,
            sharpen: s.sharpen.to_bits(),
            model: s.model_path.clone(),
            runtime: s.ort_library.clone(),
            tile: s.tile_size,
            max_side: max_side.min(8192),
            cache_mb: s.cache_mb,
        };
        if self.processing.as_ref() != Some(&processing) {
            if self.processing.is_some() {
                self.invalidate();
            }
            self.processing = Some(processing);
        }
    }
    pub fn key(&self, path: PathBuf, thumb: bool) -> Key {
        Key {
            path,
            thumb,
            preview: false,
            revision: self.revision,
        }
    }
    pub fn preview_key(&self, path: PathBuf) -> Key {
        Key {
            path,
            thumb: false,
            preview: true,
            revision: self.revision,
        }
    }
    pub fn plan(&mut self, high: &[Key], low: &[Key], s: &Settings, max_side: u32) {
        let mut costs = self.queue.costs();
        costs.extend(self.thumbs.costs());
        let cost = |key: &Key| {
            self.textures
                .get(key)
                .map(|t| t.bytes)
                .or_else(|| costs.get(key).copied())
        };
        let unknown = |key: &Key| {
            let side = if key.thumb {
                512
            } else if key.preview {
                1024
            } else {
                max_side.min(8192)
            } as usize;
            side * side * 4
        };
        let budget = s.cache_mb.max(64) * 1024 * 1024;
        let wanted: HashSet<_> = high.iter().chain(low).cloned().collect();
        let orphan = self.queue.orphan_bytes(&wanted) + self.thumbs.orphan_bytes(&wanted);
        let foreground_bytes: usize = high
            .iter()
            .map(|k| cost(k).unwrap_or_else(|| unknown(k)))
            .sum();
        let mut available = budget.saturating_sub(foreground_bytes.saturating_add(orphan));
        let mut admitted = Vec::new();
        for key in low.iter().take(64) {
            if high.contains(key) || admitted.contains(key) || self.failures.contains_key(key) {
                continue;
            }
            // Unknown candidates can read their headers; the worker must reserve
            // their real output before decoding. Known large outputs wait.
            let bytes = cost(key).unwrap_or(0);
            if bytes <= available {
                available -= bytes;
                admitted.push(key.clone());
            }
        }
        let reserved = orphan
            + high
                .iter()
                .chain(&admitted)
                .filter(|k| !self.textures.contains_key(*k))
                .map(|k| cost(k).unwrap_or_else(|| if high.contains(k) { unknown(k) } else { 0 }))
                .sum::<usize>();
        self.prefetch = admitted;
        self.trim_cache(
            budget.saturating_sub(reserved),
            &high.iter().cloned().collect(),
        );
        let resident = self.textures.values().map(|t| t.bytes).sum();
        self.queue.resident(resident);
        self.thumbs.resident(resident);
        let needed =
            |key: &&Key| !self.textures.contains_key(*key) && !self.failures.contains_key(*key);
        let foreground = high.iter().filter(needed).cloned().collect::<Vec<_>>();
        let background = self
            .prefetch
            .iter()
            .filter(needed)
            .cloned()
            .collect::<Vec<_>>();
        let split = |keys: &[Key], thumb: bool| {
            keys.iter()
                .filter(|k| k.thumb == thumb)
                .cloned()
                .collect::<Vec<_>>()
        };
        crate::disk::set_grid_demand(foreground.iter().filter(|k| k.thumb).count());
        for (queue, thumb) in [(&self.queue, false), (&self.thumbs, true)] {
            queue.plan(
                &split(high, thumb),
                &split(&foreground, thumb),
                &split(&background, thumb),
                s,
                max_side,
            );
            queue.retain_costs(&wanted);
        }
    }
    pub fn poll(&mut self, ctx: &egui::Context, s: &Settings, keep: &HashSet<Key>) {
        self.tick += 1;
        let start = Instant::now();
        let mut uploaded = 0usize;
        const UPLOAD_BYTES: usize = 16 * 1024 * 1024;
        // Full images first (at most 4 per frame); thumbnails are ~1 MiB each,
        // so a whole page can be uploaded within the same time/byte budget.
        let mut taken = 0usize;
        for (queue, limit) in [(self.queue.clone(), 4), (self.thumbs.clone(), 24)] {
            for _ in 0..limit {
                if taken > 0
                    && (uploaded >= UPLOAD_BYTES || start.elapsed() >= Duration::from_millis(4))
                {
                    break;
                }
                let Some(loaded) =
                    queue.take_ready((taken > 0).then(|| UPLOAD_BYTES.saturating_sub(uploaded)))
                else {
                    break;
                };
                taken += 1;
                if loaded.request.key.revision != self.revision {
                    continue;
                }
                match loaded.prepared {
                    Ok(prepared) => {
                        let bytes = prepared.color.pixels.len() * 4;
                        let filter = if s.upscale == Upscale::Nearest {
                            egui::TextureOptions::NEAREST
                        } else {
                            egui::TextureOptions::LINEAR
                        };
                        let handle = ctx.load_texture(
                            loaded.request.key.path.to_string_lossy(),
                            prepared.color,
                            filter,
                        );
                        self.textures.insert(
                            loaded.request.key.clone(),
                            Texture {
                                handle,
                                dimensions: prepared.dimensions,
                                used: self.tick,
                                bytes,
                                work: (prepared.started, Instant::now()),
                            },
                        );
                        uploaded += bytes;
                        if let Some(warning) = prepared.warning {
                            self.warning = Some(warning);
                        }
                    }
                    Err(error) => {
                        // A missing first view is not remembered: the visit
                        // that found none usually stores one, and the queue
                        // already asks only once per visit.
                        if !loaded.request.key.preview {
                            self.failures.insert(loaded.request.key.clone(), error);
                        }
                    }
                }
            }
        }
        if self.queue.has_ready() || self.thumbs.has_ready() {
            ctx.request_repaint();
        }
        let budget = s.cache_mb.max(64) * 1024 * 1024;
        self.trim_cache(budget, keep);
    }
    fn trim_cache(&mut self, budget: usize, keep: &HashSet<Key>) {
        let mut size: usize = self.textures.values().map(|t| t.bytes).sum();
        while size > budget {
            let candidate = self
                .textures
                .iter()
                .filter(|(k, _)| !keep.contains(*k))
                .min_by_key(|(key, v)| {
                    (
                        self.prefetch
                            .iter()
                            .position(|k| k == *key)
                            .map(|i| 64 - i)
                            .unwrap_or(0),
                        v.used,
                    )
                })
                .map(|(k, v)| (k.clone(), v.bytes));
            if let Some((key, bytes)) = candidate {
                self.textures.remove(&key);
                size -= bytes;
            } else {
                break;
            }
        }
    }
    /// Tests: a full-detail texture for `path`, as if it had been loaded.
    #[cfg(test)]
    pub fn insert_full_texture(
        &mut self,
        ctx: &egui::Context,
        path: &std::path::Path,
    ) -> egui::TextureId {
        let handle = ctx.load_texture(
            "test",
            egui::ColorImage::new([4, 4], egui::Color32::RED),
            egui::TextureOptions::LINEAR,
        );
        let id = handle.id();
        let now = Instant::now();
        self.textures.insert(
            self.key(path.into(), false),
            Texture {
                handle,
                dimensions: [4, 4],
                used: self.tick,
                bytes: 64,
                work: (now, now),
            },
        );
        id
    }
    pub fn texture(&mut self, key: &Key) -> Option<(egui::TextureHandle, [u32; 2])> {
        let tick = self.tick;
        self.textures.get_mut(key).map(|t| {
            t.used = tick;
            (t.handle.clone(), t.dimensions)
        })
    }
    pub fn display_texture(
        &mut self,
        path: &std::path::Path,
        previews: bool,
    ) -> Option<(egui::TextureHandle, [u32; 2], bool)> {
        let full = self.key(path.into(), false);
        if let Some((texture, dimensions)) = self.texture(&full) {
            return Some((texture, dimensions, true));
        }
        if previews {
            // The sharper of the grid tile and the preview / proxy.
            let pixels = |t: &Texture| t.bytes;
            let best = [self.key(path.into(), true), self.preview_key(path.into())]
                .into_iter()
                .filter(|k| self.textures.contains_key(k))
                .max_by_key(|k| pixels(&self.textures[k]));
            if let Some((texture, dimensions)) = best.and_then(|k| self.texture(&k)) {
                return Some((texture, dimensions, false));
            }
        }
        None
    }
    /// How long workers were busy with the thumbnails of `paths` (overlapping
    /// work counted once), also when the page was prepared in advance. `None`
    /// until every thumbnail is in memory.
    pub fn prepared_in(&self, paths: &[PathBuf]) -> Option<Duration> {
        let mut spans = paths
            .iter()
            .map(|p| {
                self.textures
                    .get(&self.key(p.clone(), true))
                    .map(|t| t.work)
            })
            .collect::<Option<Vec<_>>>()?;
        spans.sort();
        let mut total = Duration::ZERO;
        let mut open: Option<(Instant, Instant)> = None;
        for (start, end) in spans {
            match &mut open {
                Some((_, last)) if start <= *last => *last = (*last).max(end),
                _ => {
                    if let Some((a, b)) = open.replace((start, end)) {
                        total += b.duration_since(a);
                    }
                }
            }
        }
        if let Some((a, b)) = open {
            total += b.duration_since(a);
        }
        Some(total)
    }
    /// Drop thumbnails and first views held in memory (after the disk caches
    /// were cleared), so the next view is prepared from the source files.
    pub fn forget_small(&mut self) {
        self.thumbs.clear();
        self.textures.retain(|k, _| !k.thumb && !k.preview);
        self.failures.retain(|k, _| !k.thumb && !k.preview);
    }
    pub fn failure(&self, key: &Key) -> Option<&str> {
        self.failures.get(key).map(|s| s.as_str())
    }
    pub fn memory_mb(&self) -> usize {
        self.textures.values().map(|t| t.bytes).sum::<usize>() / (1024 * 1024)
    }
}
impl Drop for Images {
    fn drop(&mut self) {
        if let Some(p) = &self.pregen {
            p.stop();
        }
        self.queue.shutdown();
        self.thumbs.shutdown();
    }
}
/// A proxy is stored for images whose longer side reaches the user's limit
/// (never below the proxy's own size, where it would save nothing).
fn wants_proxy(dimensions: [u32; 2], min_side: u32) -> bool {
    dimensions[0].max(dimensions[1]) >= min_side.max(crate::thumbnail_cache::PROXY_SIDE)
}
/// Reduced copies of a just-decoded full image for the disk caches: the
/// thumbnail (so the grid and the next preview need no second decode) and,
/// in Aggressive loading, a screen-sized proxy of a large image.
fn harvest(
    r: &Request,
    cache: &Arc<ThumbnailCache>,
    proxies: &Arc<ThumbnailCache>,
) -> Option<After> {
    if !matches!(r.settings.upscale, Upscale::Linear | Upscale::Nearest) {
        return None; // Processed pixels are not the source image.
    }
    // Stamps taken before decoding: a file changed meanwhile is not stored.
    let missing = |cache: &Arc<ThumbnailCache>, kind| {
        cache
            .token(&r.key.path, kind)
            .filter(|token| !cache.contains(token))
            .map(|token| (cache.clone(), token))
    };
    let thumbnail = missing(cache, Kind::Thumbnail);
    let proxy = (r.settings.loading_mode == crate::model::LoadingMode::Aggressive)
        .then(|| missing(proxies, Kind::Proxy))
        .flatten();
    if thumbnail.is_none() && proxy.is_none() {
        return None;
    }
    let min_side = r.settings.proxy_min_side;
    Some(Box::new(move |image: &DynamicImage| {
        let dimensions = [image.width(), image.height()];
        let reduce = |cap: u32| {
            let size = crate::resize::fit(dimensions, cap);
            if size == dimensions {
                Ok(image.clone())
            } else {
                crate::resize::lanczos(image.clone(), size)
            }
            .map(DynamicImage::into_rgba8)
        };
        if let Some((cache, token)) = proxy.filter(|_| wants_proxy(dimensions, min_side)) {
            if let Ok(pixels) = reduce(crate::thumbnail_cache::PROXY_SIDE) {
                // Prefetch decodes several large images at once; each proxy is
                // ~15 MiB of pixels until the cache writer has encoded it.
                cache.store_when_room(token, &pixels, dimensions, Duration::from_secs(3));
            }
        }
        if let Some((cache, token)) = thumbnail {
            if let Ok(pixels) = reduce(512) {
                cache.store(token, &pixels, dimensions);
            }
        }
    }))
}
fn decode_cached(
    r: &Request,
    ai: &Mutex<Option<(PathBuf, ort::session::Session)>>,
    queue: &WorkQueue,
    cache: &Arc<ThumbnailCache>,
    proxies: &Arc<ThumbnailCache>,
) -> Result<Decoded> {
    if !r.key.thumb && !r.key.preview {
        let after = harvest(r, cache, proxies);
        let mut decoded = decode_image(r, ai, Some(queue))?;
        decoded.after = after;
        return Ok(decoded);
    }
    // Aggressive loading: a stored screen-sized proxy is the first view.
    if r.key.preview && r.settings.loading_mode == crate::model::LoadingMode::Aggressive {
        if let Some(token) = proxies.token(&r.key.path, Kind::Proxy) {
            if let Some((pixels, dimensions)) =
                proxies.load(&token, |bytes| queue.reserve_output(r, bytes))?
            {
                return Ok((pixels, dimensions, None).into());
            }
        }
    }
    let lookup = Instant::now();
    let thumb = cache.token(&r.key.path, Kind::Thumbnail);
    if let Some(token) = &thumb {
        if let Some((pixels, dimensions)) =
            cache.load(token, |bytes| queue.reserve_output(r, bytes))?
        {
            crate::decode_stats::add(crate::decode_stats::Phase::Cached, lookup);
            return Ok((pixels, dimensions, None).into());
        }
    }
    let token = if r.key.preview {
        cache.token(&r.key.path, Kind::Preview)
    } else {
        thumb
    };
    if r.key.preview {
        if let Some(token) = &token {
            if let Some((pixels, dimensions)) =
                cache.load(token, |bytes| queue.reserve_output(r, bytes))?
            {
                return Ok((pixels, dimensions, None).into());
            }
        }
        anyhow::ensure!(
            r.key
                .path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg")),
            "No cached preview available"
        );
    }
    let result = decode(r, ai, Some(queue));
    if let (Ok((pixels, dimensions, _)), Some(token)) = (&result, token) {
        let store = Instant::now();
        cache.store(token, pixels, *dimensions);
        crate::decode_stats::add(crate::decode_stats::Phase::Store, store);
    }
    result.map(Into::into)
}
/// Decoded pixels as RGBA (thumbnails, previews and the background pass).
fn decode(
    r: &Request,
    ai: &Mutex<Option<(PathBuf, ort::session::Session)>>,
    queue: Option<&WorkQueue>,
) -> Result<(RgbaImage, [u32; 2], Option<String>)> {
    let decoded = decode_image(r, ai, queue)?;
    Ok((
        decoded.image.into_rgba8(),
        decoded.dimensions,
        decoded.warning,
    ))
}
trait Source: std::io::BufRead + std::io::Seek {}
impl<T: std::io::BufRead + std::io::Seek> Source for T {}
/// Largest file decoded from memory; anything bigger is streamed from disk.
const MAX_MEMORY_SOURCE: u64 = 256 * 1024 * 1024;
const MAX_MEMORY_THUMBNAIL_SOURCE: u64 = 64 * 1024 * 1024;
/// The whole file in memory when it is reasonably small: one sequential read
/// (thumbnails take the shared one-at-a-time disk turn) instead of many
/// small reads interleaved with other workers.
fn open_source(r: &Request) -> Result<Box<dyn Source>> {
    let len = std::fs::metadata(&r.key.path)?.len();
    let started = Instant::now();
    let data = if r.key.thumb && len <= MAX_MEMORY_THUMBNAIL_SOURCE {
        Some(crate::disk::read_file(
            &r.key.path,
            MAX_MEMORY_THUMBNAIL_SOURCE,
        )?)
    } else if !r.key.thumb && len <= MAX_MEMORY_SOURCE {
        Some(std::fs::read(&r.key.path)?)
    } else {
        None
    };
    Ok(match data {
        Some(data) => {
            if r.key.thumb {
                crate::decode_stats::add(crate::decode_stats::Phase::Read, started);
            }
            Box::new(std::io::Cursor::new(data))
        }
        None => Box::new(std::io::BufReader::new(std::fs::File::open(&r.key.path)?)),
    })
}
fn decode_image(
    r: &Request,
    ai: &Mutex<Option<(PathBuf, ort::session::Session)>>,
    queue: Option<&WorkQueue>,
) -> Result<Decoded> {
    ensure!(!r.cancelled.load(Ordering::Relaxed), "cancelled");
    let reserve = |bytes| queue.is_none_or(|q| q.reserve_output(r, bytes));
    if r.key.preview {
        match crate::jpeg_preview::decode(&r.key.path, reserve) {
            Ok((pixels, dimensions)) => return Ok((pixels, dimensions, None).into()),
            Err(error) if error.is::<PrefetchDeferred>() => return Err(error),
            Err(_) => {}
        }
        let (pixels, dimensions) = crate::jpeg_scaled::decode(
            &r.key.path,
            512,
            r.settings.cache_mb,
            &r.cancelled,
            reserve,
        )?;
        return Ok((pixels, dimensions, None).into());
    }
    if r.key.thumb
        && r.key
            .path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"))
    {
        match crate::jpeg_scaled::decode(
            &r.key.path,
            512,
            r.settings.cache_mb,
            &r.cancelled,
            reserve,
        ) {
            Ok((pixels, dimensions)) => return Ok((pixels, dimensions, None).into()),
            Err(error) if error.is::<PrefetchDeferred>() => return Err(error),
            Err(error) if r.cancelled.load(Ordering::Relaxed) => return Err(error),
            Err(_) => {} // Unsupported/over-limit JPEGs retain the ordinary thumbnail path.
        }
    }
    let mut reader = image::ImageReader::new(open_source(r)?);
    if let Ok(format) = image::ImageFormat::from_path(&r.key.path) {
        reader.set_format(format);
    }
    let mut reader = reader.with_guessed_format()?;
    // Bound decoder allocations as well as cached textures.
    let mut limits = image::Limits::default();
    limits.max_alloc =
        Some((r.settings.cache_mb.max(64) as u64 * 1024 * 1024).min(2 * 1024 * 1024 * 1024));
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    let cap = if r.key.thumb {
        512
    } else {
        r.max_side.min(8192)
    } as u64;
    let factor = if r.key.thumb {
        1
    } else {
        match r.settings.upscale {
            Upscale::Lanczos | Upscale::Cas => r.settings.upscale_factor.clamp(1, 4),
            Upscale::Onnx => 4,
            _ => 1,
        }
    } as u64;
    let output_bytes =
        ((width as u64 * factor).min(cap) * (height as u64 * factor).min(cap) * 4) as usize;
    if queue.is_some_and(|q| !q.reserve_output(r, output_bytes)) {
        return Err(PrefetchDeferred.into());
    }
    limits.reserve(decoder.total_bytes())?;
    decoder.set_limits(limits)?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let started = Instant::now();
    let mut image = DynamicImage::from_decoder(decoder)?;
    if r.key.thumb {
        crate::decode_stats::add(crate::decode_stats::Phase::Decode, started);
        crate::decode_stats::source([width, height], false);
    }
    ensure!(!r.cancelled.load(Ordering::Relaxed), "cancelled");
    let started = Instant::now();
    image.apply_orientation(orientation);
    let original = [image.width(), image.height()];
    let cap = if r.key.thumb {
        512
    } else {
        r.max_side.min(8192)
    };
    if image.width() > cap || image.height() > cap {
        image = crate::resize::lanczos(image, crate::resize::fit(original, cap))?;
    }
    if r.key.thumb {
        crate::decode_stats::add(crate::decode_stats::Phase::Resize, started);
    }
    let mut warning = None;
    if !r.key.thumb {
        match r.settings.upscale {
            Upscale::Lanczos | Upscale::Cas => {
                let factor = r.settings.upscale_factor.clamp(1, 4);
                let w = (image.width() * factor).min(cap);
                let h = (image.height() * factor).min(cap);
                image = image.resize(w, h, image::imageops::FilterType::Lanczos3);
                if r.settings.upscale == Upscale::Cas {
                    image = DynamicImage::ImageRgba8(adaptive_sharpen(
                        &image.to_rgba8(),
                        r.settings.sharpen,
                    ));
                }
            }
            Upscale::Onnx => match upscale_ai(&image, &r.settings, ai, cap, &r.cancelled) {
                Ok(i) => image = DynamicImage::ImageRgba8(i),
                Err(e) => warning = Some(format!("AI unavailable; showing original: {e:#}")),
            },
            _ => {}
        }
    }
    Ok(Decoded {
        image,
        dimensions: original,
        warning,
        after: None,
    })
}
fn adaptive_sharpen(img: &RgbaImage, strength: f32) -> RgbaImage {
    let mut out = img.clone();
    let (w, h) = img.dimensions();
    if w < 3 || h < 3 {
        return out;
    }
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            for c in 0..3 {
                let center = img[(x, y)][c] as f32;
                let neighbors = [
                    img[(x - 1, y)][c] as f32,
                    img[(x + 1, y)][c] as f32,
                    img[(x, y - 1)][c] as f32,
                    img[(x, y + 1)][c] as f32,
                ];
                let lo = neighbors.iter().fold(center, |a, b| a.min(*b));
                let hi = neighbors.iter().fold(center, |a, b| a.max(*b));
                let adaptive = (1. - (hi - lo) / 255.).clamp(0., 1.);
                let average = neighbors.iter().sum::<f32>() / 4.;
                out[(x, y)][c] = (center + (center - average) * strength.clamp(0., 1.) * adaptive)
                    .clamp(0., 255.) as u8;
            }
        }
    }
    out
}
static ORT_INIT: OnceLock<Result<(), String>> = OnceLock::new();
fn upscale_ai(
    image: &DynamicImage,
    s: &Settings,
    ai: &Mutex<Option<(PathBuf, ort::session::Session)>>,
    cap: u32,
    cancelled: &AtomicBool,
) -> Result<RgbaImage> {
    ensure!(
        s.model_path.is_file(),
        "Choose a compatible .onnx model in Settings"
    );
    ensure!(
        s.ort_library.is_file(),
        "Choose ONNX Runtime 1.22 (DirectML on Windows) in Settings"
    );
    let init = ORT_INIT.get_or_init(|| {
        ort::init_from(s.ort_library.to_string_lossy())
            .with_name("NoirView")
            .commit()
            .map(|_| ())
            .map_err(|e| e.to_string())
    });
    if let Err(e) = init {
        return Err(anyhow::anyhow!(e.clone()));
    }
    ensure!(!cancelled.load(Ordering::Relaxed), "cancelled");
    let mut guard = ai
        .lock()
        .map_err(|_| anyhow::anyhow!("AI session lock failed"))?;
    ensure!(!cancelled.load(Ordering::Relaxed), "cancelled");
    if guard.as_ref().map(|v| &v.0) != Some(&s.model_path) {
        let builder = ort::session::Session::builder()?
            .with_parallel_execution(false)?
            .with_memory_pattern(false)?;
        #[cfg(windows)]
        let builder = builder.with_execution_providers([
            ort::execution_providers::DirectMLExecutionProvider::default()
                .with_device_id(0)
                .build()
                .error_on_failure(),
        ])?;
        *guard = Some((
            s.model_path.clone(),
            builder.commit_from_file(&s.model_path)?,
        ));
    }
    let session = &mut guard.as_mut().context("No AI session")?.1;
    ensure!(
        session.inputs.len() == 1 && session.outputs.len() == 1,
        "Model must have one RGB NCHW input and one RGB NCHW output"
    );
    let tile = s.tile_size.clamp(32, 512);
    let overlap = 8;
    let side = tile + overlap * 2;
    let (w, h) = image.dimensions();
    let rgba = image.to_rgba8();
    let mut output: Option<RgbaImage> = None;
    let mut scale = 0u32;
    for y in (0..h).step_by(tile as usize) {
        for x in (0..w).step_by(tile as usize) {
            ensure!(
                !cancelled.load(Ordering::Relaxed),
                "Superseded by a newer image request"
            );
            let mut values = vec![0f32; 3 * (side * side) as usize];
            for c in 0..3 {
                for ty in 0..side {
                    for tx in 0..side {
                        let sx =
                            (x as i64 + tx as i64 - overlap as i64).clamp(0, w as i64 - 1) as u32;
                        let sy =
                            (y as i64 + ty as i64 - overlap as i64).clamp(0, h as i64 - 1) as u32;
                        values[c * (side * side) as usize + (ty * side + tx) as usize] =
                            rgba[(sx, sy)][c] as f32 / 255.;
                    }
                }
            }
            let tensor = ort::value::Tensor::from_array((
                [1usize, 3, side as usize, side as usize],
                values.into_boxed_slice(),
            ))?;
            let outputs = session.run(ort::inputs![tensor])?;
            let (shape, data) = outputs[0].try_extract_tensor::<f32>()?;
            ensure!(
                shape.len() == 4 && shape[0] == 1 && shape[1] == 3 && shape[2] == shape[3],
                "Model output must be [1,3,H,W]"
            );
            let out_side = shape[2] as u32;
            ensure!(
                out_side.is_multiple_of(side),
                "Model scale must be an integer"
            );
            let factor = out_side / side;
            ensure!(
                (1..=4).contains(&factor),
                "Supported model scales are 1x to 4x"
            );
            if output.is_none() {
                scale = factor;
                ensure!(
                    w * scale <= cap && h * scale <= cap,
                    "AI output exceeds the texture limit; use a smaller source or scale"
                );
                ensure!(
                    w as u64 * h as u64 * scale as u64 * scale as u64 * 4
                        <= s.cache_mb.max(64) as u64 * 1024 * 1024 / 2,
                    "AI output exceeds the memory budget"
                );
                output = Some(RgbaImage::new(w * scale, h * scale));
            }
            ensure!(scale == factor, "Inconsistent model output scale");
            let dest = output.as_mut().unwrap();
            for oy in 0..(h - y).min(tile) * scale {
                for ox in 0..(w - x).min(tile) * scale {
                    let idx = ((oy + overlap * scale) * out_side + ox + overlap * scale) as usize;
                    let mut p = [0u8; 4];
                    for c in 0..3 {
                        p[c] = (data[c * (out_side * out_side) as usize + idx].clamp(0., 1.) * 255.)
                            .round() as u8;
                    }
                    p[3] = rgba[(x + ox / scale, y + oy / scale)][3];
                    dest[(x * scale + ox, y * scale + oy)] = image::Rgba(p);
                }
            }
        }
    }
    output.context("Empty image")
}
#[cfg(test)]
mod tests {
    use super::*;
    fn wait_ready(images: &Images) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !images.queue.has_ready() && !images.thumbs.has_ready() {
            assert!(Instant::now() < deadline, "Worker failed to deliver pixels");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn assert_close(a: &[egui::Color32], b: &[egui::Color32]) {
        assert_eq!(a.len(), b.len());
        let worst = a
            .iter()
            .zip(b)
            .flat_map(|(x, y)| (0..4).map(move |i| x[i].abs_diff(y[i])))
            .max()
            .unwrap_or(0);
        assert!(worst <= 8, "cached thumbnail differs by {worst}");
    }
    fn wait_writes(cache: &ThumbnailCache, writes: u64) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while cache.stats().writes < writes {
            assert!(Instant::now() < deadline, "thumbnail was not stored");
            cache.flush();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(cache.stats().writes, writes);
    }
    #[test]
    fn aggressive_mode_caches_the_folder_in_the_background_and_pauses_when_blocked() {
        let _gates = crate::pregen::GATE_TEST
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<PathBuf> = (0..6u32)
            .map(|n| {
                let path = dir.path().join(format!("{n}.png"));
                image::RgbImage::from_fn(700, 500, |x, y| {
                    image::Rgb([(x / 3) as u8, (y / 2) as u8, (n * 40) as u8])
                })
                .save(&path)
                .unwrap();
                path
            })
            .collect();
        let images = Images::new(
            egui::Context::default(),
            Some(dir.path().join("cache")),
            512,
        );
        // Standard: nothing happens in the background.
        images.pregenerate(paths.clone(), &Settings::default());
        std::thread::sleep(Duration::from_millis(150));
        assert!(images.pregen_progress().is_none());
        let aggressive = Settings {
            loading_mode: crate::model::LoadingMode::Aggressive,
            ..Default::default()
        };
        // Blocked (unfocused window / indexing): the pass waits.
        crate::pregen::set_blocked(true);
        images.pregenerate(paths.clone(), &aggressive);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(images.pregen_progress().unwrap().done, 0);
        crate::pregen::set_blocked(false);
        let deadline = Instant::now() + Duration::from_secs(20);
        while images.pregen_progress().unwrap().done < 6 {
            assert!(Instant::now() < deadline, "background pass did not finish");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(images.pregen_progress().unwrap().generated, 6);
        wait_writes(&images.disk_cache, 6);
        // A second pass finds everything cached.
        images.pregenerate(paths, &aggressive);
        let deadline = Instant::now() + Duration::from_secs(20);
        while images.pregen_progress().unwrap().done < 6 {
            assert!(Instant::now() < deadline, "second pass did not finish");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(images.pregen_progress().unwrap().generated, 0);
    }
    #[test]
    fn aggressive_mode_keeps_a_screen_sized_first_view_of_a_large_image() {
        let dir = tempfile::tempdir().unwrap();
        let large = dir.path().join("large.png");
        image::RgbImage::from_fn(4000, 2000, |x, y| {
            image::Rgb([(x / 16) as u8, (y / 8) as u8, ((x + y) / 24) as u8])
        })
        .save(&large)
        .unwrap();
        let small = dir.path().join("small.png");
        image::RgbImage::from_fn(3000, 1500, |x, y| image::Rgb([x as u8, y as u8, 7]))
            .save(&small)
            .unwrap();
        let ctx = egui::Context::default();
        let aggressive = Settings {
            loading_mode: crate::model::LoadingMode::Aggressive,
            ..Default::default()
        };
        let view = |root: &std::path::Path, settings: &Settings, path: &PathBuf, preview: bool| {
            let mut images = Images::new(ctx.clone(), Some(root.to_path_buf()), 512);
            images.configure(settings, 8192);
            let key = if preview {
                images.preview_key(path.clone())
            } else {
                images.key(path.clone(), false)
            };
            images.plan(std::slice::from_ref(&key), &[], settings, 8192);
            wait_ready(&images);
            let ready = images.queue.take_ready(None).unwrap().prepared.unwrap();
            // Let the worker finish storing before the caches are inspected.
            wait_writes(&images.disk_cache, u64::from(!preview));
            images.proxy_cache.flush();
            (ready.color.size, ready.dimensions, images)
        };
        // Standard: a thumbnail only, and the next preview is that thumbnail.
        let standard_root = dir.path().join("standard").join("thumbnails");
        let (_, _, images) = view(&standard_root, &Settings::default(), &large, false);
        assert_eq!(images.proxy_cache.stats().writes, 0);
        drop(images);
        let (size, _, images) = view(&standard_root, &Settings::default(), &large, true);
        assert_eq!(size, [512, 256]);
        drop(images);
        // Aggressive: the large image leaves a proxy, the smaller one does not.
        let root = dir.path().join("aggressive").join("thumbnails");
        let (size, _, images) = view(&root, &aggressive, &large, false);
        assert_eq!(size, [4000, 2000]);
        let deadline = Instant::now() + Duration::from_secs(10);
        while images.proxy_cache.stats().writes < 1 {
            assert!(Instant::now() < deadline, "proxy was not stored");
            images.proxy_cache.flush();
            std::thread::sleep(Duration::from_millis(2));
        }
        drop(images);
        let (_, _, images) = view(&root, &aggressive, &small, false);
        assert_eq!(images.proxy_cache.stats().writes, 0);
        drop(images);
        assert!(root.with_file_name("thumbnails-proxies").is_dir());
        // A new session: the first view comes from the proxy at screen size.
        let (size, dimensions, images) = view(&root, &aggressive, &large, true);
        assert_eq!(size, [2560, 1280]);
        assert_eq!(dimensions, [4000, 2000]);
        assert_eq!(images.proxy_cache.stats().hits, 1);
        drop(images);
        // Back in Standard the stored proxy is not used.
        let (size, _, _) = view(&root, &Settings::default(), &large, true);
        assert_eq!(size, [512, 256]);
        assert!(wants_proxy([3400, 100], 3328) && !wants_proxy([3300, 3300], 3328));
        assert!(wants_proxy([2560, 100], 0) && !wants_proxy([2559, 2559], 0));
    }
    #[test]
    fn the_sharper_of_tile_and_preview_is_shown_until_the_full_image_arrives() {
        let ctx = egui::Context::default();
        let mut images = Images::with_decoder(ctx.clone(), |_, _| Ok(sample()));
        let path = PathBuf::from("a.png");
        let insert = |images: &mut Images, key: Key, side: usize| {
            let handle = ctx.load_texture(
                "t",
                egui::ColorImage::new([side, side], egui::Color32::RED),
                egui::TextureOptions::LINEAR,
            );
            images.textures.insert(
                key,
                Texture {
                    handle,
                    dimensions: [4000, 4000],
                    used: 0,
                    bytes: side * side * 4,
                    work: (Instant::now(), Instant::now()),
                },
            );
        };
        let (thumb, preview) = (
            images.key(path.clone(), true),
            images.preview_key(path.clone()),
        );
        insert(&mut images, thumb.clone(), 8);
        let tile = images.display_texture(&path, true).unwrap().0.id();
        insert(&mut images, preview, 32);
        let proxy = images.display_texture(&path, true).unwrap();
        assert_ne!(proxy.0.id(), tile);
        assert!(!proxy.2);
        assert!(images.display_texture(&path, false).is_none());
    }
    /// Browsing a folder of large images stores a first view for each, end
    /// to end (a fast browser once outran the cache writer). Only a release
    /// build decodes fast enough to overrun the cache writer;
    /// the mechanism itself is covered by the thumbnail cache's own test.
    #[test]
    #[ignore = "manual: cargo test --release browsing_a_folder -- --ignored"]
    fn browsing_a_folder_with_prefetch_stores_a_first_view_for_every_large_image() {
        const COUNT: usize = 10;
        let dir = tempfile::tempdir().unwrap();
        // BMP: trivial to decode, so the test stays light and the burst of
        // decoded images is as dense as prefetch can make it.
        let source = dir.path().join("0.bmp");
        image::RgbImage::from_fn(3400, 1900, |x, y| {
            image::Rgb([(x / 16) as u8, (y / 9) as u8, ((x ^ y) & 255) as u8])
        })
        .save(&source)
        .unwrap();
        let paths: Vec<PathBuf> = (0..COUNT)
            .map(|n| {
                let path = dir.path().join(format!("{n}.bmp"));
                if n > 0 {
                    std::fs::copy(&source, &path).unwrap();
                }
                path
            })
            .collect();
        let settings = Settings {
            loading_mode: crate::model::LoadingMode::Aggressive,
            cache_mb: 2048,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        ctx.input_mut(|i| i.max_texture_side = 8192);
        let mut images = Images::new(ctx.clone(), Some(dir.path().join("thumbnails")), 512);
        images.configure(&settings, 8192);
        // Step through the folder like the viewer: current image plus look-ahead.
        for i in 0..paths.len() {
            let high = [images.key(paths[i].clone(), false)];
            let low: Vec<Key> = paths[i + 1..]
                .iter()
                .take(7)
                .map(|p| images.key(p.clone(), false))
                .collect();
            let keep: HashSet<Key> = high.iter().cloned().collect();
            let deadline = Instant::now() + Duration::from_secs(60);
            while !images.textures.contains_key(&high[0]) {
                assert!(Instant::now() < deadline, "image {i} did not load");
                images.plan(&high, &low, &settings, 8192);
                images.poll(&ctx, &settings, &keep);
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let deadline = Instant::now() + Duration::from_secs(60);
        while images.proxy_cache.stats().writes < COUNT as u64 {
            assert!(
                Instant::now() < deadline,
                "only {} of {COUNT} first views stored, {} skipped",
                images.proxy_cache.stats().writes,
                images.proxy_cache.stats().skipped
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(images.proxy_cache.stats().skipped, 0);
    }
    #[test]
    fn texture_pixels_match_the_two_pass_conversion_for_every_layout() {
        let rgba = RgbaImage::from_fn(37, 21, |x, y| {
            image::Rgba([(x * 7) as u8, (y * 12) as u8, (x + y) as u8, (x * y) as u8])
        });
        let source = DynamicImage::ImageRgba8(rgba);
        for image in [
            source.clone(),
            DynamicImage::ImageRgb8(source.to_rgb8()),
            DynamicImage::ImageLuma8(source.to_luma8()),
            DynamicImage::ImageLumaA8(source.to_luma_alpha8()),
            DynamicImage::ImageRgb16(source.to_rgb16()),
            DynamicImage::ImageRgba16(source.to_rgba16()),
        ] {
            let expected =
                egui::ColorImage::from_rgba_unmultiplied([37, 21], image.to_rgba8().as_raw());
            let actual = color_image(&image);
            assert_eq!(actual.size, [37, 21]);
            assert_eq!(actual.pixels, expected.pixels, "{:?}", image.color());
        }
    }
    #[test]
    fn full_png_decode_leaves_a_thumbnail_that_serves_the_grid_and_the_next_preview() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.png");
        image::RgbImage::from_fn(1400, 700, |x, y| {
            image::Rgb([(x / 6) as u8, (y / 3) as u8, ((x + y) / 9) as u8])
        })
        .save(&path)
        .unwrap();
        let root = dir.path().join("cache");
        let settings = Settings::default();
        let ctx = egui::Context::default();
        {
            let mut images = Images::new(ctx.clone(), Some(root.clone()), 512);
            let full = images.key(path.clone(), false);
            images.plan(std::slice::from_ref(&full), &[], &settings, 8192);
            wait_ready(&images);
            let ready = images.queue.take_ready(None).unwrap().prepared.unwrap();
            assert_eq!(ready.color.size, [1400, 700]);
            assert_eq!(
                ready.color.pixels[350 * 1400 + 700],
                egui::Color32::from_rgb((700 / 6) as u8, (350 / 3) as u8, (1050 / 9) as u8)
            );
            wait_writes(&images.disk_cache, 1);
        }
        // A new session: the PNG has a preview and a grid tile without decoding.
        let mut images = Images::new(ctx, Some(root), 512);
        let preview = images.preview_key(path.clone());
        images.plan(std::slice::from_ref(&preview), &[], &settings, 8192);
        wait_ready(&images);
        let ready = images.queue.take_ready(None).unwrap().prepared.unwrap();
        assert_eq!(ready.color.size, [512, 256]);
        assert_eq!(ready.dimensions, [1400, 700]);
        let thumb = images.key(path, true);
        images.plan(std::slice::from_ref(&thumb), &[], &settings, 8192);
        wait_ready(&images);
        let ready = images.thumbs.take_ready(None).unwrap().prepared.unwrap();
        assert_eq!(ready.color.size, [512, 256]);
        assert_eq!(images.disk_cache.stats().hits, 2);
        assert_eq!(images.disk_cache.stats().writes, 0);
    }
    #[test]
    fn png_thumbnail_is_resized_from_memory_and_a_wrong_extension_still_decodes() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("wide.png");
        let rgba = RgbaImage::from_fn(900, 300, |x, y| {
            image::Rgba([
                (x / 4) as u8,
                (y / 2) as u8,
                40,
                if x < 450 { 255 } else { 0 },
            ])
        });
        rgba.save(&png).unwrap();
        // PNG bytes behind a .bmp name: the content decides.
        let renamed = dir.path().join("actually-png.bmp");
        std::fs::copy(&png, &renamed).unwrap();
        for path in [png, renamed] {
            let request = Request {
                key: Key {
                    path,
                    thumb: true,
                    preview: false,
                    revision: 0,
                },
                settings: Settings::default(),
                max_side: 8192,
                cancelled: AtomicBool::new(false),
            };
            let (pixels, dimensions, _) = decode(&request, &Mutex::new(None), None).unwrap();
            assert_eq!(dimensions, [900, 300]);
            assert_eq!(pixels.dimensions(), (512, 171));
            assert_eq!(pixels.get_pixel(100, 85)[3], 255);
            assert_eq!(pixels.get_pixel(400, 85)[3], 0);
            let reference = DynamicImage::ImageRgba8(rgba.clone())
                .resize_exact(512, 171, image::imageops::FilterType::Lanczos3)
                .into_rgba8();
            let p = pixels.get_pixel(100, 85);
            let q = reference.get_pixel(100, 85);
            assert!((0..3).all(|c| p[c].abs_diff(q[c]) <= 3), "{p:?} vs {q:?}");
        }
    }
    fn sample() -> (RgbaImage, [u32; 2], Option<String>) {
        (
            RgbaImage::from_pixel(2, 3, image::Rgba([100, 150, 200, 128])),
            [2, 3],
            None,
        )
    }
    #[test]
    fn disk_thumbnail_survives_new_workers_and_can_preview_a_non_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.bmp");
        let root = dir.path().join("thumbnails");
        image::RgbImage::from_fn(80, 120, |x, y| image::Rgb([x as u8, y as u8, 90]))
            .save(&path)
            .unwrap();
        let settings = Settings::default();
        let ctx = egui::Context::default();
        let expected = {
            let mut images = Images::new(ctx.clone(), Some(root.clone()), 512);
            let thumb = images.key(path.clone(), true);
            images.plan(std::slice::from_ref(&thumb), &[], &settings, 8192);
            wait_ready(&images);
            let prepared = images.thumbs.take_ready(None).unwrap().prepared.unwrap();
            images.disk_cache.flush();
            assert_eq!(images.disk_cache.stats().writes, 1);
            prepared
        };
        let mut images = Images::new(ctx, Some(root), 512);
        let preview = images.preview_key(path);
        images.plan(std::slice::from_ref(&preview), &[], &settings, 8192);
        wait_ready(&images);
        let actual = images.queue.take_ready(None).unwrap().prepared.unwrap();
        assert_eq!(actual.dimensions, [80, 120]);
        // Opaque thumbnails return from the disk cache as JPEG: close, not exact.
        assert_close(&actual.color.pixels, &expected.color.pixels);
        assert_eq!(images.disk_cache.stats().hits, 1);
        assert_eq!(images.disk_cache.stats().writes, 0);
    }
    fn collect_grid(images: &mut Images, paths: &[PathBuf]) -> HashMap<PathBuf, Prepared> {
        let settings = Settings::default();
        let keys: Vec<_> = paths.iter().map(|p| images.key(p.clone(), true)).collect();
        images.plan(&keys, &[], &settings, 8192);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut ready = HashMap::new();
        while ready.len() < paths.len() {
            if let Some(item) = images.thumbs.take_ready(None) {
                ready.insert(item.request.key.path.clone(), item.prepared.unwrap());
            } else {
                assert!(Instant::now() < deadline, "grid did not finish");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        ready
    }
    #[test]
    fn all_twelve_grid_tiles_survive_restart_and_one_changed_source_is_regenerated() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("cache");
        let paths: Vec<_> = (0..12)
            .map(|n| {
                let path = dir.path().join(format!("{n}.bmp"));
                image::RgbImage::from_pixel(80, 120, image::Rgb([n * 17, 40, 90]))
                    .save(&path)
                    .unwrap();
                path
            })
            .collect();
        let ctx = egui::Context::default();
        let mut images = Images::new(ctx.clone(), Some(root.clone()), 64);
        let cold = collect_grid(&mut images, &paths);
        images.stop();
        images.disk_cache.flush();
        assert_eq!(images.disk_cache.stats().writes, 12);
        assert_eq!(images.disk_cache.stats().skipped, 0);
        let mut warm = Images::new(ctx.clone(), Some(root.clone()), 64);
        let loaded = collect_grid(&mut warm, &paths);
        for path in &paths {
            assert_close(&loaded[path].color.pixels, &cold[path].color.pixels);
        }
        warm.disk_cache.flush();
        assert_eq!(warm.disk_cache.stats().hits, 12);
        assert_eq!(warm.disk_cache.stats().writes, 0);
        // An external editor changes one source at the same path. Invalidate its
        // GPU/failure/request variants while retaining an unrelated texture.
        let unrelated = warm.key(paths[1].clone(), true);
        insert_texture(&mut warm, &ctx, unrelated.clone());
        let edited = warm.key(paths[0].clone(), true);
        insert_texture(&mut warm, &ctx, edited.clone());
        let preview = warm.preview_key(paths[0].clone());
        warm.failures.insert(preview, "old decode failure".into());
        image::RgbImage::from_pixel(120, 80, image::Rgb([200, 100, 20]))
            .save(&paths[0])
            .unwrap();
        warm.invalidate_paths(&paths[..1]);
        assert!(warm.textures.contains_key(&unrelated));
        assert!(!warm.textures.contains_key(&edited));
        assert!(warm.failures.is_empty());
        let updated = collect_grid(&mut warm, &paths[..1]);
        assert_eq!(updated[&paths[0]].dimensions, [120, 80]);
        assert_ne!(
            updated[&paths[0]].color.pixels,
            cold[&paths[0]].color.pixels
        );
        warm.disk_cache.flush();
        assert_eq!(warm.disk_cache.stats().writes, 1);
    }
    fn insert_texture(images: &mut Images, ctx: &egui::Context, key: Key) {
        let color = egui::ColorImage::new([2, 2], egui::Color32::RED);
        images.textures.insert(
            key,
            Texture {
                handle: ctx.load_texture("test", color, egui::TextureOptions::LINEAR),
                dimensions: [2, 2],
                used: 0,
                bytes: 16,
                work: (Instant::now(), Instant::now()),
            },
        );
    }
    #[test]
    fn page_preparation_time_counts_overlapping_work_once_and_clearing_forgets_small_textures() {
        let ctx = egui::Context::default();
        let mut images = Images::with_decoder(ctx.clone(), |_, _| Ok(sample()));
        let base = Instant::now();
        let at = |ms: u64| base + Duration::from_millis(ms);
        let paths: Vec<PathBuf> = ["a", "b", "c"].iter().map(PathBuf::from).collect();
        let texture = |work| Texture {
            handle: ctx.load_texture(
                "test",
                egui::ColorImage::new([2, 2], egui::Color32::RED),
                egui::TextureOptions::LINEAR,
            ),
            dimensions: [2, 2],
            used: 0,
            bytes: 16,
            work,
        };
        // Two workers overlap (0-40 and 30-70), a third tile came later (200-210).
        for (path, work) in paths.iter().zip([(at(0), at(40)), (at(30), at(70))]) {
            images
                .textures
                .insert(images.key(path.clone(), true), texture(work));
        }
        assert_eq!(images.prepared_in(&paths), None);
        images.textures.insert(
            images.key(paths[2].clone(), true),
            texture((at(200), at(210))),
        );
        assert_eq!(images.prepared_in(&paths), Some(Duration::from_millis(80)));
        assert_eq!(images.prepared_in(&[]), Some(Duration::ZERO));
        // Clearing the cache keeps full images and forgets everything reduced.
        let full = images.key(paths[0].clone(), false);
        let preview = images.preview_key(paths[0].clone());
        images
            .textures
            .insert(full.clone(), texture((at(0), at(1))));
        images
            .textures
            .insert(preview.clone(), texture((at(0), at(1))));
        images
            .failures
            .insert(images.preview_key(paths[1].clone()), "none".into());
        images.forget_small();
        assert_eq!(images.textures.keys().collect::<Vec<_>>(), [&full]);
        assert!(images.failures.is_empty());
        assert_eq!(images.prepared_in(&paths), None);
    }
    #[test]
    fn a_missing_first_view_is_asked_again_on_the_next_visit() {
        let stored = Arc::new(AtomicBool::new(false));
        let available = stored.clone();
        let ctx = egui::Context::default();
        let mut images = Images::with_decoder(ctx.clone(), move |request, _| {
            anyhow::ensure!(
                !request.key.preview || available.load(Ordering::Relaxed),
                "no first view stored yet"
            );
            Ok(sample())
        });
        let path = PathBuf::from("large.png");
        let full = images.key(path.clone(), false);
        let preview = images.preview_key(path.clone());
        let demand = [full.clone(), preview.clone()];
        let keep: HashSet<Key> = demand.iter().cloned().collect();
        let settings = Settings::default();
        let settle = |images: &mut Images, demand: &[Key], wanted: &Key| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while images.texture(wanted).is_none() || images.queue.busy() {
                assert!(Instant::now() < deadline, "image did not arrive");
                images.plan(demand, &[], &settings, 8192);
                images.poll(&ctx, &settings, &keep);
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        // First visit: no first view exists; the full decode stores one.
        settle(&mut images, &demand, &full);
        assert!(images.texture(&preview).is_none());
        assert!(images.failure(&preview).is_none());
        stored.store(true, Ordering::Relaxed);
        // Another image, then back after the full image left memory.
        let other = [images.key("other.png".into(), false)];
        settle(&mut images, &other, &other[0]);
        images.textures.remove(&full);
        settle(&mut images, &demand, &preview);
    }
    #[test]
    fn preview_arrives_while_full_decode_is_blocked_and_can_never_replace_full_detail() {
        use crossbeam_channel::bounded;
        for preview_first in [true, false] {
            let (release_tx, release_rx) = bounded(1);
            let ctx = egui::Context::default();
            let mut images = Images::with_decoder(ctx.clone(), move |request, _| {
                if request.key.preview != preview_first {
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                Ok(sample())
            });
            let path = std::path::Path::new("sample.jpg");
            let full = images.key(path.into(), false);
            let preview = images.preview_key(path.into());
            assert_ne!(full, preview);
            assert_ne!(preview, images.key(path.into(), true));
            let demand = [full.clone(), preview.clone()];
            let keep = demand.iter().cloned().collect();
            let settings = Settings::default();
            images.plan(&demand, &[], &settings, 8192);
            wait_ready(&images);
            images.poll(&ctx, &settings, &keep);
            assert_eq!(
                images.display_texture(path, true).unwrap().2,
                !preview_first
            );
            if preview_first {
                assert!(images.texture(&full).is_none());
                assert!(images.display_texture(path, false).is_none());
            }
            release_tx.send(()).unwrap();
            wait_ready(&images);
            images.poll(&ctx, &settings, &keep);
            let full_texture = images.texture(&full).unwrap().0.id();
            assert_eq!(
                images.display_texture(path, true).unwrap().0.id(),
                full_texture
            );
            assert!(images.display_texture(path, true).unwrap().2);
            assert!(images.texture(&preview).is_some());
            images.invalidate();
            assert!(images.display_texture(path, true).is_none());
        }
    }
    #[test]
    fn real_header_defers_a_large_prefetch_before_decode_and_allows_foreground_error_reporting() {
        use crossbeam_channel::bounded;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.bmp");
        let mut data = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image::RgbImage::new(1, 1))
            .write_to(&mut data, image::ImageFormat::Bmp)
            .unwrap();
        let mut bytes = data.into_inner();
        bytes[18..22].copy_from_slice(&10000u32.to_le_bytes());
        bytes[22..26].copy_from_slice(&10000u32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let settings = Settings {
            cache_mb: 64,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let ai = Mutex::new(None);
        let (deferred_tx, deferred_rx) = bounded(1);
        let mut images = Images::with_decoder(ctx.clone(), move |r, q| {
            let result = decode(r, &ai, Some(q));
            if result.as_ref().is_err_and(|e| e.is::<PrefetchDeferred>()) {
                deferred_tx.send(()).unwrap();
            }
            result
        });
        let key = images.key(path, false);
        images.plan(&[], std::slice::from_ref(&key), &settings, 8192);
        deferred_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        images.plan(&[], std::slice::from_ref(&key), &settings, 8192);
        assert!(images.prefetch.is_empty());
        assert!(images.failure(&key).is_none());
        assert_eq!(images.queue.costs()[&key], 8192 * 8192 * 4);
        images.plan(std::slice::from_ref(&key), &[], &settings, 8192);
        wait_ready(&images);
        images.poll(&ctx, &settings, &HashSet::from([key.clone()]));
        assert!(images.failure(&key).is_some());
        assert!(!images.failure(&key).unwrap().contains("waiting for memory"));
    }
    #[test]
    fn real_workers_fill_a_deeper_horizon_within_output_budget_and_prioritize_a_new_neighbor() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut images = Images::new(ctx.clone(), None, 0);
        let settings = Settings {
            cache_mb: 64,
            ..Default::default()
        };
        let mut keys = Vec::new();
        for i in 0..12 {
            let path = dir.path().join(format!("{i}.bmp"));
            image::RgbImage::from_pixel(2048, 1024, image::Rgb([i * 16, 80, 160]))
                .save(&path)
                .unwrap();
            keys.push(images.key(path, false));
        }
        let high = [keys[0].clone()];
        let keep = HashSet::from([keys[0].clone()]);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            images.plan(&high, &keys[1..], &settings, 8192);
            images.poll(&ctx, &settings, &keep);
            assert!(images.memory_mb() <= 64);
            if images.textures.len() == 8 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "horizon did not fill: {} textures",
                images.textures.len()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        // The nearest new target displaces a farther cached image, even if the
        // old window already uses the entire output allowance.
        let nearest = keys[11].clone();
        let mut low = vec![nearest.clone()];
        low.extend_from_slice(&keys[1..11]);
        loop {
            images.plan(&high, &low, &settings, 8192);
            images.poll(&ctx, &settings, &keep);
            if images.textures.contains_key(&nearest) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "new nearest target was starved by cached far targets"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(images.texture(&keys[0]).is_some());
        assert!(images.memory_mb() <= 64);
        images.plan(&high, &[], &settings, 8192);
        assert!(images.prefetch.is_empty());
    }
    #[test]
    fn real_workers_reuse_a_blocked_prefetch_when_it_becomes_current() {
        use crossbeam_channel::{bounded, unbounded};
        use std::sync::atomic::AtomicUsize;
        let (started_tx, started_rx) = unbounded();
        let (release_a_tx, release_a_rx) = bounded(1);
        let (release_b_tx, release_b_rx) = bounded(1);
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let main_thread = std::thread::current().id();
        let ctx = egui::Context::default();
        let mut images = Images::with_decoder(ctx.clone(), move |r, _| {
            assert_ne!(std::thread::current().id(), main_thread);
            started_tx.send(r.key.path.clone()).unwrap();
            if r.key.path == std::path::Path::new("a") {
                release_a_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            } else {
                count.fetch_add(1, Ordering::Relaxed);
                release_b_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            Ok(sample())
        });
        let a = images.key("a".into(), false);
        let b = images.key("b".into(), false);
        images.plan(
            std::slice::from_ref(&a),
            std::slice::from_ref(&b),
            &Settings::default(),
            8192,
        );
        let starts = [
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        ];
        assert!(starts.contains(&PathBuf::from("a")) && starts.contains(&PathBuf::from("b")));
        images.plan(std::slice::from_ref(&b), &[], &Settings::default(), 8192);
        release_b_tx.send(()).unwrap();
        wait_ready(&images);
        images.poll(&ctx, &Settings::default(), &HashSet::from([b.clone()]));
        assert!(images.texture(&b).is_some());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(!images.textures.contains_key(&a)); // the old foreground is still blocked
        release_a_tx.send(()).unwrap();
        images.plan(std::slice::from_ref(&b), &[], &Settings::default(), 8192);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn reserved_foreground_worker_serves_a_new_image_while_prefetch_is_blocked() {
        use crossbeam_channel::{bounded, unbounded};
        let (started_tx, started_rx) = unbounded();
        let (release_tx, release_rx) = bounded(1);
        let ctx = egui::Context::default();
        let mut images = Images::with_decoder(ctx, move |r, _| {
            started_tx.send(r.key.path.clone()).unwrap();
            if r.key.path == std::path::Path::new("b") {
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            Ok(sample())
        });
        let a = images.key("a".into(), false);
        let b = images.key("b".into(), false);
        let c = images.key("c".into(), false);
        images.plan(
            std::slice::from_ref(&a),
            std::slice::from_ref(&b),
            &Settings::default(),
            8192,
        );
        for _ in 0..2 {
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        images.plan(
            std::slice::from_ref(&c),
            std::slice::from_ref(&b),
            &Settings::default(),
            8192,
        );
        assert_eq!(
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            PathBuf::from("c")
        );
        release_tx.send(()).unwrap();
    }
    #[test]
    fn worker_prepares_alpha_and_new_processing_parameters_invalidate_old_pixels() {
        let ctx = egui::Context::default();
        let mut images = Images::with_decoder(ctx, |_, _| Ok(sample()));
        let mut settings = Settings::default();
        images.configure(&settings, 8192);
        let key = images.key("alpha".into(), false);
        images.plan(std::slice::from_ref(&key), &[], &settings, 8192);
        wait_ready(&images);
        let loaded = images.queue.take_ready(None).unwrap().prepared.unwrap();
        assert_eq!(loaded.color.size, [2, 3]);
        assert_eq!(
            loaded.color.pixels[0],
            egui::Color32::from_rgba_unmultiplied(100, 150, 200, 128)
        );
        settings.upscale_factor = 3;
        images.configure(&settings, 8192);
        assert_ne!(images.key("alpha".into(), false), key);
    }
    #[test]
    fn texture_delivery_defers_a_second_large_image_to_the_next_frame() {
        let ctx = egui::Context::default();
        let queue = Arc::new(WorkQueue::default());
        let mut images = Images {
            queue: queue.clone(),
            thumbs: Arc::new(WorkQueue::default()),
            pregen: None,
            processing: None,
            textures: HashMap::new(),
            failures: HashMap::new(),
            revision: 0,
            tick: 0,
            warning: None,
            prefetch: Vec::new(),
            disk_cache: Arc::new(ThumbnailCache::new(None, 0)),
            proxy_cache: Arc::new(ThumbnailCache::new(None, 0)),
        };
        let a = images.key("a".into(), false);
        let b = images.key("b".into(), false);
        let settings = Settings {
            cache_mb: 256,
            ..Default::default()
        };
        images.plan(&[a.clone(), b.clone()], &[], &settings, 8192);
        let first = queue.take(false).unwrap();
        let second = queue.take(true).unwrap();
        let prepared = || {
            Ok(Prepared {
                started: std::time::Instant::now(),
                color: egui::ColorImage::new([1536, 1536], egui::Color32::WHITE),
                dimensions: [1536, 1536],
                warning: None,
            })
        };
        queue.complete(first, prepared(), false);
        queue.complete(second, prepared(), true);
        let keep = HashSet::from([a, b]);
        images.poll(&ctx, &settings, &keep);
        assert_eq!(images.textures.len(), 1);
        images.poll(&ctx, &settings, &keep);
        assert_eq!(images.textures.len(), 2);
    }
    fn request(path: PathBuf) -> Request {
        Request {
            key: Key {
                path,
                thumb: false,
                preview: false,
                revision: 0,
            },
            settings: Settings::default(),
            max_side: 8192,
            cancelled: AtomicBool::new(false),
        }
    }
    #[test]
    fn jpeg_without_exif_previews_while_full_detail_is_blocked_and_cache_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.JPG");
        let cache_root = dir.path().join("cache");
        image::RgbImage::from_fn(1600, 1200, |x, y| {
            image::Rgb([(x / 7) as u8, (y / 5) as u8, 90])
        })
        .save(&path)
        .unwrap();
        assert!(crate::jpeg_preview::decode(&path, |_| true).is_err());
        let expected = image::open(&path).unwrap().into_rgba8();
        let ctx = egui::Context::default();
        let cache = Arc::new(ThumbnailCache::new(Some(cache_root.clone()), 512));
        let working = cache.clone();
        let (release, wait) = crossbeam_channel::bounded(1);
        let mut images = Images::with_decoder(ctx.clone(), move |r, q| {
            if !r.key.preview && !r.key.thumb {
                wait.recv_timeout(Duration::from_secs(10)).unwrap();
            }
            decode_cached(
                r,
                &Mutex::new(None),
                q,
                &working,
                &Arc::new(ThumbnailCache::new(None, 0)),
            )
        });
        let settings = Settings::default();
        let full = images.key(path.clone(), false);
        let preview = images.preview_key(path.clone());
        let demand = [full.clone(), preview.clone()];
        let keep = demand.iter().cloned().collect();
        images.plan(&demand, &[], &settings, 8192);
        wait_ready(&images);
        images.poll(&ctx, &settings, &keep);
        let (_, dimensions, is_full) = images.display_texture(&path, true).unwrap();
        assert!(!is_full);
        assert_eq!(dimensions, [1600, 1200]);
        assert!(images.textures[&preview].bytes <= 512 * 512 * 4);
        release.send(()).unwrap();
        wait_ready(&images);
        let ready = images.queue.take_ready(None).unwrap().prepared.unwrap();
        assert_eq!(ready.dimensions, [1600, 1200]);
        assert_eq!(
            ready.color.pixels,
            egui::ColorImage::from_rgba_unmultiplied([1600, 1200], expected.as_raw()).pixels
        );
        // The preview and the thumbnail harvested from the full decode.
        wait_writes(&cache, 2);
        images.stop();
        drop(images);
        let mut warm = Images::new(ctx, Some(cache_root), 512);
        let preview = warm.preview_key(path);
        warm.plan(std::slice::from_ref(&preview), &[], &settings, 8192);
        wait_ready(&warm);
        let ready = warm.queue.take_ready(None).unwrap().prepared.unwrap();
        assert_eq!(ready.dimensions, [1600, 1200]);
        assert_eq!(warm.disk_cache.stats().hits, 1);
        assert_eq!(warm.disk_cache.stats().writes, 0);
    }
    #[test]
    fn small_jpegs_and_oversized_headers_keep_the_original_thumbnail_path() {
        use std::io::Cursor;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small.jpg");
        let original = image::RgbImage::from_fn(80, 120, |x, y| image::Rgb([x as u8, y as u8, 90]));
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(original)
            .write_to(&mut encoded, image::ImageFormat::Jpeg)
            .unwrap();
        let bytes = encoded.into_inner();
        std::fs::write(&path, &bytes).unwrap();
        let expected = image::open(&path).unwrap().into_rgba8();
        let mut request = request(path.clone());
        request.key.thumb = true;
        let (pixels, dimensions, _) = decode(&request, &Mutex::new(None), None).unwrap();
        assert_eq!(pixels, expected);
        assert_eq!(dimensions, [80, 120]);
        let mut padded = bytes[..2].to_vec();
        for _ in 0..17 {
            padded.extend([0xff, 0xed, 0xff, 0xff]);
            padded.extend(vec![0; 65533]);
        }
        padded.extend(&bytes[2..]);
        std::fs::write(&path, &padded).unwrap();
        assert!(
            crate::jpeg_scaled::decode(&path, 512, 768, &AtomicBool::new(false), |_| true).is_err()
        );
        let (pixels, dimensions, _) = decode(&request, &Mutex::new(None), None).unwrap();
        assert_eq!(pixels, expected);
        assert_eq!(dimensions, [80, 120]);
    }
    #[test]
    fn single_decoder_preserves_exif_orientation_and_pixels() {
        use std::io::Cursor;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("orientation.jpg");
        let original = DynamicImage::ImageRgba8(RgbaImage::from_fn(3, 2, |x, y| {
            image::Rgba([(x * 80) as u8, (y * 100) as u8, 90, 255])
        }))
        .to_rgb8();
        let mut data = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(original)
            .write_to(&mut data, image::ImageFormat::Jpeg)
            .unwrap();
        let jpeg = data.into_inner();
        let expected = image::load_from_memory(&jpeg)
            .unwrap()
            .rotate90()
            .into_rgba8();
        let mut exif =
            b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec();
        let mut tagged = jpeg[..2].to_vec();
        tagged.extend_from_slice(&[0xff, 0xe1]);
        tagged.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        tagged.append(&mut exif);
        tagged.extend_from_slice(&jpeg[2..]);
        std::fs::write(&path, tagged).unwrap();
        let (pixels, dimensions, _) =
            decode(&request(path.clone()), &Mutex::new(None), None).unwrap();
        assert_eq!(dimensions, [2, 3]);
        assert_eq!(pixels, expected);
        for tags in [
            std::collections::BTreeSet::from(["crème brûlée 日本語 🌅".into()]),
            std::collections::BTreeSet::new(),
        ] {
            crate::metadata::write(&path, &tags).unwrap();
            let (pixels, dimensions, _) =
                decode(&request(path.clone()), &Mutex::new(None), None).unwrap();
            assert_eq!(dimensions, [2, 3]);
            assert_eq!(pixels, expected);
        }
    }
    #[test]
    fn single_decoder_rejects_an_output_buffer_above_the_allocation_limit() {
        use std::io::Cursor;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.bmp");
        let mut data = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image::RgbImage::new(1, 1))
            .write_to(&mut data, image::ImageFormat::Bmp)
            .unwrap();
        let mut bytes = data.into_inner();
        bytes[18..22].copy_from_slice(&10000u32.to_le_bytes());
        bytes[22..26].copy_from_slice(&10000u32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let mut r = request(path);
        r.settings.cache_mb = 64;
        let error = decode(&r, &Mutex::new(None), None).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<image::ImageError>(),
                Some(image::ImageError::Limits(_))
            ),
            "{error:#}"
        );
    }
    /// Synthetic photo-like PNGs: thumbnail and full-view preparation, the
    /// 0.1.30 steps against the current ones. Runs in Windows CI.
    #[test]
    #[ignore = "manual benchmark: cargo test --release bench_png -- --ignored --nocapture"]
    fn bench_png() {
        use image::ImageEncoder;
        let dir = tempfile::tempdir().unwrap();
        let best = |f: &mut dyn FnMut()| {
            (0..3)
                .map(|_| {
                    let t = Instant::now();
                    f();
                    t.elapsed().as_secs_f64() * 1000.
                })
                .fold(f64::MAX, f64::min)
        };
        eprintln!("size          MB | thumb: 0.1.30 -> now ms | full view: 0.1.30 -> now ms");
        for (w, h) in [(1024u32, 1024u32), (2560, 1440), (3840, 2160), (6000, 4000)] {
            let mut seed = 0x9e37_79b9u32 ^ w;
            let mut pixels = vec![0u8; (w * h * 3) as usize];
            for y in 0..h {
                for x in 0..w {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let noise = (seed >> 24) as f32 / 255. * 10. - 5.;
                    let (fx, fy) = (x as f32 / w as f32 * 9., y as f32 / h as f32 * 7.);
                    let base = [
                        128. + 70. * (fx * 1.3).sin() * (fy * 0.9).cos(),
                        128. + 60. * (fx * 0.7 + fy).cos(),
                        110. + 50. * (fy * 1.7).sin() + 30. * (fx * 3.1).sin(),
                    ];
                    for c in 0..3 {
                        pixels[((y * w + x) * 3) as usize + c] =
                            (base[c] + noise).clamp(0., 255.) as u8;
                    }
                }
            }
            let path = dir.path().join(format!("{w}x{h}.png"));
            image::codecs::png::PngEncoder::new_with_quality(
                std::fs::File::create(&path).unwrap(),
                image::codecs::png::CompressionType::Default,
                image::codecs::png::FilterType::Adaptive,
            )
            .write_image(&pixels, w, h, image::ExtendedColorType::Rgb8)
            .unwrap();
            drop(pixels);
            let mb = std::fs::metadata(&path).unwrap().len() as f64 / 1e6;
            let request = |thumb| Request {
                key: Key {
                    path: path.clone(),
                    thumb,
                    preview: false,
                    revision: 0,
                },
                settings: Settings {
                    cache_mb: 2048,
                    ..Default::default()
                },
                max_side: 8192,
                cancelled: AtomicBool::new(false),
            };
            let old = |thumb: bool| {
                let reader = image::ImageReader::open(&path)
                    .unwrap()
                    .with_guessed_format()
                    .unwrap();
                let mut image = DynamicImage::from_decoder(reader.into_decoder().unwrap()).unwrap();
                if thumb {
                    image = image.resize(512, 512, image::imageops::FilterType::Lanczos3);
                }
                let rgba = image.into_rgba8();
                egui::ColorImage::from_rgba_unmultiplied(
                    [rgba.width() as usize, rgba.height() as usize],
                    rgba.as_raw(),
                )
            };
            let new = |thumb: bool| {
                let decoded = decode_image(&request(thumb), &Mutex::new(None), None).unwrap();
                color_image(&decoded.image)
            };
            let times = [
                best(&mut || drop(old(true))),
                best(&mut || drop(new(true))),
                best(&mut || drop(old(false))),
                best(&mut || drop(new(false))),
            ];
            eprintln!(
                "{w:>4}x{h:<4} {mb:6.1} | {:8.1} -> {:6.1}      | {:8.1} -> {:6.1}",
                times[0], times[1], times[2], times[3]
            );
        }
        // A 24 Mpx JPEG: the conversion passes cost more than the decode did.
        let (w, h) = (6000u32, 4000u32);
        let pixels: Vec<u8> = (0..w * h * 3)
            .map(|i| ((i / 3 % w) / 24 + (i / 3 / w) / 16 + i % 3 * 20) as u8)
            .collect();
        let path = dir.path().join("large.jpg");
        jpeg_encoder::Encoder::new_file(&path, 90)
            .unwrap()
            .encode(&pixels, w as u16, h as u16, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        drop(pixels);
        let request = Request {
            key: Key {
                path: path.clone(),
                thumb: false,
                preview: false,
                revision: 0,
            },
            settings: Settings {
                cache_mb: 2048,
                ..Default::default()
            },
            max_side: 8192,
            cancelled: AtomicBool::new(false),
        };
        let old = best(&mut || {
            let reader = image::ImageReader::open(&path)
                .unwrap()
                .with_guessed_format()
                .unwrap();
            let rgba = DynamicImage::from_decoder(reader.into_decoder().unwrap())
                .unwrap()
                .into_rgba8();
            drop(egui::ColorImage::from_rgba_unmultiplied(
                [w as usize, h as usize],
                rgba.as_raw(),
            ));
        });
        let new = best(&mut || {
            let decoded = decode_image(&request, &Mutex::new(None), None).unwrap();
            drop(color_image(&decoded.image));
        });
        eprintln!("JPEG 6000x4000 full view: 0.1.30 {old:.1} -> now {new:.1} ms");
    }
    #[test]
    #[ignore = "Requires ONNX Runtime 1.22; set NOIRVIEW_TEST_ORT to its library path"]
    fn onnx_tiled_identity_preserves_rgb_alpha_and_dimensions() {
        let runtime = std::env::var_os("NOIRVIEW_TEST_ORT").expect("Set NOIRVIEW_TEST_ORT");
        let settings = Settings {
            ort_library: runtime.into(),
            model_path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/identity.onnx"),
            tile_size: 32,
            ..Default::default()
        };
        let original = RgbaImage::from_fn(79, 57, |x, y| {
            image::Rgba([
                (x * 3) as u8,
                (y * 4) as u8,
                ((x + y) * 5) as u8,
                (x * 2 + y) as u8,
            ])
        });
        let ai = Mutex::new(None);
        let cancelled = AtomicBool::new(false);
        let result = upscale_ai(
            &DynamicImage::ImageRgba8(original.clone()),
            &settings,
            &ai,
            8192,
            &cancelled,
        )
        .unwrap();
        assert_eq!(result, original);
    }
}
