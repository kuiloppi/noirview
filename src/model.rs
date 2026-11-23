mod view_build;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::{
    collections::{btree_map::Entry, BTreeMap, BTreeSet, HashMap, VecDeque},
    ops::Deref,
    path::{Path, PathBuf},
};
pub use view_build::{Retire, ViewBuild};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Order {
    #[default]
    Sequential,
    Random,
    Shuffle,
}
impl Order {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sequential => "In order",
            Self::Random => "Random",
            Self::Shuffle => "Shuffle (no repeats)",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisplayMode {
    Fit,
    Fill,
    ShrinkOnly,
}
impl DisplayMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fit => "Fit whole image",
            Self::Fill => "Fill window (crop)",
            Self::ShrinkOnly => "Shrink only",
        }
    }
}
/// Standard decodes what is on screen and its neighbours. Aggressive also
/// fills the thumbnail cache for the whole open folder in the background.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoadingMode {
    #[default]
    Standard,
    Aggressive,
}
impl LoadingMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Aggressive => "Aggressive",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Upscale {
    #[default]
    Linear,
    Nearest,
    Lanczos,
    Cas,
    Onnx,
}
impl Upscale {
    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Smooth · GPU",
            Self::Nearest => "Pixel / nearest · GPU",
            Self::Lanczos => "Lanczos · background CPU",
            Self::Cas => "Adaptive sharpening · background CPU",
            Self::Onnx => "AI / ESRGAN · ONNX",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterMode {
    #[default]
    All,
    Any,
    Exclude,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhotoMeta {
    pub tags: BTreeSet<String>,
    pub favorite: bool,
    pub elite: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    /// Explicit NoirView replacement/move; bind the resulting object on the writer.
    #[serde(skip)]
    pub rebound_from: Option<String>,
}
/// Read access behaves like a map; every mutable access records the affected key.
/// A save snapshots only these records, never clones the complete collection.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Photos {
    records: BTreeMap<PathBuf, PhotoMeta>,
    #[serde(skip)]
    changed: BTreeSet<PathBuf>,
    #[serde(skip)]
    pub(crate) marks_dirty: BTreeSet<PathBuf>,
    #[serde(skip)]
    pub(crate) removed_marks: Vec<(PathBuf, String)>,
}
impl Deref for Photos {
    type Target = BTreeMap<PathBuf, PhotoMeta>;
    fn deref(&self) -> &Self::Target {
        &self.records
    }
}
impl Photos {
    pub fn entry(&mut self, path: PathBuf) -> Entry<'_, PathBuf, PhotoMeta> {
        self.changed.insert(path.clone());
        self.marks_dirty.insert(path.clone());
        self.records.entry(path)
    }
    pub fn insert(&mut self, path: PathBuf, meta: PhotoMeta) -> Option<PhotoMeta> {
        self.changed.insert(path.clone());
        self.marks_dirty.insert(path.clone());
        self.records.insert(path, meta)
    }
    pub fn remove(&mut self, path: &Path) -> Option<PhotoMeta> {
        self.changed.insert(path.into());
        if let Some(id) = self.records.get(path).and_then(|m| m.identity.clone()) {
            self.removed_marks.push((path.into(), id));
        }
        self.records.remove(path)
    }
    pub fn clear_changes(&mut self) {
        self.changed.clear();
        self.marks_dirty.clear();
        self.removed_marks.clear();
    }
    pub fn take_changes(&mut self) -> BTreeMap<PathBuf, Option<PhotoMeta>> {
        std::mem::take(&mut self.changed)
            .into_iter()
            .map(|p| {
                let meta = self.records.get_mut(&p).map(|m| {
                    let copy = m.clone();
                    m.rebound_from = None;
                    copy
                });
                (p, meta)
            })
            .collect()
    }
}

/// How hard the folder watcher works: Gentle rests as long as each image
/// took (half the disk time), Balanced a third of that, Fast not at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WatchPace {
    #[default]
    Gentle,
    Balanced,
    Fast,
}
impl WatchPace {
    pub const ALL: [Self; 3] = [Self::Gentle, Self::Balanced, Self::Fast];
    pub fn label(self) -> &'static str {
        match self {
            Self::Gentle => "Gentle",
            Self::Balanced => "Balanced",
            Self::Fast => "Fast",
        }
    }
}
/// A folder whose thumbnails and keywords are kept up to date in the
/// background (Settings → Library → Watched folders).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchedFolder {
    pub path: PathBuf,
    #[serde(default = "yes")]
    pub subfolders: bool,
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub fit: bool,
    pub display_mode: Option<DisplayMode>,
    pub recursive: bool,
    pub hover: bool,
    /// Hover activation for the floating trash button; `None` in settings
    /// saved before 0.1.45 (then taken from `hover` at load).
    pub hover_trash: Option<bool>,
    /// Hover activation for the Settings window's controls (as above).
    pub hover_settings: Option<bool>,
    /// The floating Move button activates by hovering (as `hover` until
    /// changed).
    pub hover_move: Option<bool>,
    pub hover_ms: u64,
    pub destructive_ms: u64,
    /// Play animated GIFs in the single image view and the big image from
    /// the grid (grid thumbnails show the first frame).
    pub animate_gif: bool,
    /// Under grid images: only ♥ / ★ instead of "♥ Favorite" / "★ Elite".
    pub grid_marks_icons: bool,
    /// The green ring that fills up before a hovered control activates.
    #[serde(default = "yes")]
    pub hover_ring: bool,
    /// Most memory (MB of RGBA frames) one animation may use.
    pub animation_mb: usize,
    pub bar_edge_px: f32,
    pub bar_delay_ms: u64,
    pub bar_width: f32,
    #[serde(default)]
    pub tags_max_width: Option<f32>,
    pub bar_height: f32,
    pub bar_bottom: f32,
    /// The toolbar's distance from the bottom in the grid; `None` in settings
    /// saved before 0.1.46 (then `bar_bottom` at load).
    pub bar_bottom_grid: Option<f32>,
    /// The toolbar stays open in the image view (the bottom edge and Escape
    /// do not close it).
    pub bar_locked: bool,
    /// The same for the grid page.
    pub bar_locked_grid: bool,
    /// One lock for both: `bar_locked` applies everywhere.
    pub bar_lock_universal: bool,
    pub bar_offset: f32,
    pub bar_color: [u8; 4],
    pub edges: bool,
    pub side_px: f32,
    pub side_ms: u64,
    pub top_delete: bool,
    pub top_px: f32,
    pub top_ms: u64,
    pub confirm_delete: bool,
    pub magnifier: bool,
    pub lens_px: f32,
    pub lens_zoom: f32,
    pub lens_border: bool,
    pub lens_soft_edges: bool,
    pub lens_feather_px: f32,
    pub lens_full_window: bool,
    pub jpeg_preview: bool,
    pub order: Order,
    pub wheel_up_next: bool,
    /// Random order: both wheel directions show a new random image instead
    /// of stepping back through the history.
    pub random_wheel_both: bool,
    pub history_limit: usize,
    pub prefetch_ahead: usize,
    pub prefetch_behind: usize,
    pub slideshow_secs: f32,
    pub slideshow_order: Order,
    pub slideshow_side_px: f32,
    pub slideshow_side_ms: u64,
    pub slideshow_top_px: f32,
    pub slideshow_top_ms: u64,
    pub keep_selected_tags: bool,
    pub thumbnail_count: usize,
    /// Round the grid page size up to the next count with no empty cells.
    pub grid_round_full: bool,
    pub grid_controls_top: bool,
    /// The grid's controls as an icon bar in the toolbar's style instead of
    /// the row of text buttons.
    pub grid_bar_icons: bool,
    /// Each bar's button order and places ("toolbar", "grid", "big"),
    /// only once changed in Settings → Display → Bar buttons.
    pub bar_buttons: BTreeMap<String, Vec<crate::bar_layout::Slot>>,
    /// The Move dialog keeps the original file names by default (off: the
    /// eight-digit sequence).
    pub move_keep_names: bool,
    /// Width of the sequence names of moved images (001.jpg).
    pub move_digits: usize,
    /// Sequence names take the destination's free numbers first.
    pub move_fill_gaps: bool,
    /// Open the grid at the page of the image shown in the image view.
    pub grid_open_at_current: bool,
    /// The grid and the image view keep their own filter (off: one filter).
    #[serde(default = "yes")]
    pub filter_per_view: bool,
    /// Random / Shuffle page order: both directions draw a new page.
    pub grid_random_both: bool,
    /// Previous / Next page arrows in the grid's controls.
    #[serde(default = "yes")]
    pub grid_page_arrows: bool,
    /// The icon bar is always shown (the grid leaves room for it); off: it
    /// opens at the bottom edge like the toolbar.
    pub grid_bar_fixed: bool,
    /// The icon bar's distance from the bottom (px).
    pub grid_bar_bottom: f32,
    /// With the icon bar, the floating toolbar is not shown on the grid page
    /// (otherwise it is stacked above the icon bar).
    pub grid_hide_toolbar: bool,
    /// The same for the text buttons (off by default).
    pub grid_hide_toolbar_text: bool,
    /// The grid's controls keep their width (page number, image count and
    /// status take the room of their widest value).
    #[serde(default = "yes")]
    pub grid_bar_fixed_width: bool,
    /// Icon colours of the grid icon bar's buttons by key; missing = default.
    pub grid_bar_colors: std::collections::BTreeMap<String, [u8; 4]>,
    /// Text colours of the grid's text buttons by key; missing = default.
    pub grid_text_colors: std::collections::BTreeMap<String, [u8; 4]>,
    /// Text colours of the grid's text buttons by key; missing = default.
    pub grid_text_font_colors: std::collections::BTreeMap<String, [u8; 4]>,
    pub grid_show_marks: bool,
    pub grid_keep_tag_mode: bool,
    pub pregenerate_thumbnails: bool,
    /// How much background work and disk space image loading may use.
    pub loading_mode: LoadingMode,
    /// Aggressive: the background thumbnail pass continues while NoirView is used.
    pub cache_while_active: bool,
    /// Aggressive: the background thumbnail pass continues in an inactive window.
    pub cache_when_unfocused: bool,
    /// Disk budget for screen-sized first views of large images (Aggressive).
    pub proxy_cache_mb: usize,
    /// Images whose longer side is at least this get a first view (pixels).
    pub proxy_min_side: u32,
    pub diagnostic_log: bool,
    /// Draws the window's edges, a point grid and the pointer as egui sees
    /// them, with window / surface sizes; logs changes to
    /// data/noirview-window.log (resize problems on Windows).
    pub window_diagnostics: bool,
    /// Mode the grid opens in: selection, Normal, or the last one used.
    pub grid_start_mode: GridStartMode,
    pub grid_last_normal: bool,
    /// In the grid's big-image view: shorter press = back, longer = magnifier.
    pub grid_hold_ms: u64,
    pub gestures: bool,
    pub gesture: crate::gesture::Gesture,
    pub gesture_min_px: f32,
    pub gesture_max_ms: u64,
    pub gesture_trail: bool,
    pub gesture_trail_color: [u8; 4],
    pub gesture_trail_width: f32,
    /// The closing gesture made over a grid tile opens that image big.
    pub gesture_opens_tile: bool,
    /// Feedback for Select all (view / folder) and Clear selection.
    pub selection_flash: bool,
    pub selection_toast: bool,
    pub selection_toast_size: f32,
    /// Size of the large message box in pixels (clamped to the window).
    pub selection_toast_width: f32,
    pub selection_toast_height: f32,
    /// Grid hover preview: a larger image after resting on a tile.
    pub grid_hover_preview: bool,
    pub hover_preview_ms: u64,
    /// Only a centred circle of the tile starts the hover preview.
    pub hover_circle: bool,
    /// Circle diameter as a percentage of the tile's shorter side.
    pub hover_circle_pct: f32,
    /// The ring appears when the pointer is this close to the circle (px),
    /// fading in on the way (`hover_circle_fade`), at this opacity.
    pub hover_circle_reveal_px: f32,
    pub hover_circle_fade: bool,
    pub hover_circle_opacity: f32,
    /// Show the circle on every tile while the hover preview is on.
    pub hover_circle_always: bool,
    /// Ring line width (px).
    pub hover_circle_width: f32,
    /// Big image from the grid: moving the pointer while pressed starts the
    /// magnifier at once (it is no longer a click); `magnify_at_once` starts
    /// it on the press itself (a quick click still returns to the grid).
    pub magnify_on_move: bool,
    pub magnify_at_once: bool,
    /// Ask the graphics driver to queue one frame instead of two (applies
    /// after a restart).
    pub low_latency_display: bool,
    /// Graphics backend after a restart: "" automatic, "dx12", "vulkan", "opengl".
    pub graphics_backend: String,
    /// Restore the window's size and position at start (after a restart).
    #[serde(default = "yes")]
    pub remember_window: bool,
    /// While the next image loads, keep the previous one on screen instead of
    /// an empty view.
    pub keep_previous_image: bool,
    /// Forget the last deletion when NoirView closes (Restore deleted then
    /// has nothing to restore at the next start).
    pub forget_deletion_on_close: bool,
    /// Ask before Restore deleted moves images back.
    pub confirm_restore: bool,
    /// Folders kept for quick opening (holding Open folder lists them).
    pub favorite_folders: Vec<FavoriteFolder>,
    /// The favorites list: "buttons", "text" (/name) or "icon" (folder icon
    /// and name).
    pub favorite_style: String,
    /// In the image view with In order / Shuffle: say when every image of
    /// the list has been shown (a notice, or a dialog).
    pub round_notice: bool,
    pub round_notice_dialog: bool,
    /// The round notice: text size, colour and place ("top", "center",
    /// "bottom").
    pub round_notice_size: f32,
    pub round_notice_color: [u8; 4],
    pub round_notice_place: String,
    /// How long the round notice stays (seconds), its opacity, and after how
    /// many new images it goes (0: only the time).
    pub round_notice_secs: f32,
    pub round_notice_opacity: f32,
    pub round_notice_images: u32,
    /// The round notice fades in and out over this many milliseconds.
    pub round_notice_fade: bool,
    pub round_notice_fade_ms: u32,
    /// Which way the pointer must move to close the hover preview: "any",
    /// "up", "down", "left" or "right".
    pub preview_close_move_dir: String,
    /// Folder paths shown with forward slashes (D:/Photos) instead of
    /// backslashes.
    pub path_forward_slashes: bool,
    /// The image view's title shows the folder before the image name.
    pub image_title_folder: bool,
    /// Where each kind of file dialog was last left (Windows; screen pixels).
    pub dialog_places: BTreeMap<String, [i32; 2]>,
    /// Named sets of display and interaction settings, and the one in use
    /// (empty: none).
    pub profiles: Vec<Profile>,
    pub active_profile: String,
    /// The floating Move button (single image view): its place and colour.
    pub move_pos: Option<[f32; 2]>,
    pub move_color: [u8; 4],
    /// The marked images' Move grid (a grid of only them): its own page
    /// size and rounding, or one page going on downwards; the big image
    /// browses the marked images.
    pub move_grid_count: usize,
    pub move_grid_round_full: bool,
    pub move_grid_all: bool,
    pub move_grid_browse: bool,
    /// The Move grid's tiles show the "✔ MOVE" mark too; one page that
    /// moves a row at a time (all on one page).
    pub move_grid_marks: bool,
    pub move_grid_row: bool,
    /// The × on the tiles there: its corner ("top_left", "top_right",
    /// "bottom_left", "bottom_right"), fill colour, size, opacity.
    pub move_x_corner: String,
    /// Moved in from the corner (px, right / down from a left / top corner).
    pub move_x_offset: [f32; 2],
    /// The "✔ MOVE" mark on the Move grid's tiles: corner and offset.
    pub move_mark_corner: String,
    pub move_mark_offset: [f32; 2],
    /// A frame round the whole window while the Move grid is open.
    pub move_grid_frame: bool,
    pub move_grid_frame_width: f32,
    pub move_grid_frame_color: [u8; 4],
    pub move_grid_frame_opacity: f32,
    /// The Move grid's own values of the grid's settings (`MOVE_GRID_OWN`),
    /// and while it is open the grid's values it replaced.
    pub move_grid_own: serde_json::Map<String, serde_json::Value>,
    pub grid_own_saved: Option<serde_json::Map<String, serde_json::Value>>,
    pub move_x_color: [u8; 4],
    pub move_x_size: f32,
    pub move_x_opacity: f32,
    /// Images marked to move show their mark on the grid's tiles too.
    pub grid_move_marks: bool,
    /// Favorite folders open where browsing left them (kept per profile).
    pub remember_favorite_places: bool,
    /// Where browsing was in each favorite folder, per profile (key: the
    /// profile's name, "" without one).
    pub favorite_places: BTreeMap<String, BTreeMap<PathBuf, FolderPlace>>,
    /// The title after NOIRVIEW shares its baseline, raised by this many
    /// points (negative: lowered).
    pub title_raise: f32,
    /// Grid pages wrap around: before the first page comes the last one and
    /// after the last the first (In order pages).
    pub grid_page_wrap: bool,
    /// The grid's hover preview closes once the pointer has moved this far
    /// from where it opened (0 = off).
    pub preview_close_move_px: f32,
    /// The big image opened from the grid can be browsed (wheel, keys,
    /// edges); the grid page follows behind it.
    pub grid_viewer_browse: bool,
    /// The grid page's title: the first and last image name (or the first
    /// and "…"), with the folder and the number of images if chosen.
    pub grid_caption_range: bool,
    pub grid_caption_folder: bool,
    pub grid_caption_count: bool,
    /// In fullscreen the image view shows only the image: no top bar, image
    /// number or notices (the toolbar still opens at the bottom edge).
    pub fullscreen_image_only: bool,
    /// Keep thumbnails and keywords of the watched folders up to date.
    pub watch_folders: bool,
    pub watched_folders: Vec<WatchedFolder>,
    /// Also while NoirView is closed, started at Windows sign-in.
    pub watch_background: bool,
    /// Inside the open program: keep working while the user uses it, and
    /// while another program is in front.
    pub watch_while_active: bool,
    pub watch_when_unfocused: bool,
    pub watch_pace: WatchPace,
    /// Use the NTFS change journal (USN service) for catch-up when installed.
    pub watch_use_journal: bool,
    /// Top activation strip that closes the preview.
    pub preview_close_px: f32,
    pub preview_close_ms: u64,
    /// Preview size as a percentage of the grid area.
    pub preview_size: f32,
    /// Ways to close the hover preview (any combination).
    pub preview_close_top: bool,
    pub preview_close_left: bool,
    pub preview_close_right: bool,
    pub preview_close_click: bool,
    /// Normal mode keeps the trash/tag selection (otherwise it is cleared and
    /// can be brought back with Restore selection).
    pub normal_keeps_selection: bool,
    /// Grid page order: in order, Random or Shuffle.
    pub grid_page_order: Order,
    pub sort_key: crate::sorting::SortKey,
    pub sort_descending: bool,
    /// Quick page-size buttons in the grid controls.
    pub grid_presets: Vec<usize>,
    pub confirm_preset_remove: bool,
    pub grid_wheel_pages: bool,
    pub show_favorites_button: bool,
    pub thumbnail_cache_mb: usize,
    /// Folder for the thumbnail cache; empty keeps it next to the user data.
    pub thumbnail_cache_dir: PathBuf,
    pub delete_size: f32,
    pub delete_color: [u8; 4],
    pub delete_opacity: f32,
    pub delete_border: bool,
    pub delete_border_width: f32,
    pub delete_border_color: [u8; 4],
    pub close_angle: f32,
    pub close_size: f32,
    pub delete_pos: Option<[f32; 2]>,
    pub destination: PathBuf,
    pub verified_move: bool,
    pub music: Vec<PathBuf>,
    pub music_shuffle: bool,
    pub volume: f32,
    pub cache_mb: usize,
    pub upscale: Upscale,
    pub upscale_factor: u32,
    pub sharpen: f32,
    pub model_path: PathBuf,
    pub ort_library: PathBuf,
    pub tile_size: u32,
    pub backup_extra: PathBuf,
    pub backup_days: [bool; 7],
    pub backup_hour: u32,
    pub backup_minute: u32,
    pub backup_schedule: bool,
    pub backup_exit: bool,
    pub backup_keep: usize,
    pub shortcuts: BTreeMap<String, String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            fit: true,
            display_mode: None,
            recursive: false,
            hover: false,
            // `None`: settings saved before 0.1.45 take these from `hover`.
            hover_trash: None,
            hover_settings: None,
            hover_move: None,
            hover_ms: 750,
            destructive_ms: 1500,
            animate_gif: true,
            grid_marks_icons: false,
            hover_ring: true,
            animation_mb: 1024,
            bar_edge_px: 16.,
            bar_delay_ms: 350,
            bar_width: 680.,
            tags_max_width: Some(680.),
            bar_height: 56.,
            bar_bottom: 28.,
            bar_bottom_grid: None,
            bar_locked: false,
            bar_locked_grid: false,
            bar_lock_universal: false,
            bar_offset: 0.,
            bar_color: [25, 29, 36, 245],
            edges: false,
            side_px: 24.,
            side_ms: 650,
            top_delete: false,
            top_px: 20.,
            top_ms: 1200,
            confirm_delete: true,
            magnifier: true,
            lens_px: 240.,
            lens_zoom: 3.,
            lens_border: false,
            lens_soft_edges: false,
            lens_feather_px: 18.,
            lens_full_window: true,
            jpeg_preview: true,
            order: Order::Sequential,
            wheel_up_next: true,
            random_wheel_both: false,
            history_limit: DEFAULT_HISTORY_LIMIT,
            prefetch_ahead: 7,
            prefetch_behind: 3,
            slideshow_secs: 5.,
            slideshow_order: Order::Shuffle,
            slideshow_side_px: 24.,
            slideshow_side_ms: 650,
            slideshow_top_px: 20.,
            slideshow_top_ms: 800,
            keep_selected_tags: true,
            thumbnail_count: 12,
            grid_round_full: true,
            grid_controls_top: false,
            grid_bar_icons: true,
            bar_buttons: BTreeMap::new(),
            move_keep_names: false,
            move_digits: 3,
            move_fill_gaps: false,
            grid_open_at_current: false,
            filter_per_view: true,
            grid_random_both: false,
            grid_page_arrows: true,
            grid_bar_fixed: true,
            grid_bar_bottom: 12.,
            grid_hide_toolbar: true,
            grid_hide_toolbar_text: false,
            grid_bar_fixed_width: true,
            grid_bar_colors: Default::default(),
            grid_text_colors: Default::default(),
            grid_text_font_colors: Default::default(),
            graphics_backend: String::new(),
            remember_window: true,
            keep_previous_image: true,
            forget_deletion_on_close: false,
            confirm_restore: true,
            favorite_folders: vec![],
            favorite_style: "buttons".into(),
            round_notice: true,
            round_notice_dialog: false,
            round_notice_size: 28.,
            round_notice_color: [235, 240, 236, 255],
            round_notice_place: "center".into(),
            round_notice_secs: 3.,
            round_notice_opacity: 0.9,
            round_notice_images: 0,
            round_notice_fade: false,
            round_notice_fade_ms: 300,
            preview_close_move_dir: "any".into(),
            path_forward_slashes: false,
            image_title_folder: false,
            title_raise: 0.,
            dialog_places: BTreeMap::new(),
            profiles: Vec::new(),
            active_profile: String::new(),
            move_pos: None,
            move_color: [38, 84, 150, 235],
            move_grid_count: 12,
            move_grid_round_full: true,
            move_grid_all: false,
            move_grid_browse: true,
            move_grid_marks: false,
            move_grid_row: false,
            move_x_corner: "top_left".into(),
            move_x_offset: [0., 0.],
            move_mark_corner: "top_left".into(),
            move_mark_offset: [0., 30.],
            move_grid_frame: true,
            move_grid_frame_width: 4.,
            move_grid_frame_color: [90, 160, 255, 255],
            move_grid_frame_opacity: 0.8,
            move_grid_own: serde_json::Map::new(),
            grid_own_saved: None,
            move_x_color: [20, 24, 30, 255],
            move_x_size: 22.,
            move_x_opacity: 0.9,
            grid_move_marks: true,
            remember_favorite_places: false,
            favorite_places: BTreeMap::new(),
            grid_page_wrap: false,
            preview_close_move_px: 0.,
            grid_viewer_browse: false,
            grid_caption_range: true,
            grid_caption_folder: false,
            grid_caption_count: false,
            fullscreen_image_only: false,
            grid_show_marks: true,
            grid_keep_tag_mode: false,
            pregenerate_thumbnails: false,
            loading_mode: LoadingMode::Standard,
            cache_while_active: false,
            cache_when_unfocused: false,
            proxy_cache_mb: 1024,
            proxy_min_side: 3328,
            diagnostic_log: false,
            window_diagnostics: false,
            grid_start_mode: GridStartMode::Selection,
            grid_last_normal: false,
            grid_hold_ms: 250,
            gestures: false,
            gesture: crate::gesture::Gesture::LeftToRight,
            gesture_min_px: 120.,
            gesture_max_ms: 2000,
            gesture_trail: true,
            gesture_trail_color: [80, 220, 160, 220],
            gesture_trail_width: 3.,
            gesture_opens_tile: false,
            selection_flash: true,
            selection_toast: true,
            selection_toast_size: 28.,
            selection_toast_width: 560.,
            selection_toast_height: 140.,
            grid_hover_preview: false,
            hover_preview_ms: 600,
            hover_circle: false,
            hover_circle_pct: 50.,
            hover_circle_reveal_px: 60.,
            hover_circle_fade: true,
            hover_circle_opacity: 0.45,
            hover_circle_always: false,
            hover_circle_width: 1.5,
            magnify_on_move: true,
            magnify_at_once: false,
            low_latency_display: true,
            watch_folders: false,
            watched_folders: Vec::new(),
            watch_background: false,
            watch_while_active: false,
            watch_when_unfocused: true,
            watch_pace: WatchPace::Gentle,
            watch_use_journal: true,
            preview_close_px: 24.,
            preview_close_ms: 400,
            preview_size: 85.,
            preview_close_top: true,
            preview_close_left: false,
            preview_close_right: false,
            preview_close_click: false,
            normal_keeps_selection: true,
            grid_page_order: Order::Sequential,
            sort_key: crate::sorting::SortKey::Name,
            sort_descending: false,
            grid_presets: Vec::new(),
            confirm_preset_remove: true,
            grid_wheel_pages: true,
            show_favorites_button: true,
            thumbnail_cache_mb: 512,
            thumbnail_cache_dir: PathBuf::new(),
            delete_size: 100.,
            delete_color: [221, 55, 76, 255],
            delete_opacity: 0.92,
            delete_border: true,
            delete_border_width: 2.,
            delete_border_color: [255, 150, 160, 255],
            close_angle: 45.,
            close_size: 26.,
            delete_pos: None,
            destination: PathBuf::new(),
            verified_move: true,
            music: vec![],
            music_shuffle: false,
            volume: 0.7,
            cache_mb: 768,
            upscale: Upscale::Linear,
            upscale_factor: 2,
            sharpen: 0.45,
            model_path: PathBuf::new(),
            ort_library: PathBuf::new(),
            tile_size: 128,
            backup_extra: PathBuf::new(),
            backup_days: [true; 7],
            backup_hour: 20,
            backup_minute: 0,
            backup_schedule: false,
            backup_exit: false,
            backup_keep: 14,
            shortcuts: [
                ("next", "ArrowRight"),
                ("previous", "ArrowLeft"),
                ("fullscreen", "F11"),
                ("open", "O"),
                ("settings", "S"),
                ("tags", "T"),
                ("thumbnails", "G"),
                ("delete", "Delete"),
                ("slideshow", "Space"),
                ("info", "I"),
                ("bar", "Tab"),
            ]
            .into_iter()
            .map(|(a, b)| (a.into(), b.into()))
            .collect(),
        }
    }
}
impl Settings {
    /// The settings a profile keeps, by their saved names.
    pub fn profile_values(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut values = match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        };
        values.retain(|k, _| !PROFILE_EXCLUDED.contains(&k.as_str()));
        // With the Move grid open, the grid's own values (not the Move
        // grid's) belong to the profile.
        if let Some(saved) = &self.grid_own_saved {
            for (k, v) in saved {
                values.insert(k.clone(), v.clone());
            }
        }
        values
    }
    /// Sets the profile's settings; settings it does not have (newer than
    /// the profile) and values this version cannot read stay as they are.
    pub fn apply_profile_values(&mut self, values: &serde_json::Map<String, serde_json::Value>) {
        // With the Move grid open, the grid's settings go to the grid's
        // values kept aside; the Move grid keeps its own.
        let open = self.grid_own_saved.is_some();
        if let Some(saved) = &mut self.grid_own_saved {
            for (k, v) in values {
                if saved.contains_key(k) {
                    saved.insert(k.clone(), v.clone());
                }
            }
        }
        self.set_values(values, |k| {
            !PROFILE_EXCLUDED.contains(&k) && !(open && MOVE_GRID_OWN.contains(&k))
        });
        self.initialize_after_load();
    }
    /// These settings back to their defaults.
    pub fn reset_values(&mut self, keys: &[&str]) {
        let mut defaults = Settings::default();
        defaults.initialize_after_load();
        let values = defaults.values_of(keys);
        self.set_values(&values, |k| keys.contains(&k));
        self.initialize_after_load();
    }
    /// These settings, by their saved names.
    fn values_of(&self, keys: &[&str]) -> serde_json::Map<String, serde_json::Value> {
        let mut values = match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        };
        values.retain(|k, _| keys.contains(&k.as_str()));
        values
    }
    /// The Move grid opens: its own values of the grid's settings replace
    /// the grid's (kept to come back; first time: the grid's own).
    pub fn enter_move_grid_settings(&mut self) {
        if self.grid_own_saved.is_some() {
            return;
        }
        let grid = self.values_of(MOVE_GRID_OWN);
        let mut own = grid.clone();
        for (k, v) in &self.move_grid_own {
            if own.contains_key(k) {
                own.insert(k.clone(), v.clone());
            }
        }
        self.set_values(&own, |k| MOVE_GRID_OWN.contains(&k));
        self.grid_own_saved = Some(grid);
    }
    /// The Move grid closes (or NoirView starts after closing with it open):
    /// its values are kept and the grid's come back.
    pub fn leave_move_grid_settings(&mut self) {
        let Some(grid) = self.grid_own_saved.take() else {
            return;
        };
        self.move_grid_own = self.values_of(MOVE_GRID_OWN);
        self.set_values(&grid, |k| MOVE_GRID_OWN.contains(&k));
    }
    /// Sets the values whose names `allowed` takes, each only when this
    /// version has and can read it.
    fn set_values(
        &mut self,
        values: &serde_json::Map<String, serde_json::Value>,
        allowed: impl Fn(&str) -> bool,
    ) {
        let Ok(serde_json::Value::Object(mut current)) = serde_json::to_value(&*self) else {
            return;
        };
        for (key, value) in values {
            if !allowed(key) || !current.contains_key(key) {
                continue;
            }
            let old = current.insert(key.clone(), value.clone());
            if serde_json::from_value::<Settings>(serde_json::Value::Object(current.clone()))
                .is_err()
            {
                if let Some(old) = old {
                    current.insert(key.clone(), old);
                }
            }
        }
        if let Ok(settings) = serde_json::from_value::<Settings>(serde_json::Value::Object(current))
        {
            *self = settings;
        }
    }
    /// A profile's settings: a saved one, or the defaults for "Default".
    pub fn profile_target(&self, name: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
        if name == DEFAULT_PROFILE {
            // As loaded: options that follow another until changed are set.
            let mut defaults = Settings::default();
            defaults.initialize_after_load();
            return Some(defaults.profile_values());
        }
        self.profiles
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.values.clone())
    }
    /// The settings that differ from the profile in use (none in use: 0).
    pub fn profile_differences(&self) -> usize {
        let Some(values) = self.profile_target(&self.active_profile) else {
            return 0;
        };
        let now = self.profile_values();
        values
            .iter()
            .filter(|(k, v)| now.get(*k).is_some_and(|n| !same_value(n, v)))
            .count()
    }
    pub fn tag_width(&self) -> f32 {
        let width = self.tags_max_width.unwrap_or(self.bar_width);
        if width.is_finite() {
            width.clamp(340., 1600.)
        } else {
            680.
        }
    }
    pub fn initialize_tag_width(&mut self) {
        self.tags_max_width = Some(self.tag_width());
    }
    /// Values derived once after loading saved settings.
    pub fn initialize_after_load(&mut self) {
        self.initialize_tag_width();
        // Before 0.1.45 one switch covered every control: keep that behaviour
        // for the trash button and the Settings window until changed.
        self.hover_trash.get_or_insert(self.hover);
        self.hover_settings.get_or_insert(self.hover);
        self.hover_move.get_or_insert(self.hover);
        self.bar_bottom_grid.get_or_insert(self.bar_bottom);
    }
    pub fn bar_bottom_grid(&self) -> f32 {
        self.bar_bottom_grid.unwrap_or(self.bar_bottom)
    }
    pub fn hover_trash(&self) -> bool {
        self.hover_trash.unwrap_or(self.hover)
    }
    pub fn hover_move(&self) -> bool {
        self.hover_move.unwrap_or(self.hover)
    }
    pub fn hover_settings(&self) -> bool {
        self.hover_settings.unwrap_or(self.hover)
    }
    pub fn view_mode(&self) -> DisplayMode {
        self.display_mode.unwrap_or(if self.fit {
            DisplayMode::Fit
        } else {
            DisplayMode::ShrinkOnly
        })
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub schema: u32,
    pub settings: Settings,
    pub tags: BTreeSet<String>,
    pub hidden_tags: BTreeSet<String>,
    /// None denotes a legacy mixed palette, whose origin was never recorded.
    pub manual_tags: Option<BTreeSet<String>>,
    #[serde(skip)]
    pub tag_revision: u64,
    pub photos: Photos,
    pub tracked_marks: crate::identity::TrackedMarks,
    pub last_folder: Option<PathBuf>,
    pub last_image: Option<PathBuf>,
    pub last_grid: bool,
    pub last_backup_slot: Option<String>,
    /// The last move to the Recycle Bin (Restore deleted).
    pub last_deletion: Option<Deletion>,
}
/// A profile: a name and the display and interaction settings it sets
/// (settings by their saved names; those not in it stay as they are).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub values: serde_json::Map<String, serde_json::Value>,
}

/// The built-in profile with the default display and interaction settings.
pub const DEFAULT_PROFILE: &str = "Default";

/// Settings a profile leaves alone: folders and files, folder watching and
/// the service, caches, backups, the graphics backend, diagnostics, the
/// profiles themselves and remembered places.
pub const PROFILE_EXCLUDED: &[&str] = &[
    "profiles",
    "active_profile",
    "dialog_places",
    "move_grid_own",
    "grid_own_saved",
    "delete_pos",
    "move_pos",
    "favorite_places",
    "favorite_folders",
    "recursive",
    "watch_folders",
    "watched_folders",
    "watch_background",
    "watch_while_active",
    "watch_when_unfocused",
    "watch_pace",
    "watch_use_journal",
    "thumbnail_cache_dir",
    "thumbnail_cache_mb",
    "pregenerate_thumbnails",
    "loading_mode",
    "cache_while_active",
    "cache_when_unfocused",
    "proxy_cache_mb",
    "proxy_min_side",
    "cache_mb",
    "animation_mb",
    "prefetch_ahead",
    "prefetch_behind",
    "history_limit",
    "destination",
    "verified_move",
    "music",
    "model_path",
    "ort_library",
    "tile_size",
    "backup_extra",
    "backup_days",
    "backup_hour",
    "backup_minute",
    "backup_schedule",
    "backup_exit",
    "backup_keep",
    "graphics_backend",
    "low_latency_display",
    "remember_window",
    "diagnostic_log",
    "window_diagnostics",
    "grid_last_normal",
    "forget_deletion_on_close",
];

/// Equal settings values, numbers by value at f32 precision (60 and 60.0
/// alike; a saved and read back 0.92 is not exactly the f32 0.92).
fn same_value(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() <= 1e-6 * x.abs().max(y.abs()).max(1.),
            _ => x == y,
        },
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same_value(x, y))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| same_value(v, w)))
        }
        _ => a == b,
    }
}

/// Where browsing was in a folder: the image shown, the browsing order then,
/// and with Shuffle the images already shown in the round.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FolderPlace {
    pub image: PathBuf,
    /// Its place in the list, for when it is gone.
    pub index: usize,
    pub order: Order,
    pub seen: Vec<PathBuf>,
}

/// The grid's settings the Move grid has its own values of (Settings show
/// and change those while the Move grid is open).
pub const MOVE_GRID_OWN: &[&str] = &[
    "grid_hover_preview",
    "hover_preview_ms",
    "preview_size",
    "preview_close_px",
    "preview_close_ms",
    "preview_close_top",
    "preview_close_left",
    "preview_close_right",
    "preview_close_click",
    "preview_close_move_px",
    "preview_close_move_dir",
    "hover_circle",
    "hover_circle_pct",
    "hover_circle_reveal_px",
    "hover_circle_fade",
    "hover_circle_opacity",
    "hover_circle_always",
    "hover_circle_width",
    "grid_page_arrows",
    "grid_bar_bottom",
    "grid_bar_fixed_width",
    "grid_bar_colors",
    "grid_show_marks",
    "grid_marks_icons",
    "grid_keep_tag_mode",
    "grid_hold_ms",
    "selection_flash",
    "selection_toast",
    "selection_toast_size",
    "selection_toast_width",
    "selection_toast_height",
    "normal_keeps_selection",
    "grid_page_order",
    "grid_random_both",
    "grid_presets",
    "grid_wheel_pages",
    "grid_page_wrap",
    "grid_caption_range",
    "grid_caption_folder",
    "grid_caption_count",
    "magnify_on_move",
    "magnify_at_once",
];

/// A favorite folder: its name in the list (empty: the folder's name).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FavoriteFolder {
    pub name: String,
    pub path: PathBuf,
}
impl FavoriteFolder {
    pub fn label(&self) -> String {
        if self.name.trim().is_empty() {
            self.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.path.display().to_string())
        } else {
            self.name.clone()
        }
    }
}
/// A move to the Recycle Bin that Restore deleted can undo.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Deletion {
    /// When it started (Unix seconds).
    pub at: i64,
    /// The images, with what NoirView knew of them (keywords, marks).
    pub items: Vec<(PathBuf, Option<PhotoMeta>)>,
}
impl Library {
    /// Small application state; per-image records are persisted as deltas.
    pub fn header(&self) -> Self {
        Self {
            schema: self.schema,
            settings: self.settings.clone(),
            tags: self.tags.clone(),
            hidden_tags: self.hidden_tags.clone(),
            manual_tags: Some(self.manual_tags.clone().unwrap_or_default()),
            tag_revision: 0,
            photos: Photos::default(),
            tracked_marks: crate::identity::TrackedMarks::default(),
            last_folder: self.last_folder.clone(),
            last_image: self.last_image.clone(),
            last_grid: self.last_grid,
            last_backup_slot: self.last_backup_slot.clone(),
            last_deletion: self.last_deletion.clone(),
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub tags: BTreeSet<String>,
    pub mode: FilterMode,
    pub favorite: bool,
    pub elite: bool,
    /// Only the images marked to move (single view), and / or those chosen
    /// in the grid's Move selected mode.
    pub marked: bool,
    pub chosen: bool,
    /// The images `marked` / `chosen` allow, filled in when a view is built.
    pub allowed: Option<std::sync::Arc<std::collections::HashSet<PathBuf>>>,
}
impl Filter {
    pub fn active(&self) -> bool {
        !self.tags.is_empty() || self.favorite || self.elite || self.marked || self.chosen
    }
    /// `matches`, and among the images `marked` / `chosen` allow.
    pub fn matches_path(&self, path: &Path, m: &PhotoMeta) -> bool {
        self.allowed.as_ref().is_none_or(|a| a.contains(path)) && self.matches(m)
    }
    /// Gives the same images as `other` (both off counts as the same).
    pub fn matches_same(&self, other: &Filter) -> bool {
        (!self.active() && !other.active())
            || (self.tags == other.tags
                && self.mode == other.mode
                && self.favorite == other.favorite
                && self.elite == other.elite
                && self.marked == other.marked
                && self.chosen == other.chosen)
    }
    pub fn matches(&self, m: &PhotoMeta) -> bool {
        let tags = self.tags.is_empty()
            || match self.mode {
                FilterMode::All => self.tags.is_subset(&m.tags),
                FilterMode::Any => !self.tags.is_disjoint(&m.tags),
                FilterMode::Exclude => self.tags.is_disjoint(&m.tags),
            };
        tags && (!self.favorite || m.favorite) && (!self.elite || m.elite)
    }
}
/// Built when the visible result changes. Position lookup never scans paths.
#[derive(Default)]
pub struct VisibleFiles {
    paths: Vec<PathBuf>,
    positions: HashMap<PathBuf, usize>,
}
impl From<Vec<PathBuf>> for VisibleFiles {
    fn from(paths: Vec<PathBuf>) -> Self {
        let positions = paths
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, p)| (p, i))
            .collect();
        Self { paths, positions }
    }
}
impl Deref for VisibleFiles {
    type Target = [PathBuf];
    fn deref(&self) -> &Self::Target {
        &self.paths
    }
}
impl VisibleFiles {
    pub fn position(&self, path: &Path) -> Option<usize> {
        self.positions.get(path).copied()
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GridStartMode {
    #[default]
    Selection,
    Normal,
    LastUsed,
}
pub const DEFAULT_HISTORY_LIMIT: usize = 10_000;
#[derive(Default)]
pub struct Navigator {
    history: VecDeque<PathBuf>,
    history_limit: Option<usize>,
    cursor: usize,
    seen: HashMap<PathBuf, u64>,
    cycle: u64,
    remaining: Vec<usize>,
    remaining_len: usize,
    slots: Vec<usize>,
    future: VecDeque<usize>,
    past: VecDeque<usize>,
    future_order: Option<Order>,
    backward: bool,
}
/// Sparse copy-on-write view of the shuffle pool. Looking ahead touches only the
/// bounded horizon, never clones the full collection or marks a photo as seen.
struct ShuffleLookahead<'a> {
    nav: &'a Navigator,
    len: usize,
    values: HashMap<usize, usize>,
    slots: HashMap<usize, usize>,
}
impl ShuffleLookahead<'_> {
    fn value(&self, slot: usize) -> usize {
        self.values
            .get(&slot)
            .copied()
            .unwrap_or(self.nav.remaining[slot])
    }
    fn remove(&mut self, index: usize) {
        let slot = self
            .slots
            .get(&index)
            .copied()
            .unwrap_or(self.nav.slots[index]);
        self.len -= 1;
        let other = self.value(self.len);
        self.values.insert(slot, other);
        self.values.insert(self.len, index);
        self.slots.insert(other, slot);
        self.slots.insert(index, self.len);
    }
}
impl Navigator {
    #[cfg(test)]
    pub fn reset(&mut self, files: &VisibleFiles, path: Option<&Path>) {
        self.history.clear();
        self.seen.clear();
        self.cursor = 0;
        self.future.clear();
        self.past.clear();
        self.future_order = None;
        self.backward = false;
        if let Some(p) = path {
            self.history.push_back(p.into());
            self.seen.insert(p.into(), self.cycle);
        }
        self.rebuild_pool(files);
    }
    fn rebuild_pool(&mut self, files: &VisibleFiles) {
        self.remaining = (0..files.len()).collect();
        self.slots = (0..files.len()).collect();
        self.remaining_len = files.len();
        for (i, p) in files.iter().enumerate() {
            if self.seen.get(p) == Some(&self.cycle) {
                self.remove_candidate(i);
            }
        }
    }
    fn remove_candidate(&mut self, index: usize) {
        let slot = self.slots[index];
        if slot < self.remaining_len {
            self.remaining_len -= 1;
            let other = self.remaining[self.remaining_len];
            self.remaining.swap(slot, self.remaining_len);
            self.slots[other] = slot;
            self.slots[index] = self.remaining_len;
        }
    }
    fn mark_seen(&mut self, files: &VisibleFiles, index: usize) {
        if let Some(cycle) = self.seen.get_mut(&files[index]) {
            *cycle = self.cycle;
        } else {
            self.seen.insert(files[index].clone(), self.cycle);
        }
        self.remove_candidate(index);
    }
    #[cfg(test)]
    pub fn retain(&mut self, files: &VisibleFiles) {
        self.future.clear();
        self.past.clear();
        let old = self.cursor;
        let before = self
            .history
            .iter()
            .enumerate()
            .filter(|(i, p)| *i <= old && files.position(p).is_some())
            .count();
        self.history.retain(|p| files.position(p).is_some());
        self.cursor = before.saturating_sub(1);
        self.seen.retain(|p, _| files.position(p).is_some());
        self.rebuild_pool(files);
    }
    /// An image shown without browsing (Go to, the grid's big image,
    /// recently viewed): browsing starts again from it in both directions
    /// (In order: its neighbours, not the images browsed before the jump).
    pub fn jump(&mut self, files: &VisibleFiles, path: &Path) {
        if self.history.get(self.cursor).map(PathBuf::as_path) == Some(path) {
            return;
        }
        self.history.clear();
        self.history.push_back(path.to_path_buf());
        self.cursor = 0;
        self.future.clear();
        self.past.clear();
        // (The shuffle pool covers the list once the view is built.)
        if let Some(i) = files.position(path).filter(|i| *i < self.slots.len()) {
            self.mark_seen(files, i);
        }
        self.trim_history();
    }
    /// The images shown in the current Shuffle round.
    pub fn round_seen(&self) -> Vec<PathBuf> {
        self.seen
            .iter()
            .filter(|(_, cycle)| **cycle == self.cycle)
            .map(|(p, _)| p.clone())
            .collect()
    }
    /// Browsing goes on from a saved place: `seen` were already shown in
    /// this Shuffle round (images gone since are left out; images added
    /// since are still to come), and `current` is shown now.
    pub fn resume(&mut self, files: &VisibleFiles, seen: &[PathBuf], current: &Path) {
        self.history.clear();
        self.cursor = 0;
        self.future.clear();
        self.past.clear();
        self.backward = false;
        self.seen.clear();
        for p in seen.iter().filter(|p| files.position(p).is_some()) {
            self.seen.insert(p.clone(), self.cycle);
        }
        self.rebuild_pool(files);
        self.jump(files, current);
    }
    /// A new start: nothing shown yet in this round, no history.
    pub fn start_over(&mut self, files: &VisibleFiles) {
        self.history.clear();
        self.cursor = 0;
        self.future.clear();
        self.past.clear();
        self.backward = false;
        self.seen.clear();
        self.rebuild_pool(files);
    }
    pub fn history_len(&self) -> usize {
        self.history.len()
    }
    pub fn history_limit(&self) -> usize {
        self.history_limit.unwrap_or(DEFAULT_HISTORY_LIMIT)
    }
    pub fn set_history_limit(&mut self, limit: usize) -> bool {
        let limit = limit.clamp(2, 100_000);
        let changed = self.history_limit() != limit;
        self.history_limit = Some(limit);
        if changed {
            self.trim_history();
            if self.history.capacity() > limit.saturating_mul(2) {
                self.history.shrink_to(limit);
            }
        }
        changed
    }
    fn trim_history(&mut self) {
        let limit = self.history_limit();
        // Keep the current image and its nearest recorded visits. At a growing
        // edge only the opposite edge is discarded, preserving the active plan.
        while self.history.len() > limit {
            if self.cursor > self.history.len() - 1 - self.cursor {
                self.history.pop_front();
                self.cursor -= 1;
                self.past.clear();
            } else {
                self.history.pop_back();
                self.future.clear();
            }
        }
    }
    fn set_order(&mut self, order: Order, current: Option<&Path>) {
        if self.future_order != Some(order) {
            // A different order starts one new trail at the displayed image in
            // BOTH directions. Shuffle seen/cycle state remains independent.
            if self.future_order.is_some() {
                let anchor = current
                    .map(Path::to_path_buf)
                    .or_else(|| self.history.get(self.cursor).cloned());
                self.history.clear();
                if let Some(anchor) = anchor {
                    self.history.push_back(anchor);
                }
                self.cursor = 0;
            }
            self.future.clear();
            self.past.clear();
            self.future_order = Some(order);
        }
    }
    pub fn previous(
        &mut self,
        files: &VisibleFiles,
        current: Option<&Path>,
        order: Order,
    ) -> Option<PathBuf> {
        self.set_order(order, current);
        self.backward = true;
        if self.cursor > 0 {
            self.cursor -= 1;
            return self.history.get(self.cursor).cloned();
        }
        if files.is_empty() {
            return None;
        }
        self.prepare_plan(files, order, true, 1, current);
        let index = self.past.pop_front()?;
        self.commit(files, order, index);
        let p = files[index].clone();
        self.history.push_front(p.clone());
        self.trim_history();
        // The opposite plan was hypothetical before this new committed visit.
        self.future.clear();
        Some(p)
    }
    pub fn next(
        &mut self,
        files: &VisibleFiles,
        current: Option<&Path>,
        order: Order,
    ) -> Option<PathBuf> {
        self.set_order(order, current);
        self.backward = false;
        if files.is_empty() {
            return None;
        }
        if self.cursor + 1 < self.history.len() {
            self.cursor += 1;
            return self.history.get(self.cursor).cloned();
        }
        self.prepare_plan(files, order, false, 1, current);
        let index = self.future.pop_front()?;
        self.commit(files, order, index);
        let p = files[index].clone();
        self.history.push_back(p.clone());
        self.cursor = self.history.len() - 1;
        self.trim_history();
        self.past.clear();
        Some(p)
    }
    fn commit(&mut self, files: &VisibleFiles, order: Order, index: usize) {
        // Advance epochs only for an actual new visit, never when planning or
        // replaying history. Both boundaries share one no-repeat shuffle pool.
        if order == Order::Shuffle && self.remaining_len == 0 {
            if self.cycle == u64::MAX {
                self.seen.clear();
                self.cycle = 0;
            } else {
                self.cycle += 1;
            }
            self.remaining_len = files.len();
        }
        self.mark_seen(files, index);
    }
    fn prepare_plan(
        &mut self,
        files: &VisibleFiles,
        order: Order,
        backward: bool,
        count: usize,
        current: Option<&Path>,
    ) {
        self.set_order(order, current);
        let existing = if backward { &self.past } else { &self.future };
        if files.is_empty() || existing.len() >= count {
            return;
        }
        let mut planned = existing.clone();
        let boundary = if backward {
            self.history.front()
        } else {
            self.history.back()
        };
        let mut previous = boundary
            .map(PathBuf::as_path)
            .or(current)
            .and_then(|p| files.position(p))
            .unwrap_or(0);
        let mut pool = ShuffleLookahead {
            nav: self,
            len: self.remaining_len,
            values: HashMap::new(),
            slots: HashMap::new(),
        };
        let mut rng = rand::thread_rng();
        for step in 0..count {
            let rollover = order == Order::Shuffle && pool.len == 0;
            if rollover {
                pool.len = files.len();
            }
            let index = if let Some(index) = planned.get(step) {
                *index
            } else {
                let index = match order {
                    Order::Sequential => {
                        if backward {
                            (previous + files.len() - 1) % files.len()
                        } else {
                            (previous + 1) % files.len()
                        }
                    }
                    Order::Random => rng.gen_range(0..files.len()),
                    Order::Shuffle => {
                        let slot = if rollover && files.len() > 1 {
                            let excluded = pool
                                .slots
                                .get(&previous)
                                .copied()
                                .unwrap_or(self.slots[previous]);
                            let slot = rng.gen_range(0..pool.len - 1);
                            if slot >= excluded {
                                slot + 1
                            } else {
                                slot
                            }
                        } else {
                            rng.gen_range(0..pool.len)
                        };
                        pool.value(slot)
                    }
                };
                planned.push_back(index);
                index
            };
            if order == Order::Shuffle {
                pool.remove(index);
            }
            previous = index;
        }
        if backward {
            self.past = planned;
        } else {
            self.future = planned;
        }
    }
    /// Likely targets in priority order. History is read without moving its cursor;
    /// Boundary plans are committed only by actual Next/Previous navigation.
    pub fn prefetch(
        &mut self,
        files: &VisibleFiles,
        current: Option<&Path>,
        order: Order,
        ahead: usize,
        behind: usize,
    ) -> Vec<PathBuf> {
        if files.is_empty() {
            return Vec::new();
        }
        let ahead = ahead.min(32);
        let behind = behind.min(32);
        let (forward, backward) = if self.backward {
            (behind, ahead)
        } else {
            (ahead, behind)
        };
        self.set_order(order, current);
        let history_forward = self.history.len().saturating_sub(self.cursor + 1);
        self.prepare_plan(
            files,
            order,
            false,
            forward.saturating_sub(history_forward),
            current,
        );
        self.prepare_plan(
            files,
            order,
            true,
            backward.saturating_sub(self.cursor),
            current,
        );
        let mut next = Vec::new();
        next.extend(
            self.history
                .iter()
                .skip(self.cursor + 1)
                .take(forward)
                .cloned(),
        );
        next.extend(
            self.future
                .iter()
                .take(forward.saturating_sub(next.len()))
                .map(|&i| files[i].clone()),
        );
        let mut previous: Vec<_> = self
            .history
            .iter()
            .take(self.cursor)
            .rev()
            .take(backward)
            .cloned()
            .collect();
        previous.extend(
            self.past
                .iter()
                .take(backward.saturating_sub(previous.len()))
                .map(|&i| files[i].clone()),
        );
        let (first, second) = if self.backward {
            (previous, next)
        } else {
            (next, previous)
        };
        let mut seen = std::collections::HashSet::new();
        first
            .into_iter()
            .chain(second)
            .filter(|p| Some(p.as_path()) != current && seen.insert(p.clone()))
            .collect()
    }
}
pub fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| {
            matches!(
                s.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tif" | "tiff" | "gif" | "ico"
            )
        })
        .unwrap_or(false)
}
pub fn scan(folder: &Path, recursive: bool) -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = vec![];
    for item in walkdir::WalkDir::new(folder)
        .follow_links(false)
        .max_depth(if recursive { usize::MAX } else { 1 })
    {
        let e = item?;
        if e.file_type().is_file() && supported(e.path()) {
            paths.push(e.path().to_path_buf());
        }
    }
    paths.sort_by_cached_key(|p| p.to_string_lossy().to_lowercase());
    Ok(paths)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    #[test]
    fn a_profile_keeps_display_and_interaction_settings_only() {
        let mut s = Settings {
            fit: false,
            lens_zoom: 4.,
            round_notice_fade: true,
            thumbnail_cache_dir: "/cache".into(),
            destination: "/dest".into(),
            ..Settings::default()
        };
        s.watched_folders.push(WatchedFolder {
            path: "/watched".into(),
            subfolders: true,
        });
        let values = s.profile_values();
        assert_eq!(values.get("fit"), Some(&serde_json::json!(false)));
        assert!(values.contains_key("lens_zoom") && values.contains_key("shortcuts"));
        for key in PROFILE_EXCLUDED {
            assert!(!values.contains_key(*key), "{key}");
        }
        // Every excluded name is a real setting (a renamed one would slip in).
        let all = serde_json::to_value(Settings::default()).unwrap();
        for key in PROFILE_EXCLUDED {
            assert!(all.get(*key).is_some(), "{key} is not a setting");
        }
        // Applied elsewhere: only its settings change; the folders stay.
        let mut other = Settings {
            thumbnail_cache_dir: "/other-cache".into(),
            destination: "/other".into(),
            ..Settings::default()
        };
        other.apply_profile_values(&values);
        assert!(!other.fit && other.lens_zoom == 4. && other.round_notice_fade);
        assert_eq!(other.thumbnail_cache_dir, PathBuf::from("/other-cache"));
        assert_eq!(other.destination, PathBuf::from("/other"));
        assert!(other.watched_folders.is_empty());
    }
    #[test]
    fn the_move_grid_has_its_own_grid_settings() {
        // Every name is a real setting.
        let all = serde_json::to_value(Settings::default()).unwrap();
        for key in MOVE_GRID_OWN {
            assert!(all.get(*key).is_some(), "{key} is not a setting");
        }
        let mut s = Settings {
            hover_preview_ms: 400,
            grid_wheel_pages: true,
            ..Settings::default()
        };
        // First time: the grid's values; changes stay the Move grid's.
        s.enter_move_grid_settings();
        assert_eq!(s.hover_preview_ms, 400);
        s.hover_preview_ms = 900;
        s.grid_wheel_pages = false;
        // A profile saved meanwhile holds the grid's values.
        assert_eq!(
            s.profile_values()["hover_preview_ms"],
            serde_json::json!(400)
        );
        s.leave_move_grid_settings();
        assert_eq!(s.hover_preview_ms, 400);
        assert!(s.grid_wheel_pages);
        // Next time the Move grid's come back.
        s.enter_move_grid_settings();
        assert_eq!(s.hover_preview_ms, 900);
        assert!(!s.grid_wheel_pages);
        // Saved while open (NoirView closed): read back, the grid's again.
        let mut read: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        read.leave_move_grid_settings();
        assert_eq!(read.hover_preview_ms, 400);
        assert_eq!(
            read.move_grid_own["hover_preview_ms"],
            serde_json::json!(900)
        );
        // A profile applied while open changes the grid's values only.
        let mut values = serde_json::Map::new();
        values.insert("hover_preview_ms".into(), serde_json::json!(50));
        s.apply_profile_values(&values);
        assert_eq!(s.hover_preview_ms, 900);
        s.leave_move_grid_settings();
        assert_eq!(s.hover_preview_ms, 50);
    }
    #[test]
    fn a_profile_from_another_version_sets_what_it_can() {
        let mut values = serde_json::Map::new();
        values.insert("fit".into(), serde_json::json!(false));
        // A setting this version does not have, one it cannot read, and an
        // excluded one written by hand.
        values.insert("no_such_setting".into(), serde_json::json!(1));
        values.insert("lens_zoom".into(), serde_json::json!("big"));
        values.insert("destination".into(), serde_json::json!("/x"));
        let mut s = Settings::default();
        s.apply_profile_values(&values);
        assert!(!s.fit);
        assert_eq!(s.lens_zoom, Settings::default().lens_zoom);
        assert_eq!(s.destination, Settings::default().destination);
    }
    #[test]
    fn differences_from_the_profile_in_use_are_counted() {
        let mut s = Settings::default();
        assert_eq!(s.profile_differences(), 0, "none in use");
        s.profiles.push(Profile {
            name: "Evening".into(),
            values: s.profile_values(),
        });
        s.active_profile = "Evening".into();
        assert_eq!(s.profile_differences(), 0);
        // A number written another way is the same.
        s.profiles[0].values.insert(
            "thumbnail_count".into(),
            serde_json::json!(s.thumbnail_count as f64),
        );
        assert_eq!(s.profile_differences(), 0);
        // Saved and read back: floats come back within f32 precision.
        let text = serde_json::to_string(&s).unwrap();
        let s2: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(s2.profile_differences(), 0);
        s.fit = !s.fit;
        s.lens_zoom += 1.;
        // Settings outside profiles do not count.
        s.destination = "/elsewhere".into();
        assert_eq!(s.profile_differences(), 2);
    }
    fn numbered(count: usize) -> VisibleFiles {
        (0..count)
            .map(|i| PathBuf::from(format!("{i}.jpg")))
            .collect::<Vec<_>>()
            .into()
    }
    #[test]
    fn display_modes_preserve_legacy_fit_preferences_and_persist_explicit_fill() {
        let mut fit: Settings = serde_json::from_str(r#"{"fit":true}"#).unwrap();
        assert_eq!(fit.view_mode(), DisplayMode::Fit);
        let small: Settings = serde_json::from_str(r#"{"fit":false}"#).unwrap();
        assert_eq!(small.view_mode(), DisplayMode::ShrinkOnly);
        fit.display_mode = Some(DisplayMode::Fill);
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&fit).unwrap()).unwrap();
        assert_eq!(restored.view_mode(), DisplayMode::Fill);
        assert_eq!(restored.thumbnail_cache_mb, 512);
        assert_eq!(restored.thumbnail_cache_dir, PathBuf::new());
        assert_eq!(restored.loading_mode, LoadingMode::Standard);
        assert!(!restored.cache_while_active && !restored.cache_when_unfocused);
        assert_eq!(restored.proxy_cache_mb, 1024);
        assert_eq!(restored.proxy_min_side, 3328);
    }
    #[test]
    fn directional_prefetch_reverses_wraps_and_deduplicates_without_navigating() {
        let files = numbered(40);
        let mut nav = Navigator::default();
        nav.reset(&files, Some(&files[0]));
        let expected = [1, 2, 3, 4, 5, 6, 7, 39, 38, 37].map(|i| files[i].clone());
        assert_eq!(
            nav.prefetch(&files, Some(&files[0]), Order::Sequential, 7, 3),
            expected
        );
        assert_eq!(nav.history.len(), 1);
        assert_eq!(nav.seen.len(), 1);
        let current = nav
            .previous(&files, Some(&files[0]), Order::Sequential)
            .unwrap();
        let expected = [38, 37, 36, 35, 34, 33, 32, 0, 1, 2].map(|i| files[i].clone());
        assert_eq!(
            nav.prefetch(&files, Some(&current), Order::Sequential, 7, 3),
            expected
        );
        assert_eq!(
            nav.next(&files, Some(&current), Order::Sequential),
            Some(files[0].clone())
        );
        let small = numbered(2);
        nav.reset(&small, Some(&small[0]));
        assert_eq!(
            nav.prefetch(&small, Some(&small[0]), Order::Sequential, 32, 32),
            vec![small[1].clone()]
        );
    }
    #[test]
    fn prefetch_settings_default_migrate_and_allow_zero_and_live_changes() {
        let old: Settings = serde_json::from_str(r#"{"cache_mb":256,"order":"Random"}"#).unwrap();
        assert_eq!((old.prefetch_ahead, old.prefetch_behind), (7, 3));
        assert!(old.wheel_up_next);
        assert_eq!(old.history_limit, DEFAULT_HISTORY_LIMIT);
        assert_eq!(old.history_limit, DEFAULT_HISTORY_LIMIT);
        assert!(!old.lens_border && old.jpeg_preview);
        assert!(!old.lens_soft_edges && old.lens_full_window && old.magnifier);
        assert_eq!(old.cache_mb, 256);
        let settings = Settings {
            prefetch_ahead: 0,
            prefetch_behind: 12,
            ..old
        };
        let loaded: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!((loaded.prefetch_ahead, loaded.prefetch_behind), (0, 12));
        let files = numbered(100);
        let mut nav = Navigator::default();
        nav.reset(&files, Some(&files[0]));
        assert_eq!(
            nav.prefetch(&files, Some(&files[0]), Order::Sequential, 7, 3)
                .len(),
            10
        );
        assert!(nav
            .prefetch(&files, Some(&files[0]), Order::Sequential, 0, 0)
            .is_empty());
        assert_eq!(
            nav.prefetch(&files, Some(&files[0]), Order::Sequential, 0, 1),
            vec![files[99].clone()]
        );
        assert_eq!(
            nav.prefetch(
                &files,
                Some(&files[0]),
                Order::Sequential,
                usize::MAX,
                usize::MAX
            )
            .len(),
            64
        );
        assert_eq!(nav.future.len(), 32);
    }
    #[test]
    fn independent_tag_width_captures_legacy_toolbar_width_and_normalizes_limits() {
        let mut old: Settings = serde_json::from_str(r#"{"bar_width":960}"#).unwrap();
        assert!(old.tags_max_width.is_none());
        old.initialize_tag_width();
        assert_eq!(old.tag_width(), 960.);
        old.bar_width = 400.;
        assert_eq!(old.tag_width(), 960.);
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&old).unwrap()).unwrap();
        assert_eq!(restored.tag_width(), 960.);
        assert_eq!(Settings::default().tag_width(), 680.);
        for (given, expected) in [
            (0., 340.),
            (9999., 1600.),
            (f32::NAN, 680.),
            (f32::INFINITY, 680.),
        ] {
            old.tags_max_width = Some(given);
            old.initialize_tag_width();
            assert_eq!(old.tags_max_width, Some(expected));
        }
    }
    #[test]
    fn random_lookahead_is_stable_and_committed_only_on_real_navigation() {
        let files = numbered(100);
        let mut nav = Navigator::default();
        nav.reset(&files, Some(&files[0]));
        let before = (
            nav.history.clone(),
            nav.seen.clone(),
            nav.remaining.clone(),
            nav.slots.clone(),
            nav.remaining_len,
            nav.cycle,
        );
        let first = nav.prefetch(&files, Some(&files[0]), Order::Random, 7, 3);
        assert_eq!(
            nav.prefetch(&files, Some(&files[0]), Order::Random, 7, 3),
            first
        );
        assert_eq!(
            (
                nav.history.clone(),
                nav.seen.clone(),
                nav.remaining.clone(),
                nav.slots.clone(),
                nav.remaining_len,
                nav.cycle
            ),
            before
        );
        let planned = files[nav.future[0]].clone();
        let current = nav.next(&files, Some(&files[0]), Order::Random).unwrap();
        assert_eq!(current, planned);
        assert_eq!(nav.history.len(), 2);
        assert_eq!(
            nav.previous(&files, Some(&current), Order::Random),
            Some(files[0].clone())
        );
        let preview = nav.prefetch(&files, Some(&files[0]), Order::Random, 2, 1);
        if current != files[0] {
            assert!(preview.contains(&current));
        }
        assert_eq!(
            nav.next(&files, Some(&files[0]), Order::Random),
            Some(current)
        );
    }
    #[test]
    fn shuffle_lookahead_crosses_cycles_without_repeats_or_pool_clones() {
        for count in [1, 2, 5, 31] {
            let files = numbered(count);
            let mut nav = Navigator::default();
            nav.reset(&files, Some(&files[0]));
            let allocation = nav.remaining.as_ptr();
            let mut current = files[0].clone();
            let mut visited = HashSet::from([current.clone()]);
            for _ in 0..500 {
                let cycle = nav.cycle;
                let seen = nav.seen.clone();
                let len = nav.remaining_len;
                nav.prefetch(&files, Some(&current), Order::Shuffle, 32, 32);
                assert_eq!(
                    (nav.cycle, &nav.seen, nav.remaining_len),
                    (cycle, &seen, len)
                );
                let expected = files[nav.future[0]].clone();
                let previous = current;
                current = nav.next(&files, Some(&previous), Order::Shuffle).unwrap();
                assert_eq!(current, expected);
                if nav.cycle != cycle {
                    assert_eq!(visited.len(), files.len());
                    visited.clear();
                    if count > 1 {
                        assert_ne!(current, previous);
                    }
                }
                assert!(visited.insert(current.clone()));
                assert_eq!(nav.remaining.as_ptr(), allocation);
            }
        }
    }
    #[test]
    fn lookahead_invalidates_on_order_scope_and_backward_extension_changes() {
        let files = numbered(40);
        let mut nav = Navigator::default();
        nav.reset(&files, Some(&files[0]));
        nav.prefetch(&files, Some(&files[0]), Order::Random, 32, 0);
        assert_eq!(
            nav.prefetch(&files, Some(&files[0]), Order::Sequential, 2, 0),
            vec![files[1].clone(), files[2].clone()]
        );
        let current = nav
            .previous(&files, Some(&files[0]), Order::Sequential)
            .unwrap();
        assert!(nav.future.is_empty());
        nav.prefetch(&files, Some(&current), Order::Shuffle, 7, 3);
        let filtered = numbered(2);
        nav.retain(&filtered);
        assert!(nav.future.is_empty());
        let preview = nav.prefetch(&filtered, Some(&files[0]), Order::Shuffle, 7, 3);
        assert!(preview.iter().all(|p| filtered.position(p).is_some()));
    }
    #[test]
    fn shuffle_never_repeats_until_cycle_and_previous_is_history() {
        let files: VisibleFiles = (0..8)
            .map(|i| PathBuf::from(format!("{i}.png")))
            .collect::<Vec<_>>()
            .into();
        let mut n = Navigator::default();
        n.reset(&files, Some(&files[0]));
        let mut current = files[0].clone();
        let mut visited = HashSet::from([current.clone()]);
        for _ in 1..files.len() {
            current = n.next(&files, Some(&current), Order::Shuffle).unwrap();
            assert!(visited.insert(current.clone()));
        }
        let old = current.clone();
        let back = n.previous(&files, Some(&current), Order::Shuffle).unwrap();
        assert_eq!(n.next(&files, Some(&back), Order::Shuffle), Some(old));
        assert!(n.next(&files, Some(&current), Order::Shuffle).is_some());
    }
    #[test]
    fn shuffle_cycle_reset_reuses_pool_and_avoids_immediate_repeat() {
        let files: VisibleFiles = (0..31)
            .map(|i| PathBuf::from(format!("{i}.png")))
            .collect::<Vec<_>>()
            .into();
        let mut nav = Navigator::default();
        nav.reset(&files, Some(&files[0]));
        let mut current = files[0].clone();
        let pool = nav.remaining.as_ptr();
        for _ in 0..20 {
            let mut visited = HashSet::from([current.clone()]);
            for _ in 1..files.len() {
                current = nav.next(&files, Some(&current), Order::Shuffle).unwrap();
                assert!(visited.insert(current.clone()));
            }
            let old = current.clone();
            current = nav.next(&files, Some(&current), Order::Shuffle).unwrap();
            assert_ne!(old, current);
            assert_eq!(pool, nav.remaining.as_ptr());
            // The next iteration starts with this newly selected first image.
        }
    }
    #[test]
    fn filters_combine_tags_and_elite() {
        let m = PhotoMeta {
            identity: None,
            rebound_from: None,
            tags: BTreeSet::from(["night".into(), "city".into()]),
            favorite: true,
            elite: false,
        };
        let mut f = Filter {
            tags: BTreeSet::from(["night".into()]),
            ..Default::default()
        };
        assert!(f.matches(&m));
        f.mode = FilterMode::Exclude;
        assert!(!f.matches(&m));
        f.tags.clear();
        f.elite = true;
        assert!(!f.matches(&m));
    }
    #[test]
    fn index_refresh_preserves_back_and_forward_history_for_visible_images() {
        let files: VisibleFiles = ["a.png", "b.png", "c.png"]
            .map(PathBuf::from)
            .to_vec()
            .into();
        let mut n = Navigator::default();
        n.reset(&files, Some(&files[0]));
        assert_eq!(
            n.next(&files, Some(&files[0]), Order::Sequential),
            Some(files[1].clone())
        );
        assert_eq!(
            n.next(&files, Some(&files[1]), Order::Sequential),
            Some(files[2].clone())
        );
        assert_eq!(
            n.previous(&files, Some(&files[2]), Order::Sequential),
            Some(files[1].clone())
        );
        n.retain(&files);
        assert_eq!(
            n.next(&files, Some(&files[1]), Order::Sequential),
            Some(files[2].clone())
        );
        let files: VisibleFiles = files[..2].to_vec().into();
        n.retain(&files);
        assert_eq!(
            n.previous(&files, Some(&files[1]), Order::Sequential),
            Some(files[0].clone())
        );
    }
    #[test]
    fn navigation_keeps_history_and_shuffle_consistent_after_filtering_and_reordering() {
        let files: VisibleFiles = (0..100)
            .map(|i| PathBuf::from(format!("{i}.png")))
            .collect::<Vec<_>>()
            .into();
        let mut nav = Navigator::default();
        nav.reset(&files, Some(&files[0]));
        let mut current = files[0].clone();
        for _ in 0..30 {
            current = nav.next(&files, Some(&current), Order::Random).unwrap();
        }
        let previous = nav.previous(&files, Some(&current), Order::Random).unwrap();
        assert_eq!(
            nav.next(&files, Some(&previous), Order::Random),
            Some(current.clone())
        );
        let mut paths = files
            .iter()
            .filter(|p| *p != &current)
            .cloned()
            .collect::<Vec<_>>();
        paths.reverse();
        paths.push("new.png".into());
        let filtered: VisibleFiles = paths.into();
        nav.retain(&filtered);
        current = filtered[0].clone();
        for _ in 0..300 {
            let next = nav.next(&filtered, Some(&current), Order::Shuffle).unwrap();
            assert!(filtered.position(&next).is_some());
            current = next;
        }
    }
    #[test]
    fn browsing_goes_on_from_an_image_shown_directly() {
        let files = numbered(10);
        let mut nav = Navigator::default();
        nav.reset(&files, Some(&files[0]));
        assert_eq!(
            nav.next(&files, Some(&files[0]), Order::Sequential),
            Some(files[1].clone())
        );
        // Image 7 shown with Go to: next is 8; back goes 8 → 7 → 6, not to 1.
        nav.jump(&files, &files[7]);
        assert_eq!(
            nav.next(&files, Some(&files[7]), Order::Sequential),
            Some(files[8].clone())
        );
        assert_eq!(
            nav.previous(&files, Some(&files[8]), Order::Sequential),
            Some(files[7].clone())
        );
        assert_eq!(
            nav.previous(&files, Some(&files[7]), Order::Sequential),
            Some(files[6].clone())
        );
        // Straight back after a jump: its neighbour too.
        nav.jump(&files, &files[4]);
        assert_eq!(
            nav.previous(&files, Some(&files[4]), Order::Sequential),
            Some(files[3].clone())
        );
        // Jumping to the image already at the cursor changes nothing.
        let before = nav.history_len();
        nav.jump(&files, &files[3]);
        assert_eq!(nav.history_len(), before);
    }
    #[test]
    fn one_image_shuffle_and_previous_remain_valid() {
        let files: VisibleFiles = vec![PathBuf::from("only.png")].into();
        let mut nav = Navigator::default();
        nav.reset(&files, Some(&files[0]));
        for _ in 0..5 {
            assert_eq!(
                nav.next(&files, Some(&files[0]), Order::Shuffle),
                Some(files[0].clone())
            );
        }
        assert_eq!(
            nav.previous(&files, Some(&files[0]), Order::Sequential),
            Some(files[0].clone())
        );
    }
    #[test]
    fn photo_change_tracking_preserves_json_and_coalesces_edits_and_deletions() {
        let mut photos = Photos::default();
        photos.entry("a.png".into()).or_default().favorite = true;
        photos.entry("a.png".into()).or_default().elite = true;
        photos.insert("b.png".into(), PhotoMeta::default());
        photos.remove(Path::new("b.png"));
        let changes = photos.take_changes();
        assert_eq!(changes.len(), 2);
        assert!(changes[Path::new("a.png")].as_ref().unwrap().elite);
        assert!(changes[Path::new("b.png")].is_none());
        assert!(photos.take_changes().is_empty());
        let loaded: Photos =
            serde_json::from_str(&serde_json::to_string(&photos).unwrap()).unwrap();
        assert_eq!(*loaded, *photos);
    }
    #[test]
    #[ignore = "Synthetic path-list benchmark; no image decoding or display timing"]
    fn navigation_large_collection_latency() {
        use std::time::Instant;
        for count in [5000, 100_000, 1_000_000] {
            let started = Instant::now();
            let files: VisibleFiles = (0..count)
                .map(|i| {
                    PathBuf::from(format!(
                        "C:/Photos/archive/collection-{i:08}/image-{i:08}.jpg"
                    ))
                })
                .collect::<Vec<_>>()
                .into();
            let mut nav = Navigator::default();
            nav.reset(&files, Some(&files[0]));
            println!(
                "paths={count}, build_and_shuffle_pool_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.
            );
            let mut current = files[0].clone();
            for order in [Order::Sequential, Order::Random, Order::Shuffle] {
                let mut samples = Vec::with_capacity(10_000);
                for _ in 0..10_000 {
                    let started = Instant::now();
                    current = nav.next(&files, Some(&current), order).unwrap();
                    std::hint::black_box(files.position(&current));
                    samples.push(started.elapsed().as_nanos());
                }
                samples.sort_unstable();
                println!("paths={count}, mode={order:?}, action_us p50={:.3}, p95={:.3}, p99={:.3}, max={:.3}",
                    samples[5000] as f64/1000., samples[9500] as f64/1000., samples[9900] as f64/1000., samples[9999] as f64/1000.);
            }
        }
    }
    #[test]
    fn switching_order_at_the_start_of_a_5000_image_trail_branches_immediately() {
        for order in [Order::Random, Order::Shuffle] {
            let files = numbered(5000);
            let mut nav = Navigator::default();
            nav.reset(&files, Some(&files[0]));
            let mut current = files[0].clone();
            for i in 1..=4 {
                current = nav.next(&files, Some(&current), Order::Sequential).unwrap();
                assert_eq!(current, files[i]);
            }
            for _ in 0..4 {
                current = nav
                    .previous(&files, Some(&current), Order::Sequential)
                    .unwrap();
            }
            assert_eq!(current, files[0]);
            assert_eq!(nav.history.len(), 5);
            // Even disabled look-ahead applies policy changes before replay.
            nav.prefetch(&files, Some(&current), order, 0, 0);
            assert_eq!(nav.history.len(), 1);
            assert_eq!(nav.cursor, 0);
            nav.prefetch(&files, Some(&current), order, 7, 3);
            let expected = files[nav.future[0]].clone();
            let planned = nav.future.len();
            let next = nav.next(&files, Some(&current), order).unwrap();
            assert_eq!(next, expected);
            assert_eq!(nav.future.len(), planned - 1);
            assert_eq!(nav.history.len(), 2);
            assert!(nav.past.is_empty());
            assert_eq!(
                nav.previous(&files, Some(&next), order),
                Some(files[0].clone())
            );
            // A normal direction reversal without a mode change still replays history.
            assert_eq!(nav.next(&files, Some(&files[0]), order), Some(next));
        }
    }
    #[test]
    fn random_and_shuffle_extend_past_the_initial_image_using_the_actual_prefetch_plan() {
        for order in [Order::Random, Order::Shuffle] {
            let files = numbered(5000);
            let mut nav = Navigator::default();
            nav.reset(&files, Some(&files[0]));
            let mut current = files[0].clone();
            let mut trail = vec![current.clone()];
            for _ in 0..5 {
                current = nav.next(&files, Some(&current), order).unwrap();
                trail.push(current.clone());
            }
            for expected in trail[..trail.len() - 1].iter().rev() {
                current = nav.previous(&files, Some(&current), order).unwrap();
                assert_eq!(&current, expected);
            }
            assert_eq!(nav.cursor, 0);
            assert_eq!(current, files[0]);
            let before = (
                nav.history.clone(),
                nav.cursor,
                nav.seen.clone(),
                nav.cycle,
                nav.remaining_len,
            );
            let first = nav.prefetch(&files, Some(&current), order, 7, 3);
            assert_eq!(first, nav.prefetch(&files, Some(&current), order, 7, 3));
            assert_eq!(
                before,
                (
                    nav.history.clone(),
                    nav.cursor,
                    nav.seen.clone(),
                    nav.cycle,
                    nav.remaining_len
                )
            );
            let expected = files[nav.past[0]].clone();
            current = nav.previous(&files, Some(&current), order).unwrap();
            assert_eq!(current, expected);
            assert!(nav.future.is_empty());
            assert_eq!(nav.history.len(), 7);
            assert_eq!(
                nav.next(&files, Some(&current), order),
                Some(files[0].clone())
            );
        }
    }
    #[test]
    fn bidirectional_shuffle_plans_replay_history_and_cross_cycles_without_repeats() {
        for count in [1, 2, 5, 31] {
            let files = numbered(count);
            let mut nav = Navigator::default();
            nav.reset(&files, Some(&files[0]));
            let allocation = nav.remaining.as_ptr();
            let mut current = files[0].clone();
            let mut visited = HashSet::from([current.clone()]);
            for (backward, steps) in [(false, count * 3), (true, count * 8), (false, count * 12)] {
                for _ in 0..steps {
                    let before = (
                        nav.cursor,
                        nav.history.len(),
                        nav.cycle,
                        nav.seen.clone(),
                        nav.remaining_len,
                    );
                    let plans = nav.prefetch(&files, Some(&current), Order::Shuffle, 7, 3);
                    assert_eq!(
                        plans,
                        nav.prefetch(&files, Some(&current), Order::Shuffle, 7, 3)
                    );
                    assert_eq!(
                        before,
                        (
                            nav.cursor,
                            nav.history.len(),
                            nav.cycle,
                            nav.seen.clone(),
                            nav.remaining_len
                        )
                    );
                    let fresh = if backward {
                        nav.cursor == 0
                    } else {
                        nav.cursor + 1 == nav.history.len()
                    };
                    let expected = if backward {
                        if fresh {
                            files[nav.past[0]].clone()
                        } else {
                            nav.history[nav.cursor - 1].clone()
                        }
                    } else if fresh {
                        files[nav.future[0]].clone()
                    } else {
                        nav.history[nav.cursor + 1].clone()
                    };
                    let previous = current.clone();
                    let cycle = nav.cycle;
                    current = if backward {
                        nav.previous(&files, Some(&current), Order::Shuffle)
                    } else {
                        nav.next(&files, Some(&current), Order::Shuffle)
                    }
                    .unwrap();
                    assert_eq!(current, expected);
                    assert_eq!(nav.remaining.as_ptr(), allocation);
                    if fresh {
                        if nav.cycle != cycle {
                            assert_eq!(visited.len(), count);
                            visited.clear();
                            if count > 1 {
                                assert_ne!(current, previous);
                            }
                        }
                        assert!(
                            visited.insert(current.clone()),
                            "repeat in new visits: count={count}, backward={backward}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn order_change_starts_a_new_trail_in_both_directions_and_keeps_seen_pool() {
        let files = numbered(20);
        let mut nav = Navigator::default();
        nav.reset(&files, Some(&files[0]));
        let mut current = files[0].clone();
        for _ in 0..4 {
            current = nav.next(&files, Some(&current), Order::Sequential).unwrap();
        }
        current = nav
            .previous(&files, Some(&current), Order::Sequential)
            .unwrap();
        nav.prefetch(&files, Some(&current), Order::Sequential, 32, 32);
        let seen = nav.seen.clone();
        let pool = nav.remaining.clone();
        let len = nav.remaining_len;
        nav.set_order(Order::Shuffle, Some(&current));
        assert_eq!(nav.history, VecDeque::from([current.clone()]));
        assert_eq!(nav.cursor, 0);
        assert!(nav.future.is_empty() && nav.past.is_empty());
        assert_eq!(
            (nav.seen.clone(), nav.remaining.clone(), nav.remaining_len),
            (seen, pool, len)
        );
        nav.prefetch(&files, Some(&current), Order::Shuffle, 7, 3);
        let planned = files[nav.past[0]].clone();
        assert_eq!(
            nav.previous(&files, Some(&current), Order::Shuffle),
            Some(planned.clone())
        );
        assert_eq!(
            nav.next(&files, Some(&planned), Order::Shuffle),
            Some(current)
        );
    }
    #[test]
    fn order_switch_at_either_end_or_middle_of_a_5000_image_trail_is_immediate_both_ways() {
        let files = numbered(5000);
        for approach in [0, 1, 2] {
            for order in [Order::Random, Order::Shuffle] {
                for prefetch in [0, 7] {
                    for backward in [false, true] {
                        let mut nav = Navigator::default();
                        nav.reset(&files, Some(&files[0]));
                        let mut current = files[0].clone();
                        if approach == 1 {
                            // 1 -> 5000 -> 4999 -> 5000 -> 1: old history is BEHIND us.
                            for _ in 0..2 {
                                current = nav
                                    .previous(&files, Some(&current), Order::Sequential)
                                    .unwrap();
                            }
                            for _ in 0..2 {
                                current =
                                    nav.next(&files, Some(&current), Order::Sequential).unwrap();
                            }
                            assert_eq!(current, files[0]);
                            assert_eq!(nav.cursor, 2);
                        } else {
                            for _ in 0..3 {
                                current =
                                    nav.next(&files, Some(&current), Order::Sequential).unwrap();
                            }
                            for _ in 0..if approach == 0 { 3 } else { 1 } {
                                current = nav
                                    .previous(&files, Some(&current), Order::Sequential)
                                    .unwrap();
                            }
                        }
                        let anchor = current.clone();
                        let seen = nav.seen.clone();
                        nav.prefetch(&files, Some(&current), order, prefetch, prefetch);
                        assert_eq!(nav.history, VecDeque::from([anchor.clone()]));
                        assert_eq!(nav.cursor, 0);
                        assert_eq!(nav.seen, seen);
                        nav.prepare_plan(&files, order, backward, 1, Some(&current));
                        let expected =
                            files[if backward { nav.past[0] } else { nav.future[0] }].clone();
                        current = if backward {
                            nav.previous(&files, Some(&current), order)
                        } else {
                            nav.next(&files, Some(&current), order)
                        }
                        .unwrap();
                        assert_eq!(current, expected);
                        assert_eq!(nav.history.len(), 2);
                        let replay = if backward {
                            nav.next(&files, Some(&current), order)
                        } else {
                            nav.previous(&files, Some(&current), order)
                        };
                        assert_eq!(replay, Some(anchor));
                    }
                }
            }
        }
    }
    #[test]
    fn history_limit_retains_nearest_visits_and_current_when_reduced_in_the_middle() {
        let files = numbered(50);
        let mut nav = Navigator::default();
        nav.reset(&files, Some(&files[0]));
        assert_eq!(nav.history_limit(), DEFAULT_HISTORY_LIMIT);
        assert!(nav.set_history_limit(6));
        let mut current = files[0].clone();
        for _ in 0..20 {
            current = nav.next(&files, Some(&current), Order::Sequential).unwrap();
            assert!(nav.history_len() <= 6);
        }
        assert_eq!(
            nav.history,
            files[15..=20].iter().cloned().collect::<VecDeque<_>>()
        );
        for _ in 0..2 {
            current = nav
                .previous(&files, Some(&current), Order::Sequential)
                .unwrap();
        }
        assert_eq!(current, files[18]);
        let seen = nav.seen.clone();
        let pool = nav.remaining.clone();
        assert!(nav.set_history_limit(4));
        assert_eq!(
            nav.history,
            files[16..=19].iter().cloned().collect::<VecDeque<_>>()
        );
        assert_eq!(nav.history[nav.cursor], current);
        assert_eq!(nav.seen, seen);
        assert_eq!(nav.remaining, pool);
        let back = nav
            .previous(&files, Some(&current), Order::Sequential)
            .unwrap();
        assert_eq!(back, files[17]);
        assert_eq!(
            nav.next(&files, Some(&back), Order::Sequential),
            Some(current)
        );
        nav.set_history_limit(0);
        assert_eq!(nav.history_limit(), 2);
        assert!(nav.history_len() <= 2);
        nav.set_history_limit(usize::MAX);
        assert_eq!(nav.history_limit(), 100_000);
    }
    #[test]
    fn capped_bidirectional_shuffle_keeps_plans_and_cycle_uniqueness() {
        for count in [1, 2, 5, 31] {
            for limit in [2, 7] {
                let files = numbered(count);
                let mut nav = Navigator::default();
                nav.reset(&files, Some(&files[0]));
                nav.set_history_limit(limit);
                let mut current = files[0].clone();
                let mut visited = HashSet::from([current.clone()]);
                let allocation = nav.remaining.as_ptr();
                for (backward, steps) in
                    [(false, count * 3), (true, count * 8), (false, count * 12)]
                {
                    for _ in 0..steps {
                        let before = (
                            nav.cursor,
                            nav.history.clone(),
                            nav.seen.clone(),
                            nav.cycle,
                            nav.remaining_len,
                        );
                        let first = nav.prefetch(&files, Some(&current), Order::Shuffle, 7, 3);
                        assert_eq!(
                            first,
                            nav.prefetch(&files, Some(&current), Order::Shuffle, 7, 3)
                        );
                        assert_eq!(
                            before,
                            (
                                nav.cursor,
                                nav.history.clone(),
                                nav.seen.clone(),
                                nav.cycle,
                                nav.remaining_len
                            )
                        );
                        let fresh = if backward {
                            nav.cursor == 0
                        } else {
                            nav.cursor + 1 == nav.history.len()
                        };
                        let expected = if backward {
                            if fresh {
                                files[nav.past[0]].clone()
                            } else {
                                nav.history[nav.cursor - 1].clone()
                            }
                        } else if fresh {
                            files[nav.future[0]].clone()
                        } else {
                            nav.history[nav.cursor + 1].clone()
                        };
                        let cycle = nav.cycle;
                        current = if backward {
                            nav.previous(&files, Some(&current), Order::Shuffle)
                        } else {
                            nav.next(&files, Some(&current), Order::Shuffle)
                        }
                        .unwrap();
                        assert_eq!(current, expected);
                        assert!(nav.history_len() <= limit);
                        assert_eq!(nav.history[nav.cursor], current);
                        assert_eq!(nav.remaining.as_ptr(), allocation);
                        if fresh {
                            if nav.cycle != cycle {
                                assert_eq!(visited.len(), count);
                                visited.clear();
                            }
                            assert!(visited.insert(current.clone()));
                        }
                    }
                }
            }
        }
    }
}
