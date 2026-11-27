//! Disposable, bounded thumbnail/embedded-preview cache. PNG blobs are committed
//! atomically in a separate SQLite database; user state and source files are untouched.
use anyhow::{ensure, Result};
use crossbeam_channel::{bounded, Sender};
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    DynamicImage, ImageDecoder, ImageEncoder, RgbaImage,
};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Longer side of a stored proxy.
pub const PROXY_SIDE: u32 = 2560;
const VERSION: u8 = 1;
const MAX_BLOB: usize = 6 * 1024 * 1024;
// Opaque thumbnails are stored as JPEG (~40-80 KiB), so the byte budget, not
// the entry count, normally decides how many fit.
const MAX_ENTRIES: i64 = 65536;
/// A budget with no practical limit (MiB); only free disk space stops writes.
pub const UNLIMITED_MB: usize = 1 << 28;
/// How often free disk space is checked again.
const SPACE_CHECK: Duration = Duration::from_secs(10);
const JPEG_QUALITY: u8 = 90;
const PENDING_BYTES: usize = 64 * 1024 * 1024;
const PENDING_ENTRIES: usize = 128;
const DATABASE: &str = "cache-v1.sqlite3";

#[derive(Clone, Copy)]
pub enum Kind {
    Thumbnail,
    Preview,
    /// Screen-sized first view of a large image (Aggressive loading).
    Proxy,
}
impl Kind {
    fn cap(self) -> u32 {
        match self {
            Self::Thumbnail => 512,
            Self::Preview => 1024,
            Self::Proxy => PROXY_SIDE,
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    size: u64,
    seconds: u64,
    nanos: u32,
    identity: Vec<u8>,
}
impl Stamp {
    fn read(path: &Path) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        if !meta.is_file() {
            return None;
        }
        let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
        let mut identity = Vec::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            identity.extend(meta.dev().to_le_bytes());
            identity.extend(meta.ino().to_le_bytes());
            identity.extend(meta.ctime().to_le_bytes());
            identity.extend(meta.ctime_nsec().to_le_bytes());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            identity.extend(meta.creation_time().to_le_bytes());
        }
        Some(Self {
            size: meta.len(),
            seconds: modified.as_secs(),
            nanos: modified.subsec_nanos(),
            identity,
        })
    }
}
pub struct Token {
    source: PathBuf,
    stamp: Stamp,
    key: Vec<u8>,
    epoch: u64,
    kind: Kind,
}
pub struct Stats {
    pub hits: u64,
    pub writes: u64,
    pub bytes: u64,
    pub entries: u64,
    pub pending: usize,
    pub skipped: u64,
    pub errors: u64,
}
struct Core {
    root: Option<PathBuf>,
    wake: Arc<dyn Fn() + Send + Sync>,
    reader: Mutex<Option<Connection>>,
    budget: AtomicUsize,
    epoch: AtomicU64,
    hits: AtomicU64,
    writes: AtomicU64,
    bytes: AtomicU64,
    entries: AtomicU64,
    needs_prune: AtomicBool,
    pending: Mutex<PendingWrites>,
    active: AtomicBool,
    accepting: AtomicBool,
    skipped: AtomicU64,
    errors: AtomicU64,
    /// Writes stop while the cache's disk has less free space than this.
    keep_free: AtomicU64,
    space: Mutex<Option<(Instant, Option<u64>)>>,
}
impl Core {
    /// Free space on the cache's disk (checked at most every `SPACE_CHECK`).
    fn free_space(&self) -> Option<u64> {
        let root = self.root.as_deref()?;
        let mut space = self.space.lock().unwrap();
        if let Some((at, free)) = *space {
            if at.elapsed() < SPACE_CHECK {
                return free;
            }
        }
        let free = crate::disk::free_space(root);
        *space = Some((Instant::now(), free));
        free
    }
    fn space_ok(&self) -> bool {
        let keep = self.keep_free.load(Ordering::Relaxed);
        keep == 0 || self.free_space().is_none_or(|free| free >= keep)
    }
}
fn budget_bytes(mb: usize) -> usize {
    mb.saturating_mul(1024 * 1024)
}
/// Pixels still to be encoded by the writer, or an image its producer
/// already encoded (large first views: ~1 MiB instead of ~15 MiB waiting).
enum Payload {
    Pixels(RgbaImage),
    Encoded { blob: Vec<u8>, size: [u32; 2] },
}
impl Payload {
    fn bytes(&self) -> usize {
        match self {
            Self::Pixels(pixels) => pixels.as_raw().len(),
            Self::Encoded { blob, .. } => blob.len(),
        }
    }
}
struct PendingWrite {
    token: Token,
    pixels: Payload,
    dimensions: [u32; 2],
}
#[derive(Default)]
struct PendingWrites {
    jobs: VecDeque<PendingWrite>,
    bytes: usize,
}
impl PendingWrites {
    fn remove(&mut self, index: usize) -> PendingWrite {
        let job = self.jobs.remove(index).unwrap();
        self.bytes -= job.pixels.bytes();
        job
    }
    fn insert(&mut self, job: PendingWrite) -> u64 {
        if let Some(i) = self.jobs.iter().position(|j| j.token.key == job.token.key) {
            self.remove(i);
        }
        let bytes = job.pixels.bytes();
        let mut skipped = 0;
        // Keep the latest demand during extreme fast paging. A complete maximum
        // grid page (64 x 512² RGBA) fits; control-channel pressure never drops it.
        while !self.jobs.is_empty()
            && (self.jobs.len() >= PENDING_ENTRIES || self.bytes + bytes > PENDING_BYTES)
        {
            self.remove(0);
            skipped += 1;
        }
        if bytes > PENDING_BYTES {
            return skipped + 1;
        }
        self.bytes += bytes;
        self.jobs.push_back(job);
        skipped
    }
}
enum Command {
    Wake,
    Prune,
    Touch(Vec<u8>),
    Clear(Sender<Result<(), String>>),
    Flush(Sender<()>),
    #[cfg(test)]
    Pause(Sender<()>, crossbeam_channel::Receiver<()>),
}
pub struct ThumbnailCache {
    core: Arc<Core>,
    tx: Option<Sender<Command>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl ThumbnailCache {
    pub fn new(root: Option<PathBuf>, budget_mb: usize) -> Self {
        Self::with_wake(root, budget_mb, || {})
    }
    pub fn with_wake(
        root: Option<PathBuf>,
        budget_mb: usize,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let core = Arc::new(Core {
            root,
            wake: Arc::new(wake),
            reader: Mutex::new(None),
            budget: AtomicUsize::new(budget_bytes(budget_mb)),
            epoch: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            writes: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            entries: AtomicU64::new(0),
            needs_prune: AtomicBool::new(false),
            pending: Mutex::new(PendingWrites::default()),
            active: AtomicBool::new(false),
            accepting: AtomicBool::new(true),
            skipped: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            keep_free: AtomicU64::new(0),
            space: Mutex::new(None),
        });
        let mut worker_handle = None;
        let tx = if core.root.is_some() {
            // Small control channel; pixel writes remain in the bounded retry buffer.
            let (tx, rx) = bounded(2);
            let worker = core.clone();
            worker_handle = Some(std::thread::spawn(move || {
                let mut connection = None;
                let mut flushes = Vec::new();
                loop {
                    let command = match rx.try_recv() {
                        Ok(command) => Some(command),
                        Err(_) if !worker.pending.lock().unwrap().jobs.is_empty() => None,
                        Err(_) => match rx.recv() {
                            Ok(command) => Some(command),
                            Err(_) => break,
                        },
                    };
                    match command {
                        Some(Command::Wake) | None => {}
                        Some(Command::Prune) => {}
                        Some(Command::Touch(key)) => {
                            if let Ok(c) = worker.writer(&mut connection) {
                                let used = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_micros()
                                    .min(i64::MAX as u128)
                                    as i64;
                                let _ = c.execute(
                                    "UPDATE thumbnails SET used=MAX(?1,COALESCE((SELECT MAX(used) FROM thumbnails),0)+1) WHERE key=?2",
                                    params![used, key],
                                );
                            }
                        }
                        Some(Command::Clear(reply)) => {
                            connection = None;
                            *worker.reader.lock().unwrap() = None;
                            let result = worker.remove_database().map_err(|e| format!("{e:#}"));
                            let _ = reply.send(result);
                        }
                        Some(Command::Flush(reply)) => flushes.push(reply),
                        #[cfg(test)]
                        Some(Command::Pause(entered, release)) => {
                            let _ = entered.send(());
                            let _ = release.recv_timeout(Duration::from_secs(10));
                        }
                    }
                    let job = {
                        let mut pending = worker.pending.lock().unwrap();
                        if pending.jobs.is_empty() {
                            None
                        } else {
                            worker.active.store(true, Ordering::Relaxed);
                            Some(pending.remove(0))
                        }
                    };
                    if let Some(job) = job {
                        for attempt in 0..4 {
                            let result = worker.store(
                                &mut connection,
                                &job.token,
                                &job.pixels,
                                job.dimensions,
                            );
                            match result {
                                Ok(()) => break,
                                Err(error) if attempt < 3 && busy(&error) => {
                                    std::thread::sleep(Duration::from_millis(20 << attempt));
                                }
                                Err(error) => {
                                    worker.errors.fetch_add(1, Ordering::Relaxed);
                                    if std::env::var_os("NOIRVIEW_IMAGE_TRACE").is_some() {
                                        eprintln!("thumbnail cache write failed: {error:#}");
                                    }
                                    break;
                                }
                            }
                        }
                        worker.active.store(false, Ordering::Relaxed);
                    }
                    if worker.pending.lock().unwrap().jobs.is_empty() {
                        for reply in flushes.drain(..) {
                            let _ = reply.send(());
                        }
                    }
                    if worker.needs_prune.swap(false, Ordering::Relaxed) {
                        let _ = worker.prune(&mut connection);
                    }
                    (worker.wake)();
                }
            }));
            Some(tx)
        } else {
            None
        };
        Self {
            core,
            tx,
            worker: worker_handle,
        }
    }
    pub fn configure(&self, mb: usize) {
        let bytes = budget_bytes(mb);
        let old = self.core.budget.swap(bytes, Ordering::Relaxed);
        if bytes > 0 && bytes < old {
            self.core.needs_prune.store(true, Ordering::Relaxed);
            if let Some(tx) = &self.tx {
                let _ = tx.try_send(Command::Prune);
            }
        }
    }
    /// Byte budget (0 when disabled).
    pub fn budget(&self) -> u64 {
        self.core.budget.load(Ordering::Relaxed) as u64
    }
    /// Stop writing while the cache's disk has less free space than `bytes`
    /// (0: no check).
    pub fn keep_free(&self, bytes: u64) {
        self.core.keep_free.store(bytes, Ordering::Relaxed);
    }
    /// The folder of the cache's files.
    pub fn root(&self) -> Option<&Path> {
        self.core.root.as_deref()
    }
    /// Free space on the cache's disk, if known.
    pub fn free_space(&self) -> Option<u64> {
        self.core.free_space()
    }
    /// Whether the cache's disk has the free space it must keep.
    pub fn space_ok(&self) -> bool {
        self.core.space_ok()
    }
    /// Room for a background pass: below 90 % of the budget, so it never
    /// evicts its own earlier entries, and enough free disk space.
    pub fn has_room(&self) -> bool {
        let budget = self.budget();
        budget > 0 && self.stats().bytes < budget / 10 * 9 && self.space_ok()
    }
    /// Whether a current entry exists, without decoding it.
    pub fn contains(&self, token: &Token) -> bool {
        token.epoch == self.core.epoch.load(Ordering::Relaxed)
            && self.core.budget.load(Ordering::Relaxed) > 0
            && self.core.contains(token)
    }
    pub fn stats(&self) -> Stats {
        Stats {
            hits: self.core.hits.load(Ordering::Relaxed),
            writes: self.core.writes.load(Ordering::Relaxed),
            bytes: self.core.bytes.load(Ordering::Relaxed),
            entries: self.core.entries.load(Ordering::Relaxed),
            pending: self.core.pending.lock().unwrap().jobs.len()
                + usize::from(self.core.active.load(Ordering::Relaxed)),
            skipped: self.core.skipped.load(Ordering::Relaxed),
            errors: self.core.errors.load(Ordering::Relaxed),
        }
    }
    pub fn token(&self, source: &Path, kind: Kind) -> Option<Token> {
        if self.core.root.is_none() || self.core.budget.load(Ordering::Relaxed) == 0 {
            return None;
        }
        let stamp = Stamp::read(source)?;
        let mut hash = Sha256::new();
        hash.update(b"NoirView thumbnail pixels");
        hash.update([VERSION]);
        hash.update(kind.cap().to_le_bytes());
        let path_bytes = source.as_os_str().as_encoded_bytes();
        hash.update((path_bytes.len() as u64).to_le_bytes());
        hash.update(path_bytes);
        hash.update(stamp.size.to_le_bytes());
        hash.update(stamp.seconds.to_le_bytes());
        hash.update(stamp.nanos.to_le_bytes());
        hash.update(&stamp.identity);
        Some(Token {
            source: source.into(),
            stamp,
            key: hash.finalize().to_vec(),
            epoch: self.core.epoch.load(Ordering::Relaxed),
            kind,
        })
    }
    pub fn load(
        &self,
        token: &Token,
        reserve: impl FnOnce(usize) -> bool,
    ) -> Result<Option<(RgbaImage, [u32; 2])>> {
        if token.epoch != self.core.epoch.load(Ordering::Relaxed)
            || self.core.budget.load(Ordering::Relaxed) == 0
        {
            return Ok(None);
        }
        let row = self.core.read(token);
        let Some((png, dimensions)) = row else {
            return Ok(None);
        };
        let decoded = (|| -> Result<_> {
            // JPEG for opaque thumbnails, PNG for alpha and for older entries.
            let format = if png.starts_with(&[0xFF, 0xD8, 0xFF]) {
                image::ImageFormat::Jpeg
            } else {
                image::ImageFormat::Png
            };
            let mut reader = image::ImageReader::with_format(Cursor::new(png), format);
            let mut limits = image::Limits::default();
            // Decoded pixels plus the decoder's working rows.
            let side = token.kind.cap() as u64;
            limits.max_alloc = Some((side * side * 8).max(8 * 1024 * 1024));
            limits.max_image_width = Some(token.kind.cap());
            limits.max_image_height = Some(token.kind.cap());
            reader.limits(limits.clone());
            let mut decoder = reader.into_decoder()?;
            let (w, h) = decoder.dimensions();
            ensure!(
                w > 0 && h > 0 && w <= token.kind.cap() && h <= token.kind.cap(),
                "Invalid cached thumbnail dimensions"
            );
            ensure!(
                reserve(w as usize * h as usize * 4),
                crate::image_queue::PrefetchDeferred
            );
            limits.reserve(decoder.total_bytes())?;
            decoder.set_limits(limits)?;
            Ok(DynamicImage::from_decoder(decoder)?.into_rgba8())
        })();
        match decoded {
            Ok(pixels)
                if Stamp::read(&token.source).as_ref() == Some(&token.stamp)
                    && token.epoch == self.core.epoch.load(Ordering::Relaxed) =>
            {
                self.core.hits.fetch_add(1, Ordering::Relaxed);
                if let Some(tx) = &self.tx {
                    let _ = tx.try_send(Command::Touch(token.key.clone()));
                }
                Ok(Some((pixels, dimensions)))
            }
            Err(e) if e.is::<crate::image_queue::PrefetchDeferred>() => Err(e),
            _ => Ok(None),
        }
    }
    pub fn store(&self, token: Token, pixels: &RgbaImage, dimensions: [u32; 2]) {
        self.enqueue(token, Payload::Pixels(pixels.clone()), dimensions);
    }
    fn enqueue(&self, token: Token, pixels: Payload, dimensions: [u32; 2]) {
        let Some(tx) = &self.tx else {
            return;
        };
        if self.core.budget.load(Ordering::Relaxed) == 0 {
            return;
        }
        if !self.core.space_ok() {
            self.core.skipped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let mut pending = self.core.pending.lock().unwrap();
        if !self.core.accepting.load(Ordering::Relaxed)
            || token.epoch != self.core.epoch.load(Ordering::Relaxed)
        {
            return;
        }
        let skipped = pending.insert(PendingWrite {
            token,
            pixels,
            dimensions,
        });
        self.core.skipped.fetch_add(skipped, Ordering::Relaxed);
        drop(pending);
        // A full control channel already wakes the writer. The pixels stay queued.
        let _ = tx.try_send(Command::Wake);
    }
    /// `store` for large images produced in bursts (first views). The caller,
    /// a background worker, encodes the image itself so only the compact
    /// result waits for the writer, and then waits (up to `patience`) for
    /// room instead of pushing an older pending image out.
    pub fn store_when_room(
        &self,
        token: Token,
        pixels: &RgbaImage,
        dimensions: [u32; 2],
        patience: Duration,
    ) {
        if self.tx.is_none() || self.core.budget.load(Ordering::Relaxed) == 0 {
            return;
        }
        let Ok(blob) = encode(pixels) else {
            self.core.errors.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let bytes = blob.len();
        let deadline = Instant::now() + patience;
        loop {
            let room = {
                let pending = self.core.pending.lock().unwrap();
                pending.jobs.is_empty()
                    || (pending.jobs.len() < PENDING_ENTRIES
                        && pending.bytes + bytes <= PENDING_BYTES)
            };
            if room
                || Instant::now() >= deadline
                || !self.core.accepting.load(Ordering::Relaxed)
                || token.epoch != self.core.epoch.load(Ordering::Relaxed)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let size = [pixels.width(), pixels.height()];
        self.enqueue(token, Payload::Encoded { blob, size }, dimensions);
    }
    pub fn seal(&self) {
        let _pending = self.core.pending.lock().unwrap();
        self.core.accepting.store(false, Ordering::Relaxed);
    }
    pub fn wait_idle(&self, timeout: Duration) -> Result<(), String> {
        let Some(tx) = &self.tx else {
            return Ok(());
        };
        let deadline = Instant::now() + timeout;
        let (reply, rx) = bounded(1);
        tx.send_timeout(Command::Flush(reply), timeout)
            .map_err(|e| e.to_string())?;
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|e| e.to_string())
    }
    pub fn clear(&self) -> Result<(), String> {
        {
            let mut pending = self.core.pending.lock().unwrap();
            self.core.epoch.fetch_add(1, Ordering::Relaxed);
            pending.jobs.clear();
            pending.bytes = 0;
        }
        let Some(tx) = &self.tx else {
            return Ok(());
        };
        let (reply, rx) = bounded(1);
        tx.send(Command::Clear(reply)).map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }
    #[cfg(test)]
    pub(crate) fn flush(&self) {
        self.wait_idle(Duration::from_secs(10)).unwrap();
    }
}
impl Drop for ThumbnailCache {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(worker) = self.worker.take() {
            // Production close already waited up to five seconds. A blocked
            // optional-cache writer must not turn Drop into an unlimited wait.
            // Tests join to release SQLite handles before TempDir cleanup on Windows.
            #[cfg(test)]
            let _ = worker.join();
            #[cfg(not(test))]
            drop(worker);
        }
    }
}
/// Opaque pixels as JPEG (much smaller and faster than PNG for photos);
/// anything with transparency stays lossless PNG.
fn encode(pixels: &RgbaImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    if pixels.pixels().all(|p| p[3] == 255) {
        let rgb = DynamicImage::ImageRgba8(pixels.clone()).into_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY).write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )?;
    } else {
        PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Sub)
            .write_image(
                pixels.as_raw(),
                pixels.width(),
                pixels.height(),
                image::ExtendedColorType::Rgba8,
            )?;
    }
    Ok(out)
}
impl Core {
    fn contains(&self, token: &Token) -> bool {
        self.read_with(|c| {
            c.query_row(
                "SELECT 1 FROM thumbnails WHERE key=?1",
                params![token.key],
                |_| Ok(()),
            )
            .optional()
            .ok()
            .flatten()
        })
        .is_some()
    }
    fn read_with<T>(&self, f: impl FnOnce(&Connection) -> Option<T>) -> Option<T> {
        let mut reader = self.reader.lock().unwrap();
        if reader.is_none() {
            let c = Connection::open_with_flags(
                self.root.as_ref()?.join(DATABASE),
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .ok()?;
            c.busy_timeout(Duration::from_millis(20)).ok()?;
            if c.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .ok()?
                != VERSION as u32
            {
                return None;
            }
            *reader = Some(c);
        }
        f(reader.as_ref()?)
    }
    fn read(&self, token: &Token) -> Option<(Vec<u8>, [u32; 2])> {
        let mut reader = self.reader.lock().unwrap();
        if reader.is_none() {
            let c = Connection::open_with_flags(
                self.root.as_ref()?.join(DATABASE),
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .ok()?;
            c.busy_timeout(Duration::from_millis(20)).ok()?;
            if c.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .ok()?
                != VERSION as u32
            {
                return None;
            }
            *reader = Some(c);
        }
        reader.as_ref()?.query_row("SELECT png, source_width, source_height FROM thumbnails WHERE key=?1 AND size<=?2 AND length(png)<=?2", params![token.key, MAX_BLOB as i64], |row| {
            let dimensions = [row.get::<_,u32>(1)?,row.get::<_,u32>(2)?];
            Ok((row.get::<_,Vec<u8>>(0)?,dimensions))
        }).optional().ok().flatten().filter(|(_,d)| d.iter().all(|n| *n>0))
    }
    fn writer<'a>(&self, connection: &'a mut Option<Connection>) -> Result<&'a mut Connection> {
        if connection.is_none() {
            let root = self.root.as_ref().unwrap();
            fs::create_dir_all(root)?;
            let c = Connection::open(root.join(DATABASE))?;
            c.busy_timeout(Duration::from_millis(25))?;
            c.execute_batch("PRAGMA auto_vacuum=FULL; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA wal_autocheckpoint=256; PRAGMA journal_size_limit=8388608;
                CREATE TABLE IF NOT EXISTS thumbnails(key BLOB PRIMARY KEY, png BLOB NOT NULL, size INTEGER NOT NULL, source_width INTEGER NOT NULL, source_height INTEGER NOT NULL, used INTEGER NOT NULL);
                CREATE INDEX IF NOT EXISTS thumbnail_lru ON thumbnails(used); CREATE INDEX IF NOT EXISTS thumbnail_sizes ON thumbnails(size); PRAGMA user_version=1;")?;
            let (bytes, entries): (u64, u64) = c.query_row(
                "SELECT COALESCE(SUM(size),0),COUNT(*) FROM thumbnails",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            self.bytes.store(bytes, Ordering::Relaxed);
            self.entries.store(entries, Ordering::Relaxed);
            self.needs_prune.store(true, Ordering::Relaxed);
            *connection = Some(c);
        }
        Ok(connection.as_mut().unwrap())
    }
    fn store(
        &self,
        connection: &mut Option<Connection>,
        token: &Token,
        pixels: &Payload,
        dimensions: [u32; 2],
    ) -> Result<()> {
        if self.budget.load(Ordering::Relaxed) == 0
            || token.epoch != self.epoch.load(Ordering::Relaxed)
            || Stamp::read(&token.source).as_ref() != Some(&token.stamp)
        {
            return Ok(());
        }
        let size = match pixels {
            Payload::Pixels(pixels) => [pixels.width(), pixels.height()],
            Payload::Encoded { size, .. } => *size,
        };
        ensure!(
            size[0] <= token.kind.cap() && size[1] <= token.kind.cap(),
            "Invalid thumbnail size"
        );
        let encoded;
        let png = match pixels {
            Payload::Pixels(pixels) => {
                encoded = encode(pixels)?;
                &encoded
            }
            Payload::Encoded { blob, .. } => blob,
        };
        let budget = self.budget.load(Ordering::Relaxed);
        if png.len() > budget.min(MAX_BLOB) && budget > 0 {
            // Too large to keep: reported instead of vanishing silently.
            self.skipped.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        if budget == 0
            || token.epoch != self.epoch.load(Ordering::Relaxed)
            || Stamp::read(&token.source).as_ref() != Some(&token.stamp)
        {
            return Ok(());
        }
        let c = self.writer(connection)?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let used = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_micros()
            .min(i64::MAX as u128) as i64;
        tx.execute("INSERT OR REPLACE INTO thumbnails(key,png,size,source_width,source_height,used) VALUES(?1,?2,?3,?4,?5,MAX(?6,COALESCE((SELECT MAX(used) FROM thumbnails),0)+1))",params![token.key,png,png.len() as i64,dimensions[0],dimensions[1],used])?;
        let (bytes, entries) = trim(&tx, budget)?;
        tx.commit()?;
        self.bytes.store(bytes, Ordering::Relaxed);
        self.entries.store(entries, Ordering::Relaxed);
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn prune(&self, connection: &mut Option<Connection>) -> Result<()> {
        let budget = self.budget.load(Ordering::Relaxed);
        if budget == 0 {
            return Ok(());
        }
        let c = self.writer(connection)?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (bytes, entries) = trim(&tx, budget)?;
        tx.commit()?;
        self.bytes.store(bytes, Ordering::Relaxed);
        self.entries.store(entries, Ordering::Relaxed);
        c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }
    fn remove_database(&self) -> Result<()> {
        if let Some(root) = &self.root {
            let path = root.join(DATABASE);
            if path.exists() {
                let cleared = (|| -> rusqlite::Result<()> {
                    let c = Connection::open(&path)?;
                    c.busy_timeout(Duration::from_millis(100))?;
                    c.execute("DELETE FROM thumbnails", [])?;
                    let _ = c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;");
                    Ok(())
                })();
                match cleared {
                    Ok(()) => {
                        self.bytes.store(0, Ordering::Relaxed);
                        self.entries.store(0, Ordering::Relaxed);
                        return Ok(());
                    }
                    Err(rusqlite::Error::SqliteFailure(e, _))
                        if matches!(
                            e.code,
                            rusqlite::ErrorCode::DatabaseCorrupt
                                | rusqlite::ErrorCode::NotADatabase
                        ) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            for name in [DATABASE, "cache-v1.sqlite3-wal", "cache-v1.sqlite3-shm"] {
                match fs::remove_file(root.join(name)) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        self.bytes.store(0, Ordering::Relaxed);
        self.entries.store(0, Ordering::Relaxed);
        Ok(())
    }
}
fn busy(error: &anyhow::Error) -> bool {
    error.downcast_ref::<rusqlite::Error>().is_some_and(|e| {
        matches!(e, rusqlite::Error::SqliteFailure(code, _)
            if matches!(code.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
    })
}
fn trim(c: &Connection, budget: usize) -> Result<(u64, u64)> {
    let (mut bytes, mut entries): (i64, i64) = c.query_row(
        "SELECT COALESCE(SUM(size),0),COUNT(*) FROM thumbnails",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    // Large budgets hold more entries (first views of big folders).
    let max_entries = MAX_ENTRIES.max((budget / (64 * 1024)).min(i64::MAX as usize) as i64);
    while bytes > budget.min(i64::MAX as usize) as i64 || entries > max_entries {
        let (key, size): (Vec<u8>, i64) = c.query_row(
            "SELECT key,size FROM thumbnails ORDER BY used,key LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        c.execute("DELETE FROM thumbnails WHERE key=?1", [key])?;
        bytes -= size;
        entries -= 1;
    }
    Ok((bytes.max(0) as u64, entries.max(0) as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"source pixels").unwrap();
        path
    }
    fn pixels() -> RgbaImage {
        RgbaImage::from_fn(8, 12, |x, y| {
            image::Rgba([(x * 31) as u8, (y * 17) as u8, 90, (x * 20 + 80) as u8])
        })
    }
    fn pause(cache: &ThumbnailCache) -> Sender<()> {
        let (entered, ack) = bounded(1);
        let (release, wait) = bounded(1);
        cache
            .tx
            .as_ref()
            .unwrap()
            .send(Command::Pause(entered, wait))
            .unwrap();
        ack.recv_timeout(Duration::from_secs(5)).unwrap();
        release
    }
    #[test]
    fn opaque_tiles_use_compact_jpeg_and_alpha_tiles_stay_exact_png() {
        let opaque = RgbaImage::from_fn(512, 341, |x, y| {
            image::Rgba([(x / 2) as u8, (y / 2) as u8, ((x + y) / 4) as u8, 255])
        });
        let jpeg = encode(&opaque).unwrap();
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]));
        let png_size = {
            let mut out = Vec::new();
            PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Sub)
                .write_image(opaque.as_raw(), 512, 341, image::ExtendedColorType::Rgba8)
                .unwrap();
            out.len()
        };
        assert!(jpeg.len() * 3 < png_size, "{} vs {png_size}", jpeg.len());
        let mut alpha = opaque.clone();
        alpha.put_pixel(3, 4, image::Rgba([1, 2, 3, 77]));
        let png = encode(&alpha).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        let back = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(back, alpha);
        // Both formats round-trip through the cache on disk.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("thumbnails");
        let a = source(dir.path(), "a.png");
        let b = source(dir.path(), "b.png");
        let cache = ThumbnailCache::new(Some(root), 64);
        cache.store(
            cache.token(&a, Kind::Thumbnail).unwrap(),
            &opaque,
            [3000, 2000],
        );
        cache.store(
            cache.token(&b, Kind::Thumbnail).unwrap(),
            &alpha,
            [3000, 2000],
        );
        cache.flush();
        let (got, _) = cache
            .load(&cache.token(&b, Kind::Thumbnail).unwrap(), |_| true)
            .unwrap()
            .unwrap();
        assert_eq!(got, alpha);
        let (got, dims) = cache
            .load(&cache.token(&a, Kind::Thumbnail).unwrap(), |_| true)
            .unwrap()
            .unwrap();
        assert_eq!(dims, [3000, 2000]);
        assert_eq!(got.dimensions(), opaque.dimensions());
    }
    #[test]
    fn large_images_wait_for_the_writer_instead_of_dropping_older_ones() {
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<_> = (0..7)
            .map(|n| source(dir.path(), &format!("{n}.png")))
            .collect();
        // ~15 MiB each: only four fit the pending buffer at once.
        let pixels = RgbaImage::from_pixel(2560, 1440, image::Rgba([40, 80, 120, 255]));
        // Plain `store` under a stalled writer pushes the oldest out.
        let cache = ThumbnailCache::new(Some(dir.path().join("dropping")), 512);
        let release = pause(&cache);
        for path in &paths {
            cache.store(
                cache.token(path, Kind::Proxy).unwrap(),
                &pixels,
                [5000, 2812],
            );
        }
        assert_eq!(cache.stats().skipped, 3);
        release.send(()).unwrap();
        cache.flush();
        assert_eq!(cache.stats().writes, 4);
        drop(cache);
        // `store_when_room` encodes in the producer: a burst far beyond four
        // raw images waits compactly behind a stalled writer and none is lost.
        let cache = ThumbnailCache::new(Some(dir.path().join("waiting")), 512);
        let burst: Vec<_> = (0..10)
            .map(|n| source(dir.path(), &format!("burst{n}.png")))
            .collect();
        let release = pause(&cache);
        let started = Instant::now();
        for path in &burst {
            cache.store_when_room(
                cache.token(path, Kind::Proxy).unwrap(),
                &pixels,
                [5000, 2812],
                Duration::from_secs(30),
            );
        }
        assert!(started.elapsed() < Duration::from_secs(20));
        assert_eq!(cache.stats().skipped, 0);
        assert!(cache.core.pending.lock().unwrap().bytes < PENDING_BYTES / 4);
        release.send(()).unwrap();
        cache.flush();
        assert_eq!(cache.stats().writes, 10);
        assert_eq!(cache.stats().skipped, 0);
        let token = cache.token(&burst[7], Kind::Proxy).unwrap();
        let (stored, dimensions) = cache.load(&token, |_| true).unwrap().unwrap();
        assert_eq!([stored.width(), stored.height()], [2560, 1440]);
        assert_eq!(dimensions, [5000, 2812]);
        // A sealed (closing) cache never makes the producer wait.
        cache.seal();
        let started = Instant::now();
        cache.store_when_room(
            cache.token(&paths[0], Kind::Proxy).unwrap(),
            &pixels,
            [5000, 2812],
            Duration::from_secs(30),
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    #[test]
    fn stalled_writer_retains_a_complete_64_tile_page_and_flushes_on_close() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("thumbnails");
        let paths: Vec<_> = (0..64)
            .map(|n| source(dir.path(), &format!("{n}.png")))
            .collect();
        let cache = ThumbnailCache::new(Some(root.clone()), 64);
        let release = pause(&cache);
        let pixels = RgbaImage::from_pixel(512, 512, image::Rgba([17, 31, 90, 255]));
        for path in &paths {
            cache.store(
                cache.token(path, Kind::Thumbnail).unwrap(),
                &pixels,
                [800, 1200],
            );
        }
        assert!(cache.tx.as_ref().unwrap().is_full());
        assert_eq!(cache.stats().pending, 64);
        assert_eq!(cache.stats().writes, 0);
        assert_eq!(cache.stats().skipped, 0);
        assert_eq!(cache.core.pending.lock().unwrap().bytes, PENDING_BYTES);
        cache.seal();
        release.send(()).unwrap();
        cache.flush();
        assert_eq!(cache.stats().pending, 0);
        assert_eq!(cache.stats().writes, 64);
        assert_eq!(cache.stats().errors, 0);
        drop(cache);
        let cache = ThumbnailCache::new(Some(root), 64);
        for path in &paths {
            let (actual, dimensions) = cache
                .load(&cache.token(path, Kind::Thumbnail).unwrap(), |_| true)
                .unwrap()
                .unwrap();
            // Opaque tiles are stored as JPEG: near, not bit-exact.
            assert!(actual
                .as_raw()
                .iter()
                .zip(pixels.as_raw())
                .all(|(a, b)| a.abs_diff(*b) <= 4));
            assert_eq!(dimensions, [800, 1200]);
        }
        assert_eq!(cache.stats().hits, 64);
        assert_eq!(cache.stats().writes, 0);
    }
    #[test]
    fn retry_buffer_coalesces_duplicates_and_bounds_memory_and_entry_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = source(dir.path(), "a.png");
        // Tokens require an enabled root; the buffer itself needs no writer.
        let token_cache = ThumbnailCache::new(Some(dir.path().join("cache")), 64);
        let mut pending = PendingWrites::default();
        let pixels = RgbaImage::new(1024, 1024);
        for _ in 0..3 {
            assert_eq!(
                pending.insert(PendingWrite {
                    token: token_cache.token(&path, Kind::Preview).unwrap(),
                    pixels: Payload::Pixels(pixels.clone()),
                    dimensions: [800, 1200]
                }),
                0
            );
        }
        assert_eq!(pending.jobs.len(), 1);
        assert_eq!(pending.bytes, 4 * 1024 * 1024);
        let mut skipped = 0;
        for n in 0..30 {
            let path = source(dir.path(), &n.to_string());
            skipped += pending.insert(PendingWrite {
                token: token_cache.token(&path, Kind::Preview).unwrap(),
                pixels: Payload::Pixels(pixels.clone()),
                dimensions: [800, 1200],
            });
        }
        assert_eq!(pending.bytes, PENDING_BYTES);
        assert_eq!(pending.jobs.len(), 16);
        assert!(skipped > 0);
        while !pending.jobs.is_empty() {
            pending.remove(0);
        }
        for n in 0..PENDING_ENTRIES + 4 {
            let path = source(dir.path(), &n.to_string());
            pending.insert(PendingWrite {
                token: token_cache.token(&path, Kind::Thumbnail).unwrap(),
                pixels: Payload::Pixels(RgbaImage::new(1, 1)),
                dimensions: [1, 1],
            });
        }
        assert_eq!(pending.jobs.len(), PENDING_ENTRIES);
        assert_eq!(pending.bytes, PENDING_ENTRIES * 4);
    }
    #[test]
    fn clearing_a_stalled_backlog_cannot_repopulate_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = source(dir.path(), "source.png");
        let cache = Arc::new(ThumbnailCache::new(Some(dir.path().join("cache")), 64));
        let release = pause(&cache);
        cache.store(
            cache.token(&path, Kind::Thumbnail).unwrap(),
            &pixels(),
            [80, 120],
        );
        let clearing = cache.clone();
        let thread = std::thread::spawn(move || clearing.clear().unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        while cache.core.epoch.load(Ordering::Relaxed) == 0 {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        release.send(()).unwrap();
        thread.join().unwrap();
        cache.flush();
        assert_eq!(cache.stats().pending, 0);
        assert_eq!(cache.stats().entries, 0);
        assert_eq!(cache.stats().writes, 0);
        assert_eq!(fs::read(path).unwrap(), b"source pixels");
    }
    #[test]
    fn transient_database_lock_retries_accepted_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("cache");
        let first = source(dir.path(), "first");
        let second = source(dir.path(), "second");
        let cache = ThumbnailCache::new(Some(root.clone()), 64);
        cache.store(
            cache.token(&first, Kind::Thumbnail).unwrap(),
            &pixels(),
            [80, 120],
        );
        cache.flush();
        let blocker = Connection::open(root.join(DATABASE)).unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        cache.store(
            cache.token(&second, Kind::Thumbnail).unwrap(),
            &pixels(),
            [80, 120],
        );
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            blocker.execute_batch("ROLLBACK").unwrap();
        });
        cache.flush();
        release.join().unwrap();
        assert_eq!(cache.stats().writes, 2);
        assert_eq!(cache.stats().errors, 0);
        assert_eq!(cache.stats().pending, 0);
    }
    #[test]
    fn permanent_cache_write_failures_are_reported_without_leaving_pending_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let path = source(dir.path(), "source.png");
        let root = dir.path().join("not-a-directory");
        fs::write(&root, b"preserve").unwrap();
        let cache = ThumbnailCache::new(Some(root.clone()), 64);
        cache.store(
            cache.token(&path, Kind::Thumbnail).unwrap(),
            &pixels(),
            [80, 120],
        );
        cache.flush();
        assert_eq!(cache.stats().errors, 1);
        assert_eq!(cache.stats().pending, 0);
        assert_eq!(cache.stats().writes, 0);
        assert_eq!(fs::read(root).unwrap(), b"preserve");
    }
    #[test]
    fn cache_reopens_with_exact_oriented_pixels_and_original_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = source(dir.path(), "photo.jpg");
        let root = dir.path().join("thumbnails");
        {
            let cache = ThumbnailCache::new(Some(root.clone()), 64);
            cache.store(
                cache.token(&path, Kind::Thumbnail).unwrap(),
                &pixels(),
                [800, 1200],
            );
            cache.flush();
            assert_eq!(cache.stats().writes, 1);
        }
        let cache = ThumbnailCache::new(Some(root), 64);
        let token = cache.token(&path, Kind::Thumbnail).unwrap();
        let (actual, dimensions) = cache.load(&token, |n| n == 8 * 12 * 4).unwrap().unwrap();
        assert_eq!(actual, pixels());
        assert_eq!(dimensions, [800, 1200]);
        assert_eq!(cache.stats().hits, 1);
        assert_ne!(token.key, cache.token(&path, Kind::Preview).unwrap().key);
    }
    #[test]
    fn changed_sources_and_stale_writes_cannot_supply_cached_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let path = source(dir.path(), "a.png");
        let cache = ThumbnailCache::new(Some(dir.path().join("thumbnails")), 64);
        let old = cache.token(&path, Kind::Thumbnail).unwrap();
        cache.store(
            cache.token(&path, Kind::Thumbnail).unwrap(),
            &pixels(),
            [80, 120],
        );
        cache.flush();
        fs::write(&path, b"changed source pixels and dimensions").unwrap();
        assert!(cache.load(&old, |_| true).unwrap().is_none());
        let fresh = cache.token(&path, Kind::Thumbnail).unwrap();
        assert_ne!(old.key, fresh.key);
        assert!(cache.load(&fresh, |_| true).unwrap().is_none());
        cache.store(old, &pixels(), [80, 120]);
        cache.flush();
        assert_eq!(cache.stats().writes, 1);
    }
    #[test]
    fn large_budgets_are_kept_and_low_disk_space_stops_writes() {
        let dir = tempfile::tempdir().unwrap();
        let a = source(dir.path(), "a");
        let cache = ThumbnailCache::new(Some(dir.path().join("proxies")), 8192);
        assert_eq!(cache.budget(), 8192 << 20);
        cache.configure(UNLIMITED_MB);
        assert_eq!(cache.budget() as usize, UNLIMITED_MB << 20);
        assert!(cache.has_room());
        // The disk reports less free space than the cache must leave.
        *cache.core.space.lock().unwrap() = Some((Instant::now(), Some(10 << 20)));
        cache.keep_free(1 << 30);
        assert!(!cache.space_ok() && !cache.has_room());
        cache.store(cache.token(&a, Kind::Proxy).unwrap(), &pixels(), [80, 120]);
        cache.flush();
        assert_eq!((cache.stats().entries, cache.stats().skipped), (0, 1));
        cache.keep_free(0);
        assert!(cache.has_room());
        cache.store(cache.token(&a, Kind::Proxy).unwrap(), &pixels(), [80, 120]);
        cache.flush();
        assert_eq!(cache.stats().entries, 1);
    }
    #[test]
    fn lru_budget_evicts_the_untouched_entry_and_zero_disables_disk_access() {
        let dir = tempfile::tempdir().unwrap();
        let a = source(dir.path(), "a");
        let b = source(dir.path(), "b");
        let c = source(dir.path(), "c");
        let cache = ThumbnailCache::new(Some(dir.path().join("thumbnails")), 64);
        cache.store(
            cache.token(&a, Kind::Thumbnail).unwrap(),
            &pixels(),
            [80, 120],
        );
        cache.flush();
        let size = cache.stats().bytes;
        cache
            .core
            .budget
            .store(size as usize * 2, Ordering::Relaxed);
        cache.store(
            cache.token(&b, Kind::Thumbnail).unwrap(),
            &pixels(),
            [80, 120],
        );
        cache.flush();
        assert!(cache
            .load(&cache.token(&a, Kind::Thumbnail).unwrap(), |_| true)
            .unwrap()
            .is_some());
        cache.flush();
        cache.store(
            cache.token(&c, Kind::Thumbnail).unwrap(),
            &pixels(),
            [80, 120],
        );
        cache.flush();
        assert_eq!(cache.stats().entries, 2);
        assert!(cache.stats().bytes <= size * 2);
        assert!(cache
            .load(&cache.token(&b, Kind::Thumbnail).unwrap(), |_| true)
            .unwrap()
            .is_none());
        assert!(cache
            .load(&cache.token(&a, Kind::Thumbnail).unwrap(), |_| true)
            .unwrap()
            .is_some());
        cache.configure(0);
        assert!(cache.token(&c, Kind::Thumbnail).is_none());
    }
    #[test]
    fn corrupt_cache_is_optional_and_clear_rebuilds_it_without_touching_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("thumbnails");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(DATABASE), b"damaged disposable database").unwrap();
        fs::write(root.join("keep.txt"), b"keep").unwrap();
        let path = source(dir.path(), "source.png");
        let cache = ThumbnailCache::new(Some(root.clone()), 64);
        let old = cache.token(&path, Kind::Thumbnail).unwrap();
        assert!(cache.load(&old, |_| true).unwrap().is_none());
        cache.clear().unwrap();
        cache.store(old, &pixels(), [80, 120]);
        cache.flush();
        assert_eq!(cache.stats().writes, 0);
        cache.store(
            cache.token(&path, Kind::Thumbnail).unwrap(),
            &pixels(),
            [80, 120],
        );
        cache.flush();
        assert!(cache
            .load(&cache.token(&path, Kind::Thumbnail).unwrap(), |_| true)
            .unwrap()
            .is_some());
        cache.clear().unwrap();
        assert_eq!(cache.stats().entries, 0);
        assert_eq!(fs::read(root.join("keep.txt")).unwrap(), b"keep");
        assert_eq!(fs::read(path).unwrap(), b"source pixels");
    }
    #[test]
    #[ignore = "manual benchmark: cargo test --release bench_first_view -- --ignored --nocapture"]
    fn bench_first_view() {
        let dir = tempfile::tempdir().unwrap();
        let path = source(dir.path(), "photo.jpg");
        let root = dir.path().join("proxies");
        let mut seed = 1u32;
        // Photo-like: smooth shapes with mild sensor noise.
        let tile = RgbaImage::from_fn(PROXY_SIDE, PROXY_SIDE * 2 / 3, |x, y| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let n = (seed >> 29) as f32;
            let v = ((x as f32 / 90.).sin() * (y as f32 / 70.).cos() * 100. + 128. + n) as u8;
            image::Rgba([v, v / 2 + (x / 20) as u8, (y / 8) as u8, 255])
        });
        let cache = ThumbnailCache::new(Some(root), 1024);
        cache.store_when_room(
            cache.token(&path, Kind::Proxy).unwrap(),
            &tile,
            [6000, 4000],
            Duration::from_secs(5),
        );
        cache.flush();
        println!("stored first view: {} KiB", cache.stats().bytes / 1024);
        let token = cache.token(&path, Kind::Proxy).unwrap();
        for _ in 0..5 {
            let started = Instant::now();
            let blob = cache.core.read(&token).unwrap().0;
            let read = started.elapsed();
            let started = Instant::now();
            let (pixels, _) = cache.load(&token, |_| true).unwrap().unwrap();
            println!(
                "first view {}x{}: database read {:.1} ms ({} KiB), read + decode to RGBA {:.1} ms",
                pixels.width(),
                pixels.height(),
                read.as_secs_f64() * 1e3,
                blob.len() / 1024,
                started.elapsed().as_secs_f64() * 1e3
            );
        }
    }
}
