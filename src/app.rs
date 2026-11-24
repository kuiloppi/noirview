use crate::icons::{self, Icon};
use crate::{
    audio::Music,
    folder::{self, FolderLoader, Progress, Update},
    imaging::Images,
    input::{Dwell, WheelNavigation},
    magnifier::{self, Magnifier},
    metadata,
    model::*,
    operations::{self, Conflict, MoveItem},
    startup::{Loaded, Startup},
    state::{Notice, StateWriter},
    storage::{self, Store},
};
use crossbeam_channel::{unbounded, Receiver, Sender};
use eframe::egui::{self, Color32, Id, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const GREEN: Color32 = Color32::from_rgb(110, 224, 176);
const RED: Color32 = Color32::from_rgb(243, 101, 121);
/// The Favorite heart under grid images.
const HEART_RED: Color32 = Color32::from_rgb(235, 70, 80);
/// Move selected mode: the Move button's mark and the chosen tiles' borders.
const MOVE_BLUE: Color32 = Color32::from_rgb(90, 160, 255);
/// Settings' pages, in the column at the left.
const SETTINGS_PAGES: &[&str] = &[
    "Image view",
    "Browsing",
    "Grid",
    "Bars & buttons",
    "Move & delete",
    "Folders & tags",
    "Performance",
    "Music",
    "Upscaling (AI)",
    "Profiles & backup",
    "Shortcuts & gestures",
    "Advanced",
];
/// What each Settings page's Reset sets back to the defaults (lists of
/// folders and files, chosen paths and remembered places stay).
const SETTINGS_RESET: [&[&str]; 12] = [
    &[
        "display_mode",
        "animate_gif",
        "animation_mb",
        "magnifier",
        "lens_px",
        "lens_zoom",
        "lens_border",
        "lens_soft_edges",
        "lens_feather_px",
        "lens_full_window",
        "jpeg_preview",
        "keep_previous_image",
        "fullscreen_image_only",
        "image_title_folder",
        "path_forward_slashes",
        "title_raise",
    ],
    &[
        "order",
        "sort_key",
        "sort_descending",
        "wheel_up_next",
        "random_wheel_both",
        "history_limit",
        "keep_selected_tags",
        "round_notice",
        "round_notice_dialog",
        "round_notice_size",
        "round_notice_secs",
        "round_notice_opacity",
        "round_notice_images",
        "round_notice_fade",
        "round_notice_fade_ms",
        "round_notice_place",
        "round_notice_color",
        "edges",
        "side_px",
        "side_ms",
        "top_delete",
        "top_px",
        "top_ms",
        "slideshow_secs",
        "slideshow_order",
        "slideshow_side_px",
        "slideshow_side_ms",
        "slideshow_top_px",
        "slideshow_top_ms",
    ],
    &[
        "thumbnail_count",
        "grid_round_full",
        "grid_page_arrows",
        "grid_open_at_current",
        "grid_random_both",
        "filter_per_view",
        "grid_wheel_pages",
        "grid_page_wrap",
        "confirm_preset_remove",
        "grid_start_mode",
        "grid_bar_icons",
        "grid_controls_top",
        "grid_hide_toolbar_text",
        "grid_bar_fixed_width",
        "grid_bar_fixed",
        "grid_bar_bottom",
        "grid_hide_toolbar",
        "grid_bar_colors",
        "grid_text_colors",
        "grid_text_font_colors",
        "grid_keep_tag_mode",
        "grid_show_marks",
        "grid_marks_icons",
        "grid_caption_range",
        "grid_caption_folder",
        "grid_caption_count",
        "grid_hover_preview",
        "hover_preview_ms",
        "hover_circle",
        "hover_circle_pct",
        "hover_circle_reveal_px",
        "hover_circle_fade",
        "hover_circle_always",
        "hover_circle_opacity",
        "hover_circle_width",
        "preview_size",
        "preview_close_top",
        "preview_close_left",
        "preview_close_right",
        "preview_close_click",
        "preview_close_px",
        "preview_close_ms",
        "preview_close_move_px",
        "preview_close_move_dir",
        "normal_keeps_selection",
        "selection_flash",
        "selection_toast",
        "selection_toast_size",
        "selection_toast_width",
        "selection_toast_height",
        "grid_viewer_browse",
        "grid_hold_ms",
        "magnify_on_move",
        "magnify_at_once",
    ],
    &[
        "bar_width",
        "tags_max_width",
        "bar_height",
        "bar_bottom",
        "bar_bottom_grid",
        "bar_locked",
        "bar_lock_universal",
        "bar_locked_grid",
        "bar_offset",
        "bar_color",
        "show_favorites_button",
        "bar_edge_px",
        "bar_delay_ms",
        "bar_buttons",
        "hover",
        "hover_trash",
        "hover_move",
        "hover_settings",
        "hover_ms",
        "destructive_ms",
        "hover_ring",
        "delete_size",
        "delete_color",
        "delete_opacity",
        "delete_border",
        "delete_border_width",
        "delete_border_color",
        "close_angle",
        "close_size",
        "move_color",
    ],
    &[
        "confirm_delete",
        "confirm_restore",
        "forget_deletion_on_close",
        "verified_move",
        "move_keep_names",
        "move_digits",
        "move_fill_gaps",
        "move_grid_all",
        "move_grid_count",
        "move_grid_round_full",
        "move_grid_row",
        "move_grid_browse",
        "move_grid_marks",
        "move_x_color",
        "move_x_corner",
        "move_x_offset",
        "move_mark_corner",
        "move_mark_offset",
        "move_grid_frame",
        "move_grid_frame_color",
        "move_grid_frame_width",
        "move_grid_frame_opacity",
        "move_x_size",
        "move_x_opacity",
        "grid_move_marks",
    ],
    &[
        "recursive",
        "remember_favorite_places",
        "favorite_style",
        "watch_folders",
        "watch_while_active",
        "watch_when_unfocused",
        "watch_background",
        "watch_pace",
        "watch_use_journal",
    ],
    &[
        "loading_mode",
        "cache_while_active",
        "cache_when_unfocused",
        "proxy_cache_mb",
        "proxy_min_side",
        "prefetch_ahead",
        "prefetch_behind",
        "thumbnail_cache_mb",
        "cache_mb",
        "memory_use",
        "low_latency_display",
        "graphics_backend",
        "remember_window",
    ],
    &["music_shuffle", "volume"],
    &["upscale", "upscale_factor", "sharpen", "tile_size"],
    &[
        "backup_exit",
        "backup_schedule",
        "backup_days",
        "backup_hour",
        "backup_minute",
        "backup_keep",
    ],
    &[
        "shortcuts",
        "gestures",
        "gesture_opens_tile",
        "gesture_trail",
        "gesture_trail_color",
        "gesture_trail_width",
        "gesture",
        "gesture_min_px",
        "gesture_max_ms",
    ],
    &[
        "settings_search_opacity",
        "diagnostic_log",
        "window_diagnostics",
    ],
];
const SETTINGS_FOLDERS: usize = 5;
const SETTINGS_PROFILES: usize = 9;
/// The profile choice for no profile in use.
const NO_PROFILE: &str = "None";
const TAG_YELLOW: Color32 = Color32::from_rgb(245, 202, 83);
const MUTED: Color32 = Color32::from_rgb(130, 142, 156);
#[derive(Clone)]
struct EventSender {
    tx: Sender<Event>,
    ctx: egui::Context,
}
impl EventSender {
    fn send(&self, event: Event) -> Result<(), crossbeam_channel::SendError<Event>> {
        self.tx.send(event)?;
        self.ctx.request_repaint();
        Ok(())
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    None,
    Tags,
    Favorites,
    Filter,
}
enum Event {
    /// Copy image: the decoded pixels, ready for the clipboard.
    CopyImage(Result<egui::ColorImage, String>),
    Open(PathBuf),
    Destination(PathBuf),
    Tracks(Vec<PathBuf>),
    Model(PathBuf),
    Runtime(PathBuf),
    ExtraBackup(PathBuf),
    ThumbnailCacheDir(PathBuf),
    /// A folder chosen for the favorite folders (new, or replacing one).
    FavoriteFolder(PathBuf),
    /// Where a kind of file dialog was when it closed.
    DialogPlace(String, [i32; 2]),
    WatchFolder(PathBuf),
    /// USN service state and its version (if it answered).
    UsnStatus(crate::usn::State, Option<String>),
    /// Result of installing, removing or testing the USN service.
    UsnDone(String),
    /// The background watcher handed over (or another window keeps it).
    WatchOwner(Option<std::fs::File>),
    RestoreChoice(PathBuf),
    Tagged(Vec<(PathBuf, Result<BTreeSet<String>, String>)>),
    KeywordsRead {
        request: u64,
        path: PathBuf,
        result: Result<BTreeSet<String>, String>,
    },
    Deleted(Vec<PathBuf>, Vec<String>),
    /// Images moved back from the Recycle Bin, and errors.
    Restored(Vec<PathBuf>, Vec<String>),
    Moved(operations::MoveReport),
    State(Notice),
    ThumbnailCacheCleared(Result<(), String>),
    ThumbnailCacheFlushed,
    Recovered(Result<Vec<(PathBuf, PathBuf)>, String>),
    /// Files sorted by a metadata key on a worker: (request, files).
    Sorted(u64, Vec<PathBuf>),
}
/// Page load times: until everything was on screen, and how long workers
/// were preparing the thumbnails (also when that happened in advance).
type PageTimes = (Duration, Option<Duration>);
/// Timing of one image in the viewer, for verifying first views.
struct ViewTimer {
    path: PathBuf,
    opened: Instant,
    /// Reduced picture shown before full detail: (after, longer side in px).
    first: Option<(Duration, usize)>,
    /// A larger reduced picture that replaced it (a stored first view after
    /// the grid thumbnail).
    better: Option<(Duration, usize)>,
    sharp: Option<Duration>,
}
impl ViewTimer {
    fn summary(&self) -> Option<String> {
        let sharp = self.sharp?.as_millis();
        if self.first.is_none() {
            return Some(format!(
                "full detail after {sharp} ms, no first view needed"
            ));
        }
        let mut text = String::new();
        for (after, side) in self.first.iter().chain(&self.better) {
            text += &format!(
                "{} {side} px after {} ms · ",
                first_view_name(*side),
                after.as_millis()
            );
        }
        Some(format!("{text}full detail after {sharp} ms"))
    }
    /// A reduced picture arrived on screen.
    fn reduced(&mut self, side: usize) {
        let shown = self.better.or(self.first).map(|(_, side)| side);
        let at = (self.opened.elapsed(), side);
        match shown {
            None => self.first = Some(at),
            Some(old) if side > old => self.better = Some(at),
            _ => {}
        }
    }
    /// A stored first view (not just a thumbnail) was on screen.
    fn had_first_view(&self) -> bool {
        self.first
            .iter()
            .chain(&self.better)
            .any(|(_, side)| *side > 1024)
    }
}
/// Stored first views are larger than any thumbnail or embedded preview.
/// The notice after a graphics choice did not start last time.
fn backend_failed_notice(failed: &str) -> String {
    if failed.contains("OpenGL") {
        format!("{failed} did not start; NoirView now uses Vulkan / DirectX 12. Choose OpenGL in Settings → Display to try it again.")
    } else {
        format!(
            "The graphics backend \"{failed}\" did not start; NoirView uses the automatic choice."
        )
    }
}

/// Paint times for the diagnostic log: frames that uploaded textures (count,
/// MB, average and longest ms) and the others (count, average ms). On
/// Vulkan / DirectX 12 the wait for the display is left out.
fn paint_summary(p: &eframe::noirview_diag::PaintStats) -> String {
    let average = |ms: f64, n: u64| if n == 0 { 0.0 } else { ms / n as f64 };
    format!(
        "{} uploads {:.1} MB avg {:.1} ms max {:.1} ms · {} frames avg {:.2} ms",
        p.upload_frames,
        p.upload_bytes as f64 / 1_048_576.0,
        average(p.upload_ms, p.upload_frames),
        p.upload_max_ms,
        p.plain_frames,
        average(p.plain_ms, p.plain_frames),
    )
}

fn first_view_name(side: usize) -> &'static str {
    if side > 1024 {
        "First view"
    } else {
        "Preview"
    }
}
/// NoirView's window, for file and folder dialogs: owned by it, a dialog
/// stays in front of it (and the window waits for the dialog).
struct ParentWindow {
    window: wgpu::rwh::RawWindowHandle,
    display: wgpu::rwh::RawDisplayHandle,
}
impl wgpu::rwh::HasWindowHandle for ParentWindow {
    fn window_handle(&self) -> Result<wgpu::rwh::WindowHandle<'_>, wgpu::rwh::HandleError> {
        // SAFETY: the handle of the app's own window, which outlives the
        // dialogs (they end before the window closes).
        Ok(unsafe { wgpu::rwh::WindowHandle::borrow_raw(self.window) })
    }
}
impl wgpu::rwh::HasDisplayHandle for ParentWindow {
    fn display_handle(&self) -> Result<wgpu::rwh::DisplayHandle<'_>, wgpu::rwh::HandleError> {
        // SAFETY: as above.
        Ok(unsafe { wgpu::rwh::DisplayHandle::borrow_raw(self.display) })
    }
}
/// Clears the open-dialog flag when the dialog thread ends (also on panic).
struct PickDone(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for PickDone {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}
/// The small window above a held button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Popup {
    /// Open folder held: the favorite folders.
    Favorites,
    /// Go to held: an image or page number.
    GoTo,
    /// The Move dialog's Favourites: a favorite folder as the destination.
    Destination,
    /// The Profile button: the profiles as text.
    Profiles,
}
#[derive(Clone)]
enum Dialog {
    RemovePreset(usize),
    RemoveTags(BTreeSet<String>),
    Delete(Vec<PathBuf>),
    /// Restore deleted: these images back from the Recycle Bin?
    RestoreDeleted(Vec<PathBuf>),
    /// Every image of the list has been shown (round notice).
    RoundDone(usize),
    /// Delete this profile?
    DeleteProfile(String),
    /// Set this Settings page back to the defaults?
    ResetSettingsPage(usize),
    Move(MovePlan),
    Conflict(Vec<MoveItem>),
    Restore(PathBuf),
    Recover(PathBuf),
    ClearThumbnails,
    Rescan,
    Reindex,
    Error(String),
}
/// What the marked images' Move grid replaced (restored on leaving it).
struct MoveGridSaved {
    filter: Filter,
    current: Option<PathBuf>,
    grid_normal: bool,
}
/// The Move dialog: what to move, how to name it, and the resulting plan
/// (made again when the destination or the naming changes).
#[derive(Clone)]
struct MovePlan {
    sources: Vec<PathBuf>,
    keep_names: bool,
    /// Chosen images (Move selected) rather than the whole folder.
    selected: bool,
    /// Sequence names' width, and whether free numbers come first
    /// (Settings → Library).
    digits: usize,
    fill_gaps: bool,
    /// The image shown (the floating Move button), or the images marked
    /// with it.
    one: bool,
    marked: bool,
    items: Result<Vec<MoveItem>, String>,
}
impl MovePlan {
    fn new(
        sources: Vec<PathBuf>,
        keep_names: bool,
        selected: bool,
        dest: &Path,
        (digits, fill_gaps): (usize, bool),
    ) -> Self {
        let mut plan = Self {
            sources,
            keep_names,
            selected,
            digits,
            fill_gaps,
            one: false,
            marked: false,
            items: Ok(vec![]),
        };
        plan.replan(dest);
        plan
    }
    fn replan(&mut self, dest: &Path) {
        self.items = operations::plan_names(
            &self.sources,
            dest,
            self.keep_names,
            self.digits,
            self.fill_gaps,
        )
        .map_err(|e| format!("{e:#}"));
    }
}
/// Where a hover-activated control is; each area has its own setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum HoverArea {
    /// Toolbar, grid controls, menus, dialogs (`Settings::hover`).
    #[default]
    General,
    /// The floating trash button (`Settings::hover_trash`).
    Trash,
    /// The Settings window (`Settings::hover_settings`).
    Settings,
    /// The floating Move button (`Settings::hover_move`).
    Move,
}
#[derive(Default)]
struct Hover {
    states: HashMap<Id, Dwell>,
    responses: Vec<(egui::Response, bool)>,
    pending: Option<Pos2>,
    /// Any area on: dwell tracking runs.
    enabled: bool,
    general: bool,
    trash: bool,
    settings: bool,
    moving: bool,
    /// Area of controls registered with `watch`.
    area: HoverArea,
    /// Draw the green ring that fills up before a control activates.
    ring: bool,
    normal: Duration,
    danger: Duration,
    /// Settings' search (lower case): matching controls are outlined and
    /// their labels kept in `found`.
    search: String,
    /// Search with typos allowed (Settings' Looser matching).
    loose: bool,
    /// Opacity of the search outlines.
    outline: f32,
    found: Vec<String>,
    /// Where the first match was drawn.
    first_found: Option<Rect>,
}
impl Hover {
    /// A control, text or title with the search words: outlined, listed.
    fn found(&mut self, ui: &egui::Ui, rect: Rect, label: &str) {
        if self.search.is_empty() || !search_matches(label, &self.search, self.loose) {
            return;
        }
        self.found.push(label.to_owned());
        self.first_found.get_or_insert(rect);
        ui.painter().rect_stroke(
            rect.expand(3.),
            4.,
            Stroke::new(
                1.5_f32,
                Color32::from_rgb(240, 200, 80).gamma_multiply(self.outline.clamp(0.1, 1.)),
            ),
            egui::StrokeKind::Outside,
        );
    }
    fn begin(&mut self, s: &Settings) {
        self.responses.clear();
        self.outline = s.settings_search_opacity;
        self.general = s.hover;
        self.trash = s.hover_trash();
        self.settings = s.hover_settings();
        self.moving = s.hover_move();
        self.enabled = self.general || self.trash || self.settings || self.moving;
        self.area = HoverArea::General;
        self.ring = s.hover_ring;
        self.normal = Duration::from_millis(s.hover_ms);
        self.danger = Duration::from_millis(s.destructive_ms.max(s.hover_ms));
        if !self.enabled {
            self.states.clear();
            self.pending = None;
        }
    }
    fn allows(&self, area: HoverArea) -> bool {
        match area {
            HoverArea::General => self.general,
            HoverArea::Trash => self.trash,
            HoverArea::Settings => self.settings,
            HoverArea::Move => self.moving,
        }
    }
    fn watch(&mut self, r: egui::Response, danger: bool) -> egui::Response {
        self.watch_in(self.area, r, danger)
    }
    fn watch_in(&mut self, area: HoverArea, r: egui::Response, danger: bool) -> egui::Response {
        if self.allows(area) {
            self.responses.push((r.clone(), danger));
        }
        r
    }
    fn button(
        &mut self,
        ui: &mut egui::Ui,
        label: impl Into<egui::WidgetText>,
        active: bool,
        enabled: bool,
        danger: bool,
    ) -> bool {
        let label: egui::WidgetText = label.into();
        let text = label.text().to_owned();
        let button = egui::Button::new(label).selected(active);
        let r = ui.add_enabled(enabled, button);
        self.found(ui, r.rect, &text);
        self.watch(r, danger).clicked()
    }
    fn icon(
        &mut self,
        ui: &mut egui::Ui,
        icon: Icon,
        label: &str,
        active: bool,
        enabled: bool,
    ) -> bool {
        self.icon_response(ui, icon, label, active, enabled, None, false)
            .clicked()
    }
    /// An icon button in the toolbar style; `color` replaces the normal
    /// icon colour (the grid icon bar's per-button colours).
    #[allow(clippy::too_many_arguments)]
    fn icon_response(
        &mut self,
        ui: &mut egui::Ui,
        icon: Icon,
        label: &str,
        active: bool,
        enabled: bool,
        color: Option<Color32>,
        danger: bool,
    ) -> egui::Response {
        let response = ui
            .push_id(label, |ui| {
                ui.add_enabled(
                    enabled,
                    egui::Button::new("")
                        .min_size(Vec2::splat(38.))
                        .frame(false),
                )
            })
            .inner;
        let color = match color {
            Some(c) if enabled => c,
            Some(c) => c.gamma_multiply(0.35),
            None if !enabled => MUTED.gamma_multiply(0.4),
            None if active => GREEN,
            None => Color32::from_rgb(194, 206, 216),
        };
        if response.hovered() || active {
            ui.painter().rect_filled(
                response.rect,
                10.,
                if active {
                    Color32::from_rgb(34, 68, 55)
                } else {
                    Color32::from_rgb(45, 54, 65)
                },
            );
        }
        icons::paint(ui.painter(), response.rect.center(), icon, color);
        self.watch(response.on_hover_text(label), danger)
    }
    /// A text on a Settings page (found by the search too).
    fn note(&mut self, ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) -> egui::Response {
        let text: egui::WidgetText = text.into();
        let words = text.text().to_owned();
        let r = ui.label(text);
        self.found(ui, r.rect, &words);
        r
    }
    /// A section's title within a Settings page (found by the search too).
    fn section(&mut self, ui: &mut egui::Ui, title: &str) {
        ui.add_space(14.);
        let r = ui.label(RichText::new(title).strong().size(15.).color(GREEN));
        self.found(ui, r.rect, title);
        ui.add_space(2.);
    }
    fn check(&mut self, ui: &mut egui::Ui, value: &mut bool, label: &str) {
        let r = ui.checkbox(value, label);
        self.found(ui, r.rect, label);
        self.watch(r, false);
    }
    fn slider<T: egui::emath::Numeric>(
        &mut self,
        ui: &mut egui::Ui,
        value: &mut T,
        range: std::ops::RangeInclusive<T>,
        label: &str,
    ) {
        let r = ui.add(egui::Slider::new(value, range).text(label));
        self.found(ui, r.rect, label);
        self.watch(r, false);
    }
    fn finish(&mut self, ctx: &egui::Context) {
        if !self.enabled {
            return;
        }
        if ctx.input(|i| i.pointer.any_down()) {
            self.states.clear();
            self.pending = None;
            return;
        }
        let now = Instant::now();
        let selected = self
            .responses
            .iter()
            .rev()
            .find(|(r, _)| r.hovered() && r.enabled())
            .map(|(r, d)| (r.id, *d));
        let ids: HashSet<_> = self.responses.iter().map(|(r, _)| r.id).collect();
        self.states.retain(|id, _| ids.contains(id));
        let mut progress = 0.;
        for id in ids {
            let active = selected.map(|v| v.0 == id).unwrap_or(false);
            let delay = if selected.map(|v| v.1).unwrap_or(false) {
                self.danger
            } else {
                self.normal
            };
            let dwell = self.states.entry(id).or_default();
            if dwell.update(active, delay, now) {
                self.pending = ctx.pointer_hover_pos();
            }
            if active {
                progress = dwell.progress(delay, now);
            }
        }
        if self.pending.is_some() {
            ctx.request_repaint();
        }
        if self.states.values().any(Dwell::pending) {
            ctx.request_repaint_after(crate::repaint::ANIMATION);
        }
        if self.ring && progress > 0. && progress < 1. {
            if let Some(pos) = ctx.pointer_hover_pos() {
                let points: Vec<_> = (0..=40)
                    .map(|i| {
                        let a = -std::f32::consts::FRAC_PI_2
                            + std::f32::consts::TAU * progress * i as f32 / 40.;
                        pos + Vec2::angled(a) * 14.
                    })
                    .collect();
                ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("dwell")))
                    .add(egui::Shape::line(points, Stroke::new(2.5_f32, GREEN)));
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }
    }
}
/// Tiles of a grid page as they were when images on it were deleted; kept on
/// screen (deleted ones with a red overlay) until the page changes.
struct GridGhosts {
    page: Vec<PathBuf>,
    deleted: HashSet<PathBuf>,
    /// Thumbnails of the deleted images, kept so the overlay covers the image.
    textures: HashMap<PathBuf, (egui::TextureHandle, [u32; 2])>,
    start: usize,
    count: usize,
}
/// List length, first and last path, enabled, cache size.
type PregenKey = (
    usize,
    Option<PathBuf>,
    Option<PathBuf>,
    crate::model::LoadingMode,
    usize,
);
#[derive(Clone, Copy)]
enum TagAction {
    Create,
    Favorites,
    Additional,
    Lock,
    Remove,
    Save,
}
struct TagControls {
    favorites: bool,
    additional: bool,
    locked: bool,
    enabled: bool,
}
struct TagView {
    width: f32,
    max_height: f32,
    controls: Option<TagControls>,
    enabled: bool,
}
/// Windowed mode: leaving the window through this bottom strip opens the bar.
const BAR_EXIT_PX: f32 = 48.;
#[derive(Default)]
struct UsnView {
    state: Option<crate::usn::State>,
    version: Option<String>,
    /// A check or change is running.
    busy: bool,
    checked: Option<Instant>,
    message: Option<String>,
}
#[derive(Default)]
struct WatchState {
    /// Take-over started (once, after the library is loaded).
    requested: bool,
    /// The owner lock; None while waiting or when another window keeps it.
    lock: Option<std::fs::File>,
    /// Another open NoirView keeps the watcher.
    refused: bool,
    handle: Option<crate::watch::Handle>,
    cache: Option<std::sync::Arc<crate::thumbnail_cache::ThumbnailCache>>,
    /// The configuration last written for the background process.
    config: Vec<u8>,
    /// Sign-in registration last applied.
    autostart: Option<bool>,
    /// The background process's last status, read at the hand-over.
    background: Option<crate::watch::Status>,
    error: Option<String>,
    /// The journal source would be used (service installed and allowed).
    journal: bool,
    journal_checked: Option<Instant>,
}
/// Opacity of the hover-preview circle at `gap` px outside it (negative =
/// inside): full inside; within `reveal` px outside it fainter, fading in
/// as the pointer approaches when `fade` is on; hidden farther away.
fn ring_alpha(gap: f32, reveal: f32, fade: bool, opacity: f32) -> Option<u8> {
    let full = opacity.clamp(0.004, 1.) * 255.;
    if gap <= 0. {
        return Some(full as u8);
    }
    if gap > reveal.max(0.) {
        return None;
    }
    let t = if fade { 1. - gap / reveal } else { 1. };
    let eased = t * t * (3. - 2. * t);
    let alpha = full * 0.6 * eased;
    (alpha >= 1.).then_some(alpha as u8)
}
/// How many recently viewed images the right-click menu offers.
const RECENT: usize = 5;
/// Subfolder created inside a user-chosen cache location.
const CACHE_FOLDER: &str = "NoirView-thumbnails";
/// The chosen location when it can be created, otherwise the default inside
/// the data folder (a missing drive must not disable the cache).
fn thumbnail_cache_root(chosen: &Path, data: Option<&Path>) -> Option<PathBuf> {
    if !chosen.as_os_str().is_empty() {
        let root = chosen.join(CACHE_FOLDER);
        if std::fs::create_dir_all(&root).is_ok() {
            return Some(root);
        }
    }
    data.map(|p| p.join("thumbnails"))
}
fn settle_panel(ctx: &egui::Context, name: &str, size: Vec2) {
    let id = Id::new(("panel_size", name));
    let changed = ctx.data_mut(|data| {
        let old = data.get_temp::<Vec2>(id);
        data.insert_temp(id, size);
        old.is_none_or(|old| (old - size).length_sq() > 0.25)
    });
    if changed {
        ctx.request_repaint();
    }
}
/// One control of the grid's controls (text buttons or icon bar).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum GridItem {
    PrevPage,
    NextPage,
    Normal,
    HoverPreview,
    Sort,
    Pages,
    Trash,
    /// Restore deleted.
    Undo,
    /// Go to page 1; holding asks for a page.
    GoTo,
    Tags,
    SelectView,
    SelectFolder,
    Clear,
    Restore,
    CloseGrid,
    Fullscreen,
    /// − n + and the page-size bookmark.
    Size,
    Presets,
    Move,
    Filter,
    Folder,
    FloatingTrash,
    Lock,
    /// The profiles, as text above the button.
    Profile,
    /// The Move grid: select the page (hold: all), the destination (hold:
    /// favorites), keep names, move, cancel.
    SelectBoth,
    Destination,
    KeepNames,
    MoveMarked,
    CancelMarked,
    /// Page number, timings, caching, hint.
    Status,
}
/// An order with its "no history" variant (both directions show new ones).
fn order_choice_label(order: crate::model::Order, no_history: bool) -> String {
    match (order, no_history) {
        (crate::model::Order::Sequential, _) | (_, false) => order.label().to_owned(),
        (_, true) => format!("{} (no hist.)", order.label()),
    }
}
/// The order choices: In order, Random, Shuffle and both without history.
const ORDER_CHOICES: [(crate::model::Order, bool); 5] = [
    (crate::model::Order::Sequential, false),
    (crate::model::Order::Random, false),
    (crate::model::Order::Shuffle, false),
    (crate::model::Order::Random, true),
    (crate::model::Order::Shuffle, true),
];
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GridMenu {
    Sort,
    Pages,
}
fn sort_label(key: crate::sorting::SortKey, descending: bool) -> String {
    format!("{} {}", key.label(), if descending { "⏷" } else { "⏶" })
}
fn page_order_label(order: crate::model::Order) -> &'static str {
    match order {
        crate::model::Order::Sequential => "In order",
        crate::model::Order::Random => "Random",
        crate::model::Order::Shuffle => "Shuffle",
    }
}
/// The number of marked images on an icon's corner.
/// NOIRVIEW, "/" and the title on one baseline: each glyph's baseline is
/// known, so the smaller title is placed by it, then raised by `raise`
/// points. A long title is cut with "…" (the whole of it on hover).
fn title_line(ui: &mut egui::Ui, title: Option<String>, width: f32, raise: f32) {
    use egui::text::{LayoutJob, TextFormat};
    let format =
        |size: f32, color: Color32| TextFormat::simple(egui::FontId::proportional(size), color);
    let mut head = LayoutJob::default();
    head.append("N O I R V I E W", 0., format(13., GREEN));
    if title.is_some() {
        head.append("/", 8., format(13., MUTED));
    }
    let head = ui.fonts(|f| f.layout_job(head));
    let baseline = |g: &egui::Galley| {
        g.rows
            .first()
            .and_then(|r| r.glyphs.first())
            .map_or(g.size().y, |glyph| glyph.pos.y)
    };
    let tail = title.as_ref().map(|t| {
        let mut job = LayoutJob::single_section(t.clone(), format(12., MUTED));
        job.wrap.max_width = (width - head.size().x - 8.).max(40.);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        ui.fonts(|f| f.layout_job(job))
    });
    // The title's top relative to the head's top. Only NOIRVIEW and "/"
    // take room in the row, so raising the title never moves them.
    let tail_top = tail
        .as_ref()
        .map_or(0., |t| baseline(&head) - baseline(t) - raise);
    let tail_width = tail.as_ref().map_or(0., |t| 8. + t.size().x);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(head.size().x + tail_width, head.size().y),
        Sense::hover(),
    );
    let origin = rect.min;
    if let (Some(t), Some(title)) = (&tail, &title) {
        if t.elided {
            response.on_hover_text(title.clone());
        }
    }
    let painter = ui.painter();
    painter.galley(origin, head.clone(), Color32::WHITE);
    if let Some(t) = tail {
        painter.galley(
            origin + Vec2::new(head.size().x + 8., tail_top),
            t,
            Color32::WHITE,
        );
    }
}

/// The GUID under which Windows keeps a file dialog's position, size and
/// last folder: one per kind of dialog, the same in every NoirView version.
fn dialog_guid(kind: &str) -> u128 {
    const KINDS: [&str; 12] = [
        "open",
        "file",
        "destination",
        "tracks",
        "model",
        "runtime",
        "backup",
        "cache_dir",
        "favorite",
        "watch",
        "restore",
        "other",
    ];
    let index = KINDS
        .iter()
        .position(|k| *k == kind)
        .unwrap_or(KINDS.len() - 1);
    0x6e6f_6972_7669_4577_9d4c_3a51_7e20_0000 + index as u128
}

/// Settings' search: every word of `query` (lower case) is in `text`, in
/// any order; `loose` also takes a word with one letter wrong, missing or
/// extra (two for long words) and the start of a word.
fn search_matches(text: &str, query: &str, loose: bool) -> bool {
    let text = text.to_lowercase();
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    query.split_whitespace().all(|q| {
        text.contains(q)
            || (loose
                && q.chars().count() >= 3
                && words.iter().any(|w| {
                    let allowed = if q.chars().count() >= 8 { 2 } else { 1 };
                    w.starts_with(q) || edit_distance(w, q) <= allowed
                }))
    })
}
/// Letters to change, add or remove to turn one word into the other.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut previous = row[0];
        row[0] = i;
        for j in 1..=b.len() {
            let here = row[j];
            row[j] = if a[i - 1] == b[j - 1] {
                previous
            } else {
                1 + previous.min(row[j]).min(row[j - 1])
            };
            previous = here;
        }
    }
    row[b.len()]
}
/// A corner choice and the offset from it (Settings).
fn corner_controls(
    ui: &mut egui::Ui,
    h: &mut Hover,
    label: &str,
    corner: &mut String,
    offset: &mut [f32; 2],
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        for (value, name) in [
            ("top_left", "Top left"),
            ("top_right", "Top right"),
            ("bottom_left", "Bottom left"),
            ("bottom_right", "Bottom right"),
        ] {
            if h.button(ui, name, corner == value, true, false) {
                *corner = value.to_owned();
            }
        }
    });
    h.slider(
        ui,
        &mut offset[0],
        0.0..=400.0,
        &format!("{label}: in from the side (px)"),
    );
    h.slider(
        ui,
        &mut offset[1],
        0.0..=400.0,
        &format!("{label}: in from the top / bottom (px)"),
    );
}
/// Where something of `size` goes in `inner`: at a corner ("top_left",
/// "top_right", "bottom_left", "bottom_right"), moved in by `offset`
/// (right / down from a left / top corner, left / up from the others),
/// kept inside.
fn corner_place(inner: Rect, size: Vec2, corner: &str, offset: [f32; 2]) -> Pos2 {
    let (right, bottom) = match corner {
        "top_right" => (true, false),
        "bottom_left" => (false, true),
        "bottom_right" => (true, true),
        _ => (false, false),
    };
    let x = if right {
        inner.right() - size.x - offset[0]
    } else {
        inner.left() + offset[0]
    };
    let y = if bottom {
        inner.bottom() - size.y - offset[1]
    } else {
        inner.top() + offset[1]
    };
    Pos2::new(
        x.clamp(inner.left(), (inner.right() - size.x).max(inner.left())),
        y.clamp(inner.top(), (inner.bottom() - size.y).max(inner.top())),
    )
}
/// The size of the "✔ MOVE" mark.
fn move_mark_size(painter: &egui::Painter) -> Vec2 {
    painter
        .layout_no_wrap(
            "✔ MOVE".to_owned(),
            egui::FontId::proportional(13.),
            Color32::WHITE,
        )
        .size()
        + Vec2::new(16., 8.)
}
/// The mark of an image marked to move: a blue tag with a check mark.
fn paint_move_mark(painter: &egui::Painter, at: Pos2) {
    let galley = painter.layout_no_wrap(
        "✔ MOVE".to_owned(),
        egui::FontId::proportional(13.),
        Color32::WHITE,
    );
    let rect = Rect::from_min_size(at, galley.size() + Vec2::new(16., 8.));
    painter.rect_filled(rect, 6., MOVE_BLUE.gamma_multiply(0.9));
    painter.rect_stroke(
        rect,
        6.,
        Stroke::new(1.0_f32, Color32::from_white_alpha(180)),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.min + Vec2::new(8., 4.), galley, Color32::WHITE);
}
/// Clickable text without a button's frame: no box on hover or when chosen,
/// only the text's colour changes (chosen: green, hovered: bright).
fn plain_text_link(ui: &mut egui::Ui, text: String, chosen: bool, enabled: bool) -> egui::Response {
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(galley.size(), sense);
    let response = if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    };
    let visuals = ui.visuals();
    let color = if !enabled {
        visuals.weak_text_color()
    } else if response.hovered() {
        visuals.strong_text_color()
    } else if chosen {
        GREEN
    } else {
        visuals.text_color()
    };
    if ui.is_rect_visible(rect) {
        ui.painter().galley(rect.min, galley, color);
    }
    response
}

fn count_badge(ui: &egui::Ui, rect: Rect, n: usize, color: Color32) {
    let center = rect.right_top() + Vec2::new(-7., 7.);
    let text = if n > 999 {
        "999+".to_owned()
    } else {
        n.to_string()
    };
    let galley = ui
        .painter()
        .layout_no_wrap(text, egui::FontId::proportional(10.), Color32::WHITE);
    let size = Vec2::new(galley.size().x.max(9.) + 6., 14.);
    ui.painter()
        .rect_filled(Rect::from_center_size(center, size), 7., color);
    ui.painter()
        .galley(center - galley.size() / 2., galley, Color32::WHITE);
}
use crate::bar_layout::Place;
/// The floating toolbar's buttons: key, name in Settings, icon, default place.
const TOOL_BUTTONS: &[(&str, &str, Icon, Place)] = &[
    ("prev", "Previous image", Icon::Previous, Place::Bar),
    ("next", "Next image", Icon::Next, Place::Bar),
    ("tags", "Tags", Icon::Tag, Place::Bar),
    (
        "floating_trash",
        "Floating trash button",
        Icon::Trash,
        Place::Bar,
    ),
    ("undo", "Restore deleted", Icon::Undo, Place::Bar),
    ("favorites", "Favorites", Icon::Heart, Place::Bar),
    ("filter", "Filter", Icon::Filter, Place::Bar),
    ("order", "Browsing order", Icon::Order, Place::Bar),
    ("slideshow", "Slideshow", Icon::Slideshow, Place::Bar),
    ("grid", "Thumbnail grid", Icon::Grid, Place::Bar),
    ("music", "Music", Icon::Music, Place::Bar),
    ("fullscreen", "Fullscreen", Icon::Fullscreen, Place::Bar),
    (
        "image_only",
        "Fullscreen shows only the image",
        Icon::ImageOnly,
        Place::Group,
    ),
    (
        "round",
        "Notice when all images are shown",
        Icon::Round,
        Place::Group,
    ),
    (
        "goto",
        "Go to image 1 (hold: a number)",
        Icon::GoTo,
        Place::Group,
    ),
    ("move", "Move all folder images", Icon::Move, Place::Bar),
    (
        "move_one",
        "Floating Move button (hold: mark images to move)",
        Icon::FloatingMove,
        Place::Bar,
    ),
    ("profile", "Profiles", Icon::Profile, Place::Group),
    ("folder", "Open folder", Icon::Folder, Place::Bar),
    ("settings", "Settings", Icon::Settings, Place::Bar),
    ("lock", "Lock", Icon::Lock, Place::Bar),
];
/// The buttons of the big image opened from the grid.
const BIG_BUTTONS: &[(&str, &str, Icon, Place)] = &[
    ("trash", "Move to Recycle Bin", Icon::Trash, Place::Bar),
    ("tags", "Tags", Icon::Tag, Place::Bar),
    ("fullscreen", "Fullscreen", Icon::Fullscreen, Place::Bar),
    ("prompt", "Prompt and seed", Icon::Prompt, Place::Bar),
    ("profile", "Profiles", Icon::Profile, Place::Group),
    (
        "image_only",
        "Fullscreen shows only the image",
        Icon::ImageOnly,
        Place::Group,
    ),
    ("lock", "Lock", Icon::Lock, Place::Bar),
];
/// The grid's controls (icon bar and text buttons). Some show only in some
/// cases (page arrows, page-size buttons, lock, open folder).
const GRID_LAYOUT: &[(&str, &str, Icon, Place, GridItem)] = &[
    (
        "prev_page",
        "Previous page",
        Icon::Previous,
        Place::Bar,
        GridItem::PrevPage,
    ),
    (
        "next_page",
        "Next page",
        Icon::Next,
        Place::Bar,
        GridItem::NextPage,
    ),
    (
        "normal",
        "Normal mode",
        Icon::Eye,
        Place::Bar,
        GridItem::Normal,
    ),
    (
        "hover_preview",
        "Hover preview",
        Icon::Preview,
        Place::Bar,
        GridItem::HoverPreview,
    ),
    ("sort", "Sort", Icon::Sort, Place::Bar, GridItem::Sort),
    (
        "pages",
        "Page order",
        Icon::Pages,
        Place::Bar,
        GridItem::Pages,
    ),
    ("trash", "Trash", Icon::Trash, Place::Bar, GridItem::Trash),
    (
        "undo",
        "Restore deleted",
        Icon::Undo,
        Place::Bar,
        GridItem::Undo,
    ),
    ("tags", "Tags", Icon::Tag, Place::Bar, GridItem::Tags),
    (
        "select_view",
        "Select all (view)",
        Icon::SelectPage,
        Place::Bar,
        GridItem::SelectView,
    ),
    (
        "select_folder",
        "Select all (folder)",
        Icon::SelectAll,
        Place::Bar,
        GridItem::SelectFolder,
    ),
    (
        "clear",
        "Clear selection",
        Icon::ClearSelection,
        Place::Bar,
        GridItem::Clear,
    ),
    (
        "restore",
        "Restore selection",
        Icon::Restore,
        Place::Bar,
        GridItem::Restore,
    ),
    (
        "close_grid",
        "Close grid",
        Icon::CloseGrid,
        Place::Bar,
        GridItem::CloseGrid,
    ),
    (
        "fullscreen",
        "Fullscreen",
        Icon::Fullscreen,
        Place::Bar,
        GridItem::Fullscreen,
    ),
    (
        "size",
        "Images per page (− +, add a size)",
        Icon::Plus,
        Place::Bar,
        GridItem::Size,
    ),
    (
        "presets",
        "Page-size buttons",
        Icon::AddPageSize,
        Place::Bar,
        GridItem::Presets,
    ),
    ("move", "Move", Icon::Move, Place::Bar, GridItem::Move),
    (
        "filter",
        "Filter",
        Icon::Filter,
        Place::Bar,
        GridItem::Filter,
    ),
    (
        "folder",
        "Open folder",
        Icon::Folder,
        Place::Bar,
        GridItem::Folder,
    ),
    (
        "floating_trash",
        "Floating trash button",
        Icon::FloatingTrash,
        Place::Bar,
        GridItem::FloatingTrash,
    ),
    ("lock", "Lock", Icon::Lock, Place::Bar, GridItem::Lock),
    (
        "goto",
        "Go to page 1 (hold: a number)",
        Icon::GoTo,
        Place::Group,
        GridItem::GoTo,
    ),
    (
        "profile",
        "Profiles",
        Icon::Profile,
        Place::Group,
        GridItem::Profile,
    ),
];
fn tool_defaults() -> Vec<(&'static str, Place)> {
    TOOL_BUTTONS.iter().map(|b| (b.0, b.3)).collect()
}
fn big_defaults() -> Vec<(&'static str, Place)> {
    BIG_BUTTONS.iter().map(|b| (b.0, b.3)).collect()
}
fn grid_defaults() -> Vec<(&'static str, Place)> {
    GRID_LAYOUT.iter().map(|b| (b.0, b.3)).collect()
}
/// The marked images' Move grid: its own controls besides the grid's.
const MOVE_GRID_EXTRA: &[(&str, &str, Icon, Place, GridItem)] = &[
    (
        "select",
        "Select the page (hold: all)",
        Icon::SelectPage,
        Place::Bar,
        GridItem::SelectBoth,
    ),
    (
        "destination",
        "Destination (hold: favorite folders)",
        Icon::Folder,
        Place::Bar,
        GridItem::Destination,
    ),
    (
        "keep_names",
        "Keep the original file names",
        Icon::KeepNames,
        Place::Bar,
        GridItem::KeepNames,
    ),
    (
        "move_marked",
        "Move",
        Icon::Move,
        Place::Bar,
        GridItem::MoveMarked,
    ),
    (
        "cancel_marked",
        "Cancel (back to the image)",
        Icon::CloseGrid,
        Place::Bar,
        GridItem::CancelMarked,
    ),
];
/// The Move grid's controls in their default order: the grid's that apply
/// and its own.
const MOVE_GRID_KEYS: &[&str] = &[
    "prev_page",
    "next_page",
    "normal",
    "hover_preview",
    "tags",
    "select",
    "clear",
    "restore",
    "trash",
    "fullscreen",
    "size",
    "presets",
    "destination",
    "keep_names",
    "move_marked",
    "cancel_marked",
];
fn move_grid_defaults() -> Vec<(&'static str, Place)> {
    MOVE_GRID_KEYS.iter().map(|k| (*k, Place::Bar)).collect()
}
/// A Move grid control by its key: its own, or the grid's.
fn move_grid_entry(
    key: &str,
) -> Option<&'static (&'static str, &'static str, Icon, Place, GridItem)> {
    MOVE_GRID_EXTRA
        .iter()
        .chain(GRID_LAYOUT.iter())
        .find(|b| b.0 == key)
}
/// The keys on the bar and in the group bar, in order (hidden ones left out).
fn split_places(items: &[(&'static str, Place)]) -> (Vec<&'static str>, Vec<&'static str>) {
    let pick = |place| {
        items
            .iter()
            .filter(|(_, p)| *p == place)
            .map(|(k, _)| *k)
            .collect()
    };
    (pick(Place::Bar), pick(Place::Group))
}
/// Settings → Display → Bar buttons: each bar's buttons in order, with
/// ⏶ ⏷ to move one and Bar / Group / Hidden for where it is.
fn bar_buttons_editor(
    ui: &mut egui::Ui,
    h: &mut Hover,
    s: &mut crate::model::Settings,
    tab: &mut &'static str,
) {
    h.section(ui, "Buttons on the bars");
    ui.horizontal_wrapped(|ui| {
        for (bar, label) in [
            ("toolbar", "Toolbar"),
            ("grid", "Grid controls"),
            ("big", "Big image from the grid"),
            ("move_grid", "Move grid (marked images)"),
        ] {
            if h.button(ui, label, *tab == bar, true, false) {
                *tab = bar;
            }
        }
    });
    let buttons: Vec<(&'static str, &'static str, Icon)> = match *tab {
        "grid" => GRID_LAYOUT.iter().map(|b| (b.0, b.1, b.2)).collect(),
        "big" => BIG_BUTTONS.iter().map(|b| (b.0, b.1, b.2)).collect(),
        "move_grid" => MOVE_GRID_KEYS
            .iter()
            .filter_map(|k| move_grid_entry(k))
            .map(|b| (b.0, b.1, b.2))
            .collect(),
        _ => TOOL_BUTTONS.iter().map(|b| (b.0, b.1, b.2)).collect(),
    };
    let defaults = match *tab {
        "grid" => grid_defaults(),
        "big" => big_defaults(),
        "move_grid" => move_grid_defaults(),
        _ => tool_defaults(),
    };
    let arranged =
        crate::bar_layout::arrange(&defaults, s.bar_buttons.get(*tab).map(Vec::as_slice));
    let mut change = None;
    egui::Grid::new(("bar_buttons", *tab))
        .num_columns(4)
        .spacing(Vec2::new(8., 4.))
        .show(ui, |ui| {
            for (i, (key, place)) in arranged.iter().enumerate() {
                ui.horizontal(|ui| {
                    if h.button(ui, "⏶", false, i > 0, false) {
                        change = Some((*key, None, true));
                    }
                    if h.button(ui, "⏷", false, i + 1 < arranged.len(), false) {
                        change = Some((*key, None, false));
                    }
                });
                let (icon, name) = buttons
                    .iter()
                    .find(|b| b.0 == *key)
                    .map(|b| (b.2, b.1))
                    .unwrap_or((Icon::More, *key));
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(24.), Sense::hover());
                crate::icons::paint(
                    ui.painter(),
                    rect.center(),
                    icon,
                    if *place == Place::Hidden {
                        MUTED
                    } else {
                        Color32::from_rgb(ICON_COLOR[0], ICON_COLOR[1], ICON_COLOR[2])
                    },
                );
                ui.label(name);
                ui.horizontal(|ui| {
                    for p in Place::ALL {
                        if h.button(ui, p.label(), *place == p, true, false) && *place != p {
                            change = Some((*key, Some(p), false));
                        }
                    }
                });
                ui.end_row();
            }
        });
    if let Some((key, place, up)) = change {
        let saved = s.bar_buttons.get(*tab).map(Vec::as_slice);
        let slots = match place {
            Some(p) => crate::bar_layout::placed(&defaults, saved, key, p),
            None => crate::bar_layout::moved(&defaults, saved, key, up),
        };
        s.bar_buttons.insert((*tab).to_owned(), slots);
    }
    if h.button(
        ui,
        "Reset this bar's buttons",
        false,
        s.bar_buttons.contains_key(*tab),
        false,
    ) {
        s.bar_buttons.remove(*tab);
    }
    ui.label(RichText::new("Group: behind the » button at the end of the bar, which opens a second bar above it. Grid controls that do not fit go there too. Some grid controls show only in some cases (page arrows, page-size buttons, lock, open folder and floating trash when the toolbar is hidden).").color(MUTED));
}
/// Keys, names and icons of the grid icon bar's buttons (colour settings).
const GRID_BUTTONS: &[(&str, &str, Icon)] = &[
    ("prev_page", "Previous page", Icon::Previous),
    ("next_page", "Next page", Icon::Next),
    ("normal", "Normal mode", Icon::Eye),
    ("hover_preview", "Hover preview", Icon::Preview),
    ("sort", "Sort", Icon::Sort),
    ("pages", "Page order", Icon::Pages),
    ("trash", "Trash", Icon::Trash),
    ("undo", "Restore deleted", Icon::Undo),
    ("tags", "Tags", Icon::Tag),
    ("select_view", "Select all (view)", Icon::SelectPage),
    ("select_folder", "Select all (folder)", Icon::SelectAll),
    ("clear", "Clear selection", Icon::ClearSelection),
    ("restore", "Restore selection", Icon::Restore),
    ("close_grid", "Close grid", Icon::CloseGrid),
    ("fullscreen", "Fullscreen", Icon::Fullscreen),
    ("fewer", "Fewer images per page", Icon::Minus),
    ("more", "More images per page", Icon::Plus),
    ("add_size", "Add a page-size button", Icon::AddPageSize),
    ("filter", "Filter", Icon::Filter),
    ("folder", "Open folder", Icon::Folder),
    (
        "floating_trash",
        "Floating trash button",
        Icon::FloatingTrash,
    ),
    ("move", "Move all folder images", Icon::Move),
    ("lock", "Lock", Icon::Lock),
];
/// egui's dark button box (the text buttons' default box colour).
const TEXT_BUTTON_FILL: [u8; 4] = [60, 60, 60, 255];
/// The icon colour of toolbar buttons when no colour is set.
const ICON_COLOR: [u8; 4] = [194, 206, 216, 255];
/// A vertical divider of a fixed height (a separator would take all the
/// height it is given, e.g. in a wrapping overflow bar).
fn vline(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9., 22.), Sense::hover());
    let x = rect.center().x;
    ui.painter().line_segment(
        [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}
/// Height of a row of the grid's controls: the icons' 38 px (the text
/// controls have the page arrows and the bookmark as icons too).
const CONTROLS_ROW: f32 = 38.;
/// Height of a text button.
fn button_height(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Button) + 2. * ui.spacing().button_padding.y
}
/// A small floating box centred above `anchor` (the top centre of the
/// control that opened it), kept on screen. Returns its rect.
fn above(ctx: &egui::Context, id: Id, anchor: Pos2, contents: impl FnOnce(&mut egui::Ui)) -> Rect {
    let response = egui::Area::new(id)
        .pivot(egui::Align2::CENTER_BOTTOM)
        .fixed_pos(anchor - Vec2::new(0., 6.))
        .constrain(true)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .corner_radius(12.)
                .inner_margin(10.)
                .show(ui, contents);
        })
        .response;
    ctx.move_to_top(response.layer_id);
    response.rect
}
/// A popup above `anchor`, or below `below` when last frame's popup would
/// not fit above it on the screen.
fn near(
    ctx: &egui::Context,
    id: Id,
    anchor: Pos2,
    below: Pos2,
    contents: impl FnOnce(&mut egui::Ui),
) -> Rect {
    let height = ctx.memory(|m| m.area_rect(id)).map_or(0., |r| r.height());
    let fits_above = anchor.y - 6. - height >= ctx.screen_rect().top();
    let (pivot, pos) = if fits_above {
        (egui::Align2::CENTER_BOTTOM, anchor - Vec2::new(0., 6.))
    } else {
        (egui::Align2::CENTER_TOP, below + Vec2::new(0., 6.))
    };
    let response = egui::Area::new(id)
        .pivot(pivot)
        .fixed_pos(pos)
        .constrain(true)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .corner_radius(12.)
                .inner_margin(10.)
                .show(ui, contents);
        })
        .response;
    ctx.move_to_top(response.layer_id);
    response.rect
}
/// File names in the Recycle Bin dialog: up to four; with more, three and
/// "…" in the fourth line.
fn delete_dialog_lines(paths: &[PathBuf]) -> Vec<String> {
    const LINES: usize = 4;
    let name = |p: &PathBuf| {
        p.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    if paths.len() <= LINES {
        return paths.iter().map(name).collect();
    }
    let mut lines: Vec<_> = paths.iter().take(LINES - 1).map(name).collect();
    lines.push("…".into());
    lines
}
/// Largest share of the window one activation strip may cover.
const EDGE_MAX_SHARE: f32 = 0.45;
/// Gap between the trash circle and its close button.
const CLOSE_GAP: f32 = 3.;
/// Room around the trash circle for the close button outside it.
fn close_margin(close_size: f32) -> f32 {
    close_size + CLOSE_GAP + 2.
}
/// Centre of the close button: fully outside the trash circle, at `angle`
/// degrees (0 = right, counter-clockwise).
fn close_center(center: Pos2, radius: f32, close_size: f32, angle: f32) -> Pos2 {
    let a = angle.to_radians();
    center + Vec2::new(a.cos(), -a.sin()) * (radius + close_size / 2. + CLOSE_GAP)
}
fn tag_label(name: &str) -> std::borrow::Cow<'_, str> {
    if name.contains(['\n', '\r', '\t']) {
        name.replace(['\n', '\r', '\t'], " ").into()
    } else {
        name.into()
    }
}
fn tag_picker_ui(
    ui: &mut egui::Ui,
    hover: &mut Hover,
    picker: &mut crate::tags::TagPicker,
    library: &Library,
    selected: &mut BTreeSet<String>,
    view: TagView,
) -> Vec<TagAction> {
    let TagView {
        width,
        max_height,
        controls,
        enabled,
    } = view;
    if picker.refresh(library) {
        ui.ctx().request_repaint();
    }
    if picker.len() == 0 && controls.is_none() {
        return vec![];
    }
    let font = egui::TextStyle::Button.resolve(ui.style());
    let padding = ui.spacing().button_padding;
    let row_height = (ui.text_style_height(&egui::TextStyle::Button) + padding.y * 2.)
        .ceil()
        .max(ui.spacing().interact_size.y);
    let style = crate::tags::RowStyle {
        font: font.clone(),
        padding,
        minimum: Vec2::new(0., row_height),
        gap: ui.spacing().item_spacing.x,
        gap_y: ui.spacing().item_spacing.y,
        height: max_height,
        scrollbar: ui.spacing().scroll.bar_width
            + ui.spacing().scroll.bar_inner_margin
            + ui.spacing().scroll.bar_outer_margin,
        pixels_per_point: ui.ctx().pixels_per_point(),
        controls: controls.is_some(),
    };
    if picker.arrange(width, style, |name| {
        ui.painter()
            .layout_no_wrap(tag_label(name).into_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
    }) {
        ui.ctx().request_repaint();
    }
    let control_count = if controls.is_some() {
        crate::tags::CONTROLS.len()
    } else {
        0
    };
    let fixed = picker.rows.fixed;
    let row_count = picker.rows.ranges.len();
    let fixed_height = fixed as f32 * (row_height + ui.spacing().item_spacing.y);
    let used_width = picker.rows.used_width.max(1.);
    let mut actions = vec![];
    let mut draw_row = |ui: &mut egui::Ui, row: usize| {
        ui.horizontal(|ui| {
            for item in picker.rows.ranges[row].clone() {
                let item_width = picker.rows.widths[item].min(picker.rows.wrap_width);
                if item < control_count {
                    let c = controls.as_ref().unwrap();
                    let (active, allow, danger) = match item {
                        0 => (false, c.enabled, false),
                        1 => (false, c.enabled && c.favorites, false),
                        2 => (c.additional, true, false),
                        3 => (c.locked, true, false),
                        4 => (false, c.enabled && !selected.is_empty(), true),
                        _ => (false, c.enabled && !selected.is_empty(), false),
                    };
                    if item == 1 {
                        // Top centre of the Favorite button: its chooser opens above.
                        let top = ui.cursor().min + Vec2::new(item_width / 2., 0.);
                        ui.ctx()
                            .data_mut(|d| d.insert_temp(Id::new("tag_favorite_top"), top));
                    }
                    if hover.button(ui, crate::tags::CONTROLS[item], active, allow, danger) {
                        actions.push(
                            [
                                TagAction::Create,
                                TagAction::Favorites,
                                TagAction::Additional,
                                TagAction::Lock,
                                TagAction::Remove,
                                TagAction::Save,
                            ][item],
                        );
                    }
                } else {
                    let choice = &picker.choices()[item - control_count];
                    ui.push_id(&choice.name, |ui| {
                        let mut button = egui::Button::new(tag_label(&choice.name).as_ref())
                            .truncate()
                            .selected(selected.contains(&choice.name));
                        if choice.additional {
                            button = button.stroke(Stroke::new(1.5_f32, TAG_YELLOW));
                        }
                        let response = ui
                            .add_enabled_ui(enabled, |ui| {
                                ui.add_sized([item_width, row_height], button)
                            })
                            .inner;
                        let clicked = hover.watch(response.clone(), false).clicked();
                        response.on_hover_text(format!(
                            "{}\n{}",
                            choice.name,
                            if choice.additional {
                                "Additional keyword"
                            } else {
                                "My tag"
                            }
                        ));
                        if clicked {
                            if !selected.insert(choice.name.clone()) {
                                selected.remove(&choice.name);
                            }
                            ui.ctx().request_repaint();
                        }
                    });
                }
            }
        });
    };
    if row_count > fixed {
        egui::ScrollArea::vertical()
            .id_salt("tag_rows")
            .auto_shrink([true, true])
            .stick_to_bottom(true)
            .max_height((max_height - fixed_height).max(row_height))
            .show_rows(ui, row_height, row_count - fixed, |ui, visible| {
                ui.set_max_width(used_width);
                for visual in visible {
                    draw_row(ui, row_count - 1 - visual);
                }
            });
    }
    for row in (0..fixed).rev() {
        draw_row(ui, row);
    }
    actions
}

pub struct Viewer {
    lib: Library,
    startup: Option<Startup>,
    retire: Retire,
    pending_open: Option<(PathBuf, bool)>,
    view_build: Option<ViewBuild>,
    view_dirty: bool,
    view_reset: bool,
    view_pending: bool,
    view_fallback: Option<usize>,
    store: Option<Store>,
    persist_allowed: bool,
    writer: Option<StateWriter>,
    restoring: bool,
    dirty: bool,
    last_save: Instant,
    /// When the image on screen last changed (saves wait while browsing).
    image_changed: Instant,
    image_seen: Option<PathBuf>,
    files: Vec<PathBuf>,
    visible: VisibleFiles,
    current: Option<PathBuf>,
    folder: Option<PathBuf>,
    nav: Navigator,
    images: Images,
    music: Music,
    tx: EventSender,
    rx: Receiver<Event>,
    folders: FolderLoader,
    folder_paths: HashSet<PathBuf>,
    index_progress: Option<Progress>,
    last_index: Option<Progress>,
    loading: bool,
    busy: bool,
    backup_busy: bool,
    bar: bool,
    panel: Panel,
    settings: bool,
    info: bool,
    help: bool,
    fullscreen: bool,
    /// The last viewed images, newest first (right-click menu window).
    recent: std::collections::VecDeque<PathBuf>,
    recent_open: bool,
    /// The pointer position of the last frame it was inside the window.
    last_pointer: Option<Pos2>,
    /// Folder watcher: the owner lock, the engine inside this program and
    /// what was last applied to the background process.
    watch: WatchState,
    /// The low-latency choice last written for the next start.
    display_written: Option<(bool, String, bool)>,
    /// The optional USN service, as last checked (Settings → Library).
    usn: UsnView,
    /// Generation parameters of the image whose menu was last opened.
    generation: Option<(PathBuf, Option<crate::comfy::Generation>)>,
    tag_locked: bool,
    selected_tags: BTreeSet<String>,
    tag_picker: crate::tags::TagPicker,
    filter_picker: crate::tags::TagPicker,
    new_tag: String,
    new_tag_open: bool,
    new_tag_focus: bool,
    /// Favorite / Elite chooser opened from the tag palette: above the
    /// palette's Favorite button (its top centre).
    tag_favorite_popup: Option<Pos2>,
    /// Clear selection held down: when, and whether the long-press choices
    /// opened (then the release is not a click).
    clear_hold: Option<(Instant, bool)>,
    /// Long-press choices of Clear selection, above the button's rect.
    clear_menu: Option<Rect>,
    /// An open Sort / Page order choice of the grid icon bar.
    grid_menu: Option<(GridMenu, Rect)>,
    /// Where the open panels sit (distance from the bottom), kept while the
    /// bars below them hide.
    panel_bottom: Option<f32>,
    /// The toolbar's Order choice, above its icon.
    order_menu: Option<Rect>,
    /// The Prompt and seed window, opened for this image.
    prompt_window: Option<PathBuf>,
    /// The window follows the shown image (opened from the big image's
    /// bar) rather than staying with the image it was opened for.
    prompt_follows: bool,
    /// Move selected mode in the grid: clicks choose images to move.
    move_mode: bool,
    move_selection: HashSet<PathBuf>,
    move_hold: Option<(Instant, bool)>,
    /// Widths of the grid's controls (icon style, item) from the last frame:
    /// what fits on the line is known before drawing it.
    grid_item_widths: HashMap<(bool, GridItem), f32>,
    /// The » bar of the grid's controls that did not fit is open.
    grid_overflow: bool,
    /// The bar whose group bar (») is open: "toolbar" or "big".
    group_open: Option<&'static str>,
    /// Settings → Display → Bar buttons: the bar being arranged.
    bar_buttons_tab: &'static str,
    /// Settings → Library → Favorite folders: the entry whose folder is
    /// being chosen again (None: a new one).
    favorite_pick: Option<usize>,
    /// NoirView's window, the owner of file and folder dialogs.
    parent_window: Option<ParentWindow>,
    /// A file or folder dialog is open (only one at a time).
    picking: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Buttons being held (start, the long press already fired).
    holds: HashMap<&'static str, (Instant, bool)>,
    /// A small window above a held button: favorite folders or Go to.
    popup: Option<(Popup, Rect)>,
    /// The number typed in Go to, and whether it still needs the focus.
    goto_text: String,
    goto_focus: bool,
    /// Images shown in the image view in this round (round notice), the
    /// last one counted and the order it counts for.
    round_seen: HashSet<PathBuf>,
    round_last: Option<PathBuf>,
    round_order: Option<crate::model::Order>,
    /// The round notice on screen (text, since).
    round_message: Option<(String, Instant, u64)>,
    /// Images shown in the image view so far (the round notice can go after
    /// a number of new ones).
    images_shown: u64,
    /// When the round notice began to fade out.
    round_fade_out: Option<Instant>,
    /// Settings: scroll to this section when the window next draws.
    settings_scroll: Option<&'static str>,
    /// Where the pointer was when the hover preview opened.
    preview_origin: Option<(PathBuf, Pos2)>,
    /// The bar of the grid's controls drawn last (with its frame or margin):
    /// their menus and overflow bar open above it.
    controls_rect: Option<Rect>,
    /// Centre of the grid's Tags button (the tag palette is centred on it).
    tags_anchor: Option<f32>,
    /// A copy from a grid tile's menu: the tile flashes briefly.
    copy_flash: Option<(PathBuf, Instant)>,
    /// The window's size last frame: after a change one more frame is drawn.
    last_screen: Vec2,
    /// The window sizes last written to data/noirview-window.log.
    window_logged: String,
    /// When the running move to the Recycle Bin started (Unix seconds).
    deleting_since: i64,
    /// The image last drawn in the single view, shown while the next one
    /// loads (Settings → Display → Keep the previous image…).
    previous_image: Option<(PathBuf, egui::TextureHandle, [u32; 2])>,
    /// The chosen graphics backend was checked after the first frames.
    backend_checked: bool,
    /// The settings as they were when the Settings window opened (Cancel).
    settings_snapshot: Option<Box<Settings>>,
    /// Where the button popup was drawn last frame.
    popup_rect: Option<Rect>,
    /// Settings show the Move grid's grid settings (swapped in while the
    /// Move grid is not open).
    settings_move_target: bool,
    /// Settings' search words, and the search and page last scrolled to.
    settings_query: String,
    settings_scrolled: Option<(String, usize)>,
    /// The search last seen (a new one may change the page), and its
    /// matches per page.
    settings_last_query: String,
    settings_counts: Vec<usize>,
    /// A profile to switch to (Settings or the right-click menu).
    pending_profile: Option<String>,
    /// Naming a profile: the one renamed (none: a new one) and the text.
    profile_edit: Option<(Option<String>, String)>,
    /// Screen x of the toolbar ♥ icon and the tag-row ♥ button centres.
    favorite_anchor: Option<f32>,
    identity_snapshot: Option<std::sync::Arc<crate::identity::Snapshot>>,
    identity_pending: bool,
    identity_inflight: bool,
    identity_token: u64,
    filter: Filter,
    /// The other view's filter while filters are kept per view (grid / image
    /// view); swapped with `filter` when the view changes.
    other_filter: Filter,
    draft_filter: Filter,
    /// Entering the grid: open the page of this image once the view is ready.
    grid_jump_to: Option<PathBuf>,
    grid: bool,
    tag_mode: bool,
    delete_selection: HashSet<PathBuf>,
    tag_selection: HashSet<PathBuf>,
    grid_start: usize,
    /// Tile area of the last drawn grid, used by the page-size rounding.
    grid_area: Option<crate::grid_layout::Area>,
    /// What the background thumbnail pass was last started for.
    pregen_key: Option<PregenKey>,
    /// Cache folder setting this session's thumbnail cache was opened with.
    cache_dir_in_use: PathBuf,
    /// Grid Normal mode: a click opens the image instead of selecting it.
    grid_normal: bool,
    /// Tag mode was on when Normal mode started with the selection kept: it
    /// comes back (palette and tag selection) when Selection mode returns.
    tag_mode_paused: bool,
    /// Image shown from the grid in Normal mode (no browsing while open).
    grid_viewer: Option<PathBuf>,
    viewer_press: Option<(Instant, Pos2)>,
    grid_ghosts: Option<GridGhosts>,
    /// Right-button gesture in progress.
    gesture_track: Option<crate::gesture::Track>,
    /// Tiles drawn this frame (for gestures started over a tile).
    grid_tiles: Vec<(Rect, PathBuf)>,
    /// Selection feedback: the large centred toast.
    big_toast: Option<(String, Instant)>,
    /// Hover preview: candidate tile with its rest start, open preview, and the
    /// time the pointer entered the top closing strip.
    hover_candidate: Option<(PathBuf, Instant)>,
    preview: Option<PathBuf>,
    preview_close: Option<Instant>,
    sort_request: u64,
    /// Selection removed by Normal mode, Clear or Select all: (trash, tags).
    last_selection: Option<(HashSet<PathBuf>, HashSet<PathBuf>)>,
    /// Selection button that flashes as feedback.
    flash_button: Option<(&'static str, Instant)>,
    /// Random / Shuffle grid pages.
    grid_pages: crate::grid_pages::Pages,
    /// Memory pressure from this process, the system's free RAM and the
    /// graphics card (`memory::Guard`), checked once a second.
    memory_guard: crate::memory::Guard,
    memory_probe: crate::memory::Probe,
    pub machine: crate::memory::Machine,
    memory_checked: Option<Instant>,
    /// Animated GIF frames for the big image.
    gif_player: crate::animation::Player,
    health_checked: Instant,
    health_logged: Instant,
    /// (page contents, started, finished: shown after / prepared in) of the
    /// page load timer.
    grid_timer: Option<(Vec<PathBuf>, Instant, Option<PageTimes>)>,
    /// How the image in the viewer arrived: first view, then full detail.
    view_timer: Option<ViewTimer>,
    slideshow: bool,
    paused: bool,
    slide_suspended: bool,
    last_slide: Instant,
    delete_float: bool,
    /// A favorite folder being opened: where browsing left it, and whether
    /// one of its images was opened directly (then that image is shown).
    restore_place: Option<(FolderPlace, bool)>,
    /// The image whose place was saved last.
    place_recorded: Option<PathBuf>,
    /// The floating Move button (single image view) is shown; marking
    /// images to move with it, and the images marked so far.
    move_float: bool,
    marking: bool,
    marked: Vec<PathBuf>,
    /// The marked images' Move grid is open: what it replaced, and its
    /// scroll offset with all images on one page.
    move_grid: Option<MoveGridSaved>,
    move_grid_scroll: f32,
    /// The grid's columns and row height last drawn (the Move grid's one
    /// page steps by them).
    grid_columns: usize,
    grid_row_height: f32,
    /// Where the image was drawn last frame (the mark goes at its corner).
    image_rect: Option<Rect>,
    dialog: Option<Dialog>,
    keyword_editor: Option<(PathBuf, String)>,
    keyword_loading: Option<PathBuf>,
    keyword_request: u64,
    toast: Option<(String, Instant)>,
    hover: Hover,
    bar_dwell: Dwell,
    /// The bottom strip toggled the bar and has not been re-armed: in a
    /// window the pointer slips out through the bottom and back in, which
    /// must not toggle it again. Re-armed once the pointer is seen above it.
    bar_edge_spent: bool,
    left_dwell: Dwell,
    right_dwell: Dwell,
    top_dwell: Dwell,
    wheel: WheelNavigation,
    magnifier: Magnifier,
    settings_tab: usize,
    delete_area_revision: u64,
    next_backup_attempt: Instant,
    closing: bool,
    close_allowed: bool,
    initial_cli: Option<PathBuf>,
    gpu: String,
}
impl Viewer {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(8., 8.);
        style.spacing.button_padding = Vec2::new(12., 8.);
        style.visuals.panel_fill = Color32::from_rgb(12, 15, 20);
        style.visuals.window_fill = Color32::from_rgb(23, 28, 35);
        style.visuals.extreme_bg_color = Color32::from_rgb(14, 18, 24);
        style.visuals.selection.bg_fill = Color32::from_rgb(36, 83, 66);
        style.visuals.selection.stroke = Stroke::new(1.0_f32, GREEN);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(41, 83, 68);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 57, 68);
        cc.egui_ctx.set_style(style);
        let startup = Startup::new({
            let ctx = cc.egui_ctx.clone();
            move || ctx.request_repaint()
        });
        let initial_cli = std::env::args_os().nth(1).map(PathBuf::from);
        let gpu = cc
            .wgpu_render_state
            .as_ref()
            .map(|r| {
                let i = r.adapter.get_info();
                format!("{} · {:?}", i.name, i.backend)
            })
            .or_else(|| {
                cc.gl.as_ref().map(|gl| {
                    use eframe::glow::HasContext as _;
                    // SAFETY: plain string queries on the current context.
                    let (renderer, version) = unsafe {
                        (
                            gl.get_parameter_string(eframe::glow::RENDERER),
                            gl.get_parameter_string(eframe::glow::VERSION),
                        )
                    };
                    format!("{renderer} · OpenGL {version}")
                })
            })
            .unwrap_or_else(|| "GPU unavailable".into());
        Self::bootstrap(cc.egui_ctx.clone(), gpu, initial_cli, startup)
    }
    fn bootstrap(
        ctx: egui::Context,
        gpu: String,
        initial_cli: Option<PathBuf>,
        startup: Startup,
    ) -> Self {
        let lib = Library::default();
        let (tx, rx) = unbounded();
        let tx = EventSender {
            tx,
            ctx: ctx.clone(),
        };
        let images = Images::new(ctx.clone(), None, 0);
        let retire = Retire::new({
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        });
        Self {
            lib,
            startup: Some(startup),
            retire,
            pending_open: None,
            view_build: None,
            view_dirty: false,
            view_reset: false,
            view_pending: false,
            view_fallback: None,
            store: None,
            persist_allowed: false,
            writer: None,
            restoring: false,
            dirty: false,
            last_save: Instant::now(),
            image_changed: Instant::now(),
            image_seen: None,
            files: vec![],
            visible: VisibleFiles::default(),
            current: None,
            folder: None,
            nav: Navigator::default(),
            images,
            music: Music::default(),
            tx,
            rx,
            folders: FolderLoader::new({
                let ctx = ctx.clone();
                move || ctx.request_repaint()
            }),
            folder_paths: HashSet::new(),
            index_progress: None,
            last_index: None,
            loading: false,
            busy: false,
            backup_busy: false,
            bar: false,
            panel: Panel::None,
            settings: false,
            info: false,
            help: false,
            fullscreen: false,
            recent: Default::default(),
            recent_open: false,
            last_pointer: None,
            watch: WatchState::default(),
            display_written: None,
            usn: UsnView::default(),
            generation: None,
            tag_locked: false,
            selected_tags: BTreeSet::new(),
            tag_picker: crate::tags::TagPicker::default(),
            filter_picker: crate::tags::TagPicker::all(),
            new_tag: String::new(),
            new_tag_open: false,
            new_tag_focus: false,
            tag_favorite_popup: None,
            clear_hold: None,
            clear_menu: None,
            grid_menu: None,
            panel_bottom: None,
            order_menu: None,
            prompt_window: None,
            prompt_follows: false,
            move_mode: false,
            move_selection: HashSet::new(),
            move_hold: None,
            grid_item_widths: HashMap::new(),
            grid_overflow: false,
            group_open: None,
            bar_buttons_tab: "toolbar",
            holds: HashMap::new(),
            picking: std::sync::Arc::default(),
            parent_window: None,
            favorite_pick: None,
            popup: None,
            goto_text: String::new(),
            goto_focus: false,
            round_seen: HashSet::new(),
            round_last: None,
            round_order: None,
            preview_origin: None,
            round_message: None,
            images_shown: 0,
            round_fade_out: None,
            settings_scroll: None,
            controls_rect: None,
            tags_anchor: None,
            copy_flash: None,
            last_screen: Vec2::ZERO,
            window_logged: String::new(),
            deleting_since: 0,
            previous_image: None,
            backend_checked: false,
            settings_snapshot: None,
            popup_rect: None,
            settings_move_target: false,
            settings_query: String::new(),
            settings_scrolled: None,
            settings_last_query: String::new(),
            settings_counts: Vec::new(),
            pending_profile: None,
            profile_edit: None,
            favorite_anchor: None,
            identity_snapshot: None,
            identity_pending: false,
            identity_inflight: false,
            identity_token: 0,
            filter: Filter::default(),
            other_filter: Filter::default(),
            draft_filter: Filter::default(),
            grid_jump_to: None,
            grid: false,
            tag_mode: false,
            delete_selection: HashSet::new(),
            tag_selection: HashSet::new(),
            grid_start: 0,
            grid_area: None,
            pregen_key: None,
            cache_dir_in_use: PathBuf::new(),
            grid_normal: false,
            tag_mode_paused: false,
            grid_viewer: None,
            viewer_press: None,
            grid_ghosts: None,
            gesture_track: None,
            grid_tiles: Vec::new(),
            big_toast: None,
            hover_candidate: None,
            preview: None,
            preview_close: None,
            sort_request: 0,
            last_selection: None,
            flash_button: None,
            grid_pages: Default::default(),
            memory_guard: Default::default(),
            memory_probe: crate::memory::Probe::new(),
            machine: Default::default(),
            memory_checked: None,
            gif_player: crate::animation::Player::default(),
            health_checked: Instant::now(),
            health_logged: Instant::now(),
            grid_timer: None,
            view_timer: None,
            slideshow: false,
            paused: false,
            slide_suspended: false,
            last_slide: Instant::now(),
            delete_float: false,
            restore_place: None,
            place_recorded: None,
            move_float: false,
            marking: false,
            marked: Vec::new(),
            move_grid: None,
            move_grid_scroll: 0.,
            grid_columns: 1,
            grid_row_height: 0.,
            image_rect: None,
            dialog: None,
            keyword_editor: None,
            keyword_loading: None,
            keyword_request: 0,
            toast: None,
            hover: Hover::default(),
            bar_dwell: Dwell::default(),
            bar_edge_spent: false,
            left_dwell: Dwell::default(),
            right_dwell: Dwell::default(),
            top_dwell: Dwell::default(),
            wheel: WheelNavigation::default(),
            magnifier: Magnifier::default(),
            settings_tab: 0,
            delete_area_revision: 0,
            next_backup_attempt: Instant::now(),
            closing: false,
            close_allowed: false,
            initial_cli,
            gpu,
        }
    }
    fn install_library(&mut self, loaded: Loaded, ctx: &egui::Context) {
        self.store = loaded.store;
        match loaded.library {
            Ok(lib) => {
                self.lib = lib;
                self.lib.settings.initialize_after_load();
                // Closed with the Move grid open: the grid's settings back.
                self.lib.settings.leave_move_grid_settings();
                self.tag_picker = crate::tags::TagPicker::default();
                self.filter_picker = crate::tags::TagPicker::all();
                self.persist_allowed = true;
            }
            Err(error) => {
                self.persist_allowed = false;
                self.dialog = Some(Dialog::Error(error));
            }
        }
        if std::env::var_os("NOIRVIEW_FRAME_TRACE").is_some() {
            eprintln!(
                "noirview library ready: persistence={}, photos={}",
                self.persist_allowed,
                self.lib.photos.len()
            );
        }
        if self.lib.settings.ort_library.as_os_str().is_empty() {
            if let Some(store) = &self.store {
                // Existence is checked only if AI actually loads the library.
                self.lib.settings.ort_library = store.exe_dir.join(if cfg!(windows) {
                    "onnxruntime.dll"
                } else {
                    "libonnxruntime.so"
                });
            }
        }
        self.grid = self.lib.last_grid;
        self.grid_normal = self.initial_grid_normal();
        self.writer = self.store.clone().map(|store| {
            let tx = self.tx.clone();
            StateWriter::new(store, move |notice| {
                let _ = tx.send(Event::State(notice));
            })
        });
        self.cache_dir_in_use = self.lib.settings.thumbnail_cache_dir.clone();
        self.images = Images::new(
            ctx.clone(),
            thumbnail_cache_root(
                &self.lib.settings.thumbnail_cache_dir,
                self.store.as_ref().and_then(|s| s.path.parent()),
            ),
            self.lib.settings.thumbnail_cache_mb,
        );
        let initial = self
            .initial_cli
            .take()
            .or_else(|| self.lib.last_image.clone().filter(|p| p.exists()))
            .or_else(|| self.lib.last_folder.clone());
        if let Some(path) = initial {
            self.open(path);
        }
    }
    fn startup_frame(&mut self, ctx: &egui::Context) -> bool {
        let Some(startup) = &self.startup else {
            return false;
        };
        // Before installing state, close needs no save, backup or writer join.
        if ctx.input(|i| i.viewport().close_requested()) {
            self.startup = None;
            return true;
        }
        if let Some(path) = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .rev()
                .find_map(|f| f.path.clone())
        }) {
            self.initial_cli = Some(path);
            ctx.input_mut(|i| i.raw.dropped_files.clear());
        }
        if let Some(loaded) = startup.poll() {
            self.startup = None;
            self.install_library(loaded, ctx);
            return false;
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.35).max(20.));
                ui.label(RichText::new("N O I R V I E W").color(GREEN));
                ui.add_space(18.);
                ui.spinner();
                ui.label("Loading library…");
            });
        });
        true
    }
    fn notify(&mut self, message: impl Into<String>) {
        self.toast = Some((message.into(), Instant::now()));
        self.tx.ctx.request_repaint();
    }
    fn changed(&mut self) {
        let snapshot = self
            .identity_snapshot
            .as_deref()
            .filter(|s| s.serial == self.folders.identity_serial());
        self.lib.sync_marks(snapshot);
        self.dirty = true;
        self.tx.ctx.request_repaint();
    }
    fn pick(&self, kind: &str) {
        // One file or folder dialog at a time.
        if self.picking.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        let picking = self.picking.clone();
        let tx = self.tx.clone();
        let kind = kind.to_string();
        let place = self.lib.settings.dialog_places.get(&kind).copied();
        // Owned by NoirView's window: a click on the window cannot hide it.
        // Each kind keeps its own place, size and last folder (Windows).
        let mut dialog = rfd::FileDialog::new().set_client_guid(dialog_guid(&kind));
        if let Some(parent) = &self.parent_window {
            dialog = dialog.set_parent(parent);
        }
        std::thread::spawn(move || {
            let _done = PickDone(picking);
            // Opened where it was last left; its place is kept for next time.
            let (event, place) = crate::dialog_place::track(place, || match kind.as_str() {
                "open" => dialog
                    .set_title("Open image folder")
                    .pick_folder()
                    .map(Event::Open),
                "file" => dialog
                    .add_filter(
                        "Images",
                        &[
                            "png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "ico",
                        ],
                    )
                    .pick_file()
                    .map(Event::Open),
                "destination" => dialog
                    .set_title("Move destination")
                    .pick_folder()
                    .map(Event::Destination),
                "tracks" => dialog
                    .add_filter("Audio", &["mp3", "wav", "flac", "ogg"])
                    .pick_files()
                    .map(Event::Tracks),
                "model" => dialog
                    .add_filter("ONNX model", &["onnx"])
                    .pick_file()
                    .map(Event::Model),
                "runtime" => dialog
                    .add_filter("ONNX Runtime", &["dll", "so", "dylib"])
                    .pick_file()
                    .map(Event::Runtime),
                "backup" => dialog.pick_folder().map(Event::ExtraBackup),
                "cache_dir" => dialog
                    .set_title("Thumbnail cache folder")
                    .pick_folder()
                    .map(Event::ThumbnailCacheDir),
                "favorite" => dialog
                    .set_title("Favorite folder")
                    .pick_folder()
                    .map(Event::FavoriteFolder),
                "watch" => dialog
                    .set_title("Folder to watch")
                    .pick_folder()
                    .map(Event::WatchFolder),
                "restore" => dialog
                    .add_filter("NoirView backup", &["json"])
                    .pick_file()
                    .map(Event::RestoreChoice),
                _ => None,
            });
            if let Some(place) = place {
                let _ = tx.send(Event::DialogPlace(kind, place));
            }
            if let Some(e) = event {
                let _ = tx.send(e);
            }
        });
    }
    fn open(&mut self, path: PathBuf) {
        self.open_indexed(path, false);
    }
    fn open_indexed(&mut self, path: PathBuf, force: bool) {
        if self.busy {
            return;
        }
        self.folders.cancel();
        self.pending_open = Some((path, force));
        self.view_pending = true;
        self.wheel.reset();
        self.tx.ctx.request_repaint();
    }
    fn begin_open(&mut self) {
        if self.pending_open.is_none() || !self.retire.available() {
            return;
        }
        let (path, force) = self.pending_open.take().unwrap();
        self.retire.retire((
            std::mem::take(&mut self.files),
            std::mem::take(&mut self.folder_paths),
            std::mem::take(&mut self.visible),
            std::mem::take(&mut self.nav),
            self.view_build.take(),
        ));
        self.view_dirty = false;
        self.view_reset = false;
        self.view_fallback = None;
        let path = if path.is_relative() {
            std::env::current_dir()
                .map(|root| root.join(&path))
                .unwrap_or(path)
        } else {
            path
        };
        let preferred = if path.is_file() {
            Some(path.clone())
        } else {
            None
        };
        let folder = if path.is_file() {
            path.parent().map(Path::to_path_buf).unwrap_or(path)
        } else {
            path
        };
        self.loading = true;
        self.restore_place = self
            .favorite_place(&folder)
            .cloned()
            .map(|place| (place, preferred.is_some()));
        self.place_recorded = None;
        self.identity_token = self.identity_token.wrapping_add(1);
        self.identity_snapshot = None;
        self.identity_pending = false;
        self.wheel.reset();
        self.current = preferred;
        self.folder = Some(folder.clone());
        self.lib.last_folder = Some(folder.clone());
        self.lib.last_image = self.current.clone();
        self.filter = Filter::default();
        self.other_filter = Filter::default();
        self.delete_selection.clear();
        self.tag_selection.clear();
        // Images chosen for moving belong to the folder they were chosen in.
        self.move_selection.clear();
        self.move_mode = false;
        if self.move_grid.take().is_some() {
            self.grid = false;
            self.lib.settings.leave_move_grid_settings();
        }
        self.stop_marking();

        self.index_progress = None;
        self.last_index = None;
        self.keyword_loading = None;
        self.keyword_editor = None;
        self.images.invalidate();
        self.changed();
        let database = self
            .store
            .as_ref()
            .and_then(|s| s.path.parent())
            .map(|p| p.join("keyword-index.sqlite3"));
        self.folders
            .start(folder, self.lib.settings.recursive, database, force);
    }
    fn poll_folder(&mut self, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_millis(4);
        let mut changed = false;
        while Instant::now() < deadline && self.retire.available() && self.pending_open.is_none() {
            let Ok(update) = self.folders.try_recv() else {
                break;
            };
            match update {
                Update::Identities(snapshot) => {
                    if snapshot.serial == self.folders.identity_serial() {
                        self.identity_snapshot = Some(snapshot);
                        self.identity_token = self.identity_token.wrapping_add(1);
                        self.identity_pending = true;
                    }
                }
                Update::ImagesChangedAll => {
                    self.images.invalidate();
                    self.magnifier.reset();
                }
                Update::ImagesChanged(paths) => {
                    self.images.invalidate_paths(&paths);
                    if self.current.as_ref().is_some_and(|p| paths.contains(p)) {
                        self.magnifier.reset();
                    }
                }
                Update::Files(paths) => {
                    let first = self.loading;
                    self.loading = false;
                    self.retire.retire((
                        std::mem::replace(&mut self.files, paths.paths),
                        std::mem::replace(&mut self.folder_paths, paths.members),
                    ));
                    self.sort_files();

                    if first {
                        self.rebuild();
                        self.changed();
                    } else {
                        self.refresh_index_filter();
                        self.changed();
                    }
                    self.delete_selection
                        .retain(|p| self.folder_paths.contains(p));
                    self.tag_selection.retain(|p| self.folder_paths.contains(p));
                    self.move_selection
                        .retain(|p| self.folder_paths.contains(p));
                }
                Update::Keywords(entries) => {
                    changed |= folder::apply_keywords(
                        &mut self.lib,
                        &self.folder_paths,
                        &self.folders,
                        entries,
                    )
                }
                Update::Progress(progress) => self.index_progress = Some(progress),
                Update::Complete(progress) => {
                    self.index_progress = None;
                    self.last_index = Some(progress);
                    if progress.failed > 0 {
                        self.notify(format!("Index ready: {} image(s); keywords unreadable for {}. Images can still be browsed.",progress.total,progress.failed));
                    }
                }
                Update::Warning(message) => self.notify(message),
                Update::Error(message) => {
                    self.loading = false;
                    self.index_progress = None;
                    self.dialog = Some(Dialog::Error(message));
                    self.rebuild();
                }
            }
        }
        if let Some(delay) = self.folders.next_wake() {
            ctx.request_repaint_after(delay);
        }
        if changed {
            if self.filter.active() {
                self.refresh_index_filter();
            }
            self.changed();
        }
    }
    fn refresh_index_filter(&mut self) {
        self.schedule_view(false);
    }
    fn queue_identity(&mut self, ctx: &egui::Context) {
        if !self.identity_pending
            || self.identity_inflight
            || self.closing
            || self.restoring
            || self.busy
            || !self.persist_allowed
        {
            return;
        }
        let Some(snapshot) = self
            .identity_snapshot
            .as_ref()
            .filter(|s| s.serial == self.folders.identity_serial())
            .cloned()
        else {
            return;
        };
        if let Some(writer) = &mut self.writer {
            match writer.identify(&mut self.lib, snapshot, self.identity_token) {
                Ok(()) => {
                    self.identity_pending = false;
                    self.identity_inflight = true;
                }
                Err(_) => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }
    }
    /// The images the Marked / Selected to move filters allow (both on:
    /// those in both).
    fn filter_paths(&self) -> Option<std::sync::Arc<HashSet<PathBuf>>> {
        let marked = || self.marked.iter().cloned().collect::<HashSet<_>>();
        let set = match (self.filter.marked, self.filter.chosen) {
            (false, false) => return None,
            (true, false) => marked(),
            (false, true) => self.move_selection.clone(),
            (true, true) => marked()
                .into_iter()
                .filter(|p| self.move_selection.contains(p))
                .collect(),
        };
        Some(std::sync::Arc::new(set))
    }
    /// The marks or the grid's Move selection changed: a filter by them
    /// shows the change.
    fn marks_changed(&mut self, marks: bool) {
        if (marks && self.filter.marked) || (!marks && self.filter.chosen) {
            self.schedule_view(false);
        }
    }
    fn rebuild(&mut self) {
        // A new folder/filter/list: pages built before it no longer apply.
        self.grid_ghosts = None;
        self.schedule_view(true);
    }
    fn schedule_view(&mut self, reset: bool) {
        self.wheel.reset();
        self.view_pending = true;
        self.view_dirty = true;
        self.view_reset |= reset;
        self.tx.ctx.request_repaint();
    }
    fn poll_view(&mut self, ctx: &egui::Context) {
        if self.pending_open.is_some() {
            return;
        }
        if self.view_dirty {
            if !self.retire.available() {
                return;
            }
            if let Some(old) = self.view_build.take() {
                self.retire.retire(old);
            }
            let mut filter = self.filter.clone();
            filter.allowed = self.filter_paths();
            self.view_build = Some(ViewBuild::new(
                filter,
                self.view_reset,
                self.current.clone(),
                self.view_fallback,
            ));
            self.view_dirty = false;
        }
        let Some(build) = &mut self.view_build else {
            return;
        };
        let deadline = Instant::now() + Duration::from_millis(2);
        let mut done = false;
        // The ceiling also keeps synthetic/very fast predicates cooperative.
        for _ in 0..2048 {
            done = build.step(&self.files, &self.lib.photos, &self.nav);
            if done || Instant::now() >= deadline {
                break;
            }
        }
        if done && self.retire.available() {
            let built = self.view_build.take().unwrap().finish();
            self.retire.retire((
                std::mem::replace(&mut self.visible, built.visible),
                std::mem::replace(&mut self.nav, built.nav),
            ));
            if std::env::var_os("NOIRVIEW_FRAME_TRACE").is_some() {
                eprintln!(
                    "noirview view ready: visible={}, source={}",
                    self.visible.len(),
                    self.files.len()
                );
            }
            self.current = built.current;
            self.lib.last_image = self.current.clone();
            self.grid_start = if let Some(g) = &self.grid_ghosts {
                // After a deletion the grid stays on the page the user was on.
                g.start.min(self.visible.len().saturating_sub(1))
            } else if built.reset {
                0
            } else {
                self.grid_start.min(self.visible.len().saturating_sub(1))
            };
            self.view_pending = false;
            self.view_reset = false;
            self.view_fallback = None;
            self.last_slide = Instant::now();
            if !self.loading {
                if let Some((place, direct)) = self.restore_place.take() {
                    self.resume_place(place, direct);
                }
            }
            self.changed();
        } else if !done {
            ctx.request_repaint();
        }
    }
    fn sync_history_limit(&mut self) {
        if self.nav.set_history_limit(self.lib.settings.history_limit)
            && (self.view_build.is_some() || self.view_dirty)
        {
            // A partial view may already have copied the old navigation cursor.
            self.schedule_view(false);
        }
    }
    fn navigate(&mut self, next: bool) {
        self.sync_history_limit();
        // The grid's big-image view shows one chosen image, unless it can be
        // browsed (the grid page follows behind it).
        let big = self.grid_viewer.is_some();
        if self.visible.is_empty() || self.busy || self.view_pending || (big && !self.big_browse())
        {
            return;
        }
        let order = if self.slideshow {
            self.lib.settings.slideshow_order
        } else {
            self.lib.settings.order
        };
        // An image shown some other way (Go to, the grid, recently viewed):
        // browsing goes on from it.
        if let Some(c) = &self.current {
            self.nav.jump(&self.visible, c);
        }
        let path = if next {
            self.nav.next(&self.visible, self.current.as_deref(), order)
        } else {
            self.nav
                .previous(&self.visible, self.current.as_deref(), order)
        };
        if let Some(p) = path {
            self.current = Some(p.clone());
            self.lib.last_image = Some(p.clone());
            // Counted when browsed to, not when drawn: fast wheel browsing
            // can pass several images within one frame.
            self.count_round(&p);
            if big {
                self.grid_viewer = Some(p.clone());
                self.grid_ghosts = None;
            }
            if self.grid {
                let i = self.visible.position(&p).unwrap_or(0);
                let count = self.grid_count();
                self.grid_start = i / count * count;
            }
            if !self.tag_locked || !self.lib.settings.keep_selected_tags {
                self.selected_tags.clear();
            }
            self.panel = if self.tag_locked {
                Panel::Tags
            } else {
                Panel::None
            };
            self.last_slide = Instant::now();
            self.changed();
        }
    }
    /// Keeps the grid page as it is when a deletion starts. The open folder's
    /// watcher may report the removed files before the deletion itself
    /// finishes; without this the page would already have closed the gaps and
    /// the deleted tiles would vanish instead of staying marked.
    fn freeze_grid_page(&mut self, paths: &[PathBuf]) {
        if !self.grid {
            return;
        }
        let count = self.grid_count();
        if self.grid_ghosts.as_ref().is_none_or(|g| g.count != count) {
            self.grid_ghosts = Some(GridGhosts {
                page: self.page_paths().0,
                deleted: HashSet::new(),
                textures: HashMap::new(),
                start: self.grid_start,
                count,
            });
        }
        let keys: Vec<_> = paths
            .iter()
            .map(|p| (p.clone(), self.images.key(p.clone(), true)))
            .collect();
        let textures: Vec<_> = keys
            .into_iter()
            .filter_map(|(p, key)| Some((p, self.images.texture(&key)?)))
            .collect();
        if let Some(g) = &mut self.grid_ghosts {
            for (p, texture) in textures {
                if g.page.contains(&p) {
                    g.textures.entry(p).or_insert(texture);
                }
            }
        }
    }
    fn remove_from_view(&mut self, paths: &[PathBuf]) {
        let old_index = self
            .current
            .as_ref()
            .and_then(|p| self.visible.position(p))
            .unwrap_or(0);
        let removed: HashSet<_> = paths.iter().collect();
        if self.grid {
            let count = self.grid_count();
            let ghosts = self.grid_ghosts.take().filter(|g| g.count == count);
            let (page, start, mut deleted, mut textures) = match ghosts {
                Some(g) => (g.page, g.start, g.deleted, g.textures),
                None => (
                    self.page_paths().0,
                    self.grid_start,
                    HashSet::new(),
                    HashMap::new(),
                ),
            };
            for p in page.iter().filter(|p| removed.contains(p)) {
                deleted.insert(p.clone());
                let key = self.images.key(p.clone(), true);
                if let Some(texture) = self.images.texture(&key) {
                    textures.insert(p.clone(), texture);
                }
            }
            if !deleted.is_empty() {
                self.grid_ghosts = Some(GridGhosts {
                    page,
                    deleted,
                    textures,
                    start,
                    count,
                });
            }
        }
        if self
            .grid_viewer
            .as_ref()
            .is_some_and(|p| removed.contains(p))
        {
            self.grid_viewer = None;
        }
        self.files.retain(|p| !removed.contains(p));
        self.folder_paths.retain(|p| !removed.contains(p));
        self.delete_selection.retain(|p| !removed.contains(p));
        self.tag_selection.retain(|p| !removed.contains(p));
        self.move_selection.retain(|p| !removed.contains(p));
        self.marked.retain(|p| !removed.contains(p));
        if self.files.is_empty() {
            // Every image deleted or moved: nothing left to go back to.
            self.forget_place();
        }
        self.view_fallback = Some(old_index);
        // Not rebuild(): the deleted tiles of this page stay visible.
        self.schedule_view(true);
        self.changed();
    }
    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(e) = self.rx.try_recv() {
            match e {
                Event::State(Notice::Identified(result, token)) => {
                    self.identity_inflight = false;
                    if self.closing || self.restoring || token != self.identity_token {
                        continue;
                    }
                    if self.busy {
                        self.identity_pending = true;
                        self.identity_token = self.identity_token.wrapping_add(1);
                        continue;
                    }
                    match result {
                        Ok(plan) if plan.snapshot.serial == self.folders.identity_serial() => {
                            let changed = !plan.photos.is_empty() || !plan.marks.is_empty();
                            match plan.apply(&mut self.lib) {
                                Ok(count) => {
                                    if changed {
                                        self.changed();
                                        self.refresh_index_filter();
                                    }
                                    if count > 0 {
                                        self.notify(format!(
                                            "Preserved marks for {count} relocated image(s)"
                                        ));
                                    }
                                }
                                Err(()) => {
                                    self.identity_pending = true;
                                    self.identity_token = self.identity_token.wrapping_add(1);
                                    ctx.request_repaint();
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            self.notify(format!("File identity check unavailable: {error}"))
                        }
                    }
                }
                Event::ThumbnailCacheFlushed => {
                    self.close_allowed = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Event::CopyImage(result) => match result {
                    Ok(image) => {
                        ctx.copy_image(image);
                        self.notify("Image copied");
                    }
                    Err(e) => self.notify(format!("Copy image failed: {e}")),
                },
                Event::ThumbnailCacheCleared(result) => match result {
                    Ok(()) => {
                        // Memory too: the next page is prepared from the
                        // source files, and the background pass starts over.
                        self.images.forget_small();
                        self.grid_timer = None;
                        self.pregen_key = None;
                        self.notify("Thumbnail cache cleared (disk and memory)");
                    }
                    Err(error) => self.notify(format!("Cannot clear thumbnail cache: {error}")),
                },
                Event::Open(p) => self.open(p),
                Event::Destination(p) => {
                    self.lib.settings.destination = p;
                    if let Some(Dialog::Move(plan)) = &mut self.dialog {
                        plan.replan(&self.lib.settings.destination);
                    }
                    self.changed();
                }
                Event::Tracks(p) => {
                    self.lib.settings.music.extend(p);
                    self.changed();
                }
                Event::Model(p) => {
                    self.lib.settings.model_path = p;
                    self.images.invalidate();
                    self.changed();
                }
                Event::Runtime(p) => {
                    self.lib.settings.ort_library = p;
                    self.images.invalidate();
                    self.changed();
                    self.notify("Restart NoirView after changing the ONNX Runtime library");
                }
                Event::ExtraBackup(p) => {
                    self.lib.settings.backup_extra = p;
                    self.changed();
                }
                Event::DialogPlace(kind, place) => {
                    if self.lib.settings.dialog_places.get(&kind) != Some(&place) {
                        self.lib.settings.dialog_places.insert(kind, place);
                        self.changed();
                    }
                }
                Event::FavoriteFolder(path) => {
                    let favorites = &mut self.lib.settings.favorite_folders;
                    match self.favorite_pick.take().filter(|i| *i < favorites.len()) {
                        Some(i) => favorites[i].path = path,
                        None if favorites.iter().any(|f| f.path == path) => {}
                        None => favorites.push(crate::model::FavoriteFolder {
                            name: String::new(),
                            path,
                        }),
                    }
                    self.changed();
                }
                Event::ThumbnailCacheDir(p) => {
                    self.lib.settings.thumbnail_cache_dir = p;
                    self.changed();
                    self.notify("Restart NoirView to use the new thumbnail cache folder");
                }
                Event::WatchFolder(path) => {
                    let s = &mut self.lib.settings;
                    if !s.watched_folders.iter().any(|f| f.path == path) {
                        s.watched_folders.push(crate::model::WatchedFolder {
                            path,
                            subfolders: true,
                        });
                        self.changed();
                    }
                }
                Event::UsnStatus(state, version) => {
                    self.usn.state = Some(state);
                    self.usn.version = version;
                    self.usn.busy = false;
                    self.usn.checked = Some(Instant::now());
                }
                Event::UsnDone(message) => {
                    self.usn.message = Some(message);
                    self.usn.busy = false;
                    self.usn_refresh();
                }
                Event::WatchOwner(lock) => {
                    self.watch.refused = lock.is_none();
                    self.watch.lock = lock;
                    self.watch.background = crate::watch::control::dir()
                        .and_then(|d| crate::watch::control::read_status(&d))
                        .filter(|s| s.owner == "background");
                }
                Event::RestoreChoice(p) => self.dialog = Some(Dialog::Restore(p)),
                Event::KeywordsRead {
                    request,
                    path,
                    result,
                } => {
                    if request != self.keyword_request
                        || self.keyword_loading.as_ref() != Some(&path)
                    {
                        continue;
                    }
                    self.keyword_loading = None;
                    match result {
                        Ok(tags) => {
                            self.keyword_editor =
                                Some((path, tags.into_iter().collect::<Vec<_>>().join("\n")))
                        }
                        Err(message) => self.dialog = Some(Dialog::Error(message)),
                    }
                }
                Event::Tagged(results) => {
                    self.busy = false;
                    let mut count = 0;
                    let mut errors = vec![];
                    for (p, r) in results {
                        self.folders.finish_write(&p);
                        match r {
                            Ok(tags) => {
                                self.lib.learn_tags(&tags);
                                self.lib.rebind_file(&p);
                                self.lib.photos.entry(p).or_default().tags = tags;
                                count += 1;
                            }
                            Err(e) => errors.push(e),
                        }
                    }
                    self.changed();
                    if errors.is_empty() {
                        self.notify(format!("Keywords saved to {count} image(s)"));
                        let keep =
                            self.grid && self.tag_mode && self.lib.settings.grid_keep_tag_mode;
                        if !self.tag_locked && !keep {
                            self.panel = Panel::None;
                        }
                    } else {
                        self.dialog = Some(Dialog::Error(errors.join("\n")));
                    }
                    self.tag_selection.clear();
                    if self.filter.active() {
                        self.rebuild();
                    }
                }
                Event::Deleted(done, errors) => {
                    self.busy = false;
                    if !done.is_empty() {
                        // Restore deleted brings these back, with their marks.
                        self.lib.last_deletion = Some(crate::model::Deletion {
                            at: self.deleting_since,
                            items: done
                                .iter()
                                .map(|p| (p.clone(), self.lib.photos.get(p).cloned()))
                                .collect(),
                        });
                    }
                    for p in &done {
                        self.lib.photos.remove(p);
                    }
                    self.remove_from_view(&done);
                    self.notify(format!("Moved {} image(s) to Recycle Bin", done.len()));
                    if !errors.is_empty() {
                        self.dialog = Some(Dialog::Error(errors.join("\n")));
                    }
                }
                Event::Restored(done, errors) => {
                    self.busy = false;
                    self.restored(done, errors);
                }
                Event::Moved(report) => {
                    self.busy = false;
                    for (a, b) in &report.moved {
                        if let Some(mut meta) = self.lib.photos.remove(a) {
                            meta.identity = None;
                            self.lib.photos.insert(b.clone(), meta);
                        }
                    }
                    let paths: Vec<_> = report.moved.iter().map(|i| i.0.clone()).collect();
                    self.remove_from_view(&paths);
                    // Moved images leave the page at once (no "deleted" tiles: the
                    // folder watcher may already have dropped some of them).
                    self.grid_ghosts = None;
                    self.notify(format!(
                        "Moved {} image(s); skipped {}",
                        paths.len(),
                        report.skipped
                    ));
                    if !report.conflict.is_empty() {
                        self.dialog = Some(Dialog::Conflict(report.conflict));
                    } else if let Some(e) = report.error {
                        self.dialog = Some(Dialog::Error(e));
                    }
                }
                Event::State(Notice::Saved(result, flushing)) => match result {
                    Ok(()) if flushing && self.closing => {
                        self.finish_close(ctx);
                    }
                    Ok(()) => {}
                    Err(e) => {
                        self.dirty = true;
                        if flushing {
                            self.closing = false;
                            self.dialog =
                                Some(Dialog::Error(format!("Cannot save before closing: {e}")));
                        } else {
                            self.notify(format!("Cannot save settings: {e}"));
                        }
                    }
                },
                Event::State(Notice::BackedUp(result, slot)) => {
                    self.backup_busy = false;
                    match result {
                        Ok(p) => {
                            if let Some(slot) = slot {
                                self.lib.last_backup_slot = Some(slot);
                                self.changed();
                            }
                            self.notify(format!("Backup saved · {}", p.display()));
                            if self.closing {
                                self.flush_before_close(ctx);
                            }
                        }
                        Err(e) => {
                            self.closing = false;
                            self.next_backup_attempt = Instant::now() + Duration::from_secs(300);
                            self.dirty = true;
                            self.dialog = Some(Dialog::Error(e));
                        }
                    }
                }
                Event::State(Notice::Restored(r)) => {
                    self.busy = false;
                    self.restoring = false;
                    match r {
                        Ok(lib) => {
                            self.identity_token = self.identity_token.wrapping_add(1);
                            self.identity_snapshot = None;
                            self.identity_pending = false;
                            self.lib = *lib;
                            self.lib.settings.initialize_after_load();
                            self.tag_picker = crate::tags::TagPicker::default();
                            self.filter_picker = crate::tags::TagPicker::all();
                            self.selected_tags.clear();
                            self.persist_allowed = true;
                            self.images.invalidate();
                            if let Some(folder) = self.lib.last_folder.clone() {
                                self.open(folder);
                            }
                            self.notify("Backup restored");
                            self.changed();
                        }
                        Err(e) => {
                            self.dirty = self.persist_allowed;
                            if let Some(folder) = self.folder.clone() {
                                let database = self
                                    .store
                                    .as_ref()
                                    .and_then(|s| s.path.parent())
                                    .map(|p| p.join("keyword-index.sqlite3"));
                                self.folders.start(
                                    folder,
                                    self.lib.settings.recursive,
                                    database,
                                    false,
                                );
                            }
                            self.dialog = Some(Dialog::Error(e));
                        }
                    }
                }
                Event::Sorted(request, files) => {
                    if request == self.sort_request && files.len() == self.files.len() {
                        self.files = files;
                        self.rebuild();
                    }
                }
                Event::Recovered(r) => {
                    self.busy = false;
                    match r {
                        Ok(moved) => {
                            for (a, b) in &moved {
                                if let Some(mut m) = self.lib.photos.remove(a) {
                                    m.identity = None;
                                    self.lib.photos.insert(b.clone(), m);
                                }
                            }
                            let paths: Vec<_> = moved.into_iter().map(|i| i.0).collect();
                            self.remove_from_view(&paths);
                            self.notify(format!("Recovered {} move(s)", paths.len()));
                        }
                        Err(e) => self.dialog = Some(Dialog::Error(e)),
                    }
                }
            }
        }
    }
    fn edit_keywords(&mut self, path: PathBuf) {
        if self.view_pending {
            return;
        }
        self.keyword_request += 1;
        let request = self.keyword_request;
        self.keyword_loading = Some(path.clone());
        self.keyword_editor = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = metadata::read(&path).map_err(|e| format!("{}: {e:#}", path.display()));
            let _ = tx.send(Event::KeywordsRead {
                request,
                path,
                result,
            });
        });
    }
    fn save_tags(&mut self, replace: Option<(PathBuf, BTreeSet<String>)>) {
        if self.busy || self.view_pending {
            return;
        }
        let replacing = replace.is_some();
        let entries = if let Some(e) = replace {
            vec![e]
        } else {
            let paths: Vec<_> = if self.grid {
                self.tag_selection
                    .iter()
                    .filter(|p| !self.delete_selection.contains(*p))
                    .cloned()
                    .collect()
            } else {
                self.current.iter().cloned().collect()
            };
            paths
                .into_iter()
                .map(|p| (p, self.selected_tags.clone()))
                .collect()
        };
        if entries.is_empty() {
            self.notify("Select at least one image to tag");
            return;
        }
        for (path, _) in &entries {
            self.folders.begin_write(path);
        }
        self.busy = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let results = entries
                .into_iter()
                .map(|(p, tags)| {
                    let r = (|| -> anyhow::Result<_> {
                        let tags = if replacing {
                            tags
                        } else {
                            let mut current = metadata::read(&p)?;
                            current.extend(tags);
                            current
                        };
                        metadata::write(&p, &tags)?;
                        Ok(tags)
                    })()
                    .map_err(|e| format!("{}: {e:#}", p.display()));
                    (p, r)
                })
                .collect();
            let _ = tx.send(Event::Tagged(results));
        });
    }
    fn request_delete(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() || self.busy || self.view_pending {
            return;
        }
        if self.lib.settings.confirm_delete {
            self.dialog = Some(Dialog::Delete(paths));
        } else {
            self.delete(paths);
        }
    }
    fn delete(&mut self, paths: Vec<PathBuf>) {
        if self.view_pending {
            return;
        }
        self.busy = true;
        self.deleting_since = chrono::Utc::now().timestamp();
        self.freeze_grid_page(&paths);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let (done, errors) = operations::recycle(&paths);
            let _ = tx.send(Event::Deleted(done, errors));
        });
    }
    /// How many images Restore deleted would bring back.
    fn restorable(&self) -> usize {
        self.lib.last_deletion.as_ref().map_or(0, |d| d.items.len())
    }
    /// Restore deleted (button, menu, Ctrl+Z): asks first unless turned off.
    fn request_restore(&mut self) {
        let Some(deletion) = &self.lib.last_deletion else {
            self.notify("Nothing to restore");
            return;
        };
        if self.busy || self.view_pending {
            return;
        }
        if self.lib.settings.confirm_restore {
            let paths = deletion.items.iter().map(|(p, _)| p.clone()).collect();
            self.dialog = Some(Dialog::RestoreDeleted(paths));
        } else {
            self.restore_deleted();
        }
    }
    /// Moves the last deletion back from the Recycle Bin.
    fn restore_deleted(&mut self) {
        if self.busy || self.view_pending {
            return;
        }
        let Some(deletion) = &self.lib.last_deletion else {
            self.notify("Nothing to restore");
            return;
        };
        let paths: Vec<PathBuf> = deletion.items.iter().map(|(p, _)| p.clone()).collect();
        let since = deletion.at;
        self.busy = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let (done, errors) = operations::restore_deleted(&paths, since);
            let _ = tx.send(Event::Restored(done, errors));
        });
    }
    /// After a restore: marks back, the images in the view, the first one shown.
    fn restored(&mut self, done: Vec<PathBuf>, errors: Vec<String>) {
        if let Some(deletion) = &mut self.lib.last_deletion {
            for (path, meta) in &deletion.items {
                if let (true, Some(meta)) = (done.contains(path), meta) {
                    self.lib.photos.insert(path.clone(), meta.clone());
                }
            }
            // What could not be restored can be tried again.
            deletion.items.retain(|(p, _)| !done.contains(p));
            if deletion.items.is_empty() {
                self.lib.last_deletion = None;
            }
        }
        let folder = self.folder.clone();
        let mut added = false;
        for p in &done {
            let inside = folder.as_ref().is_some_and(|f| p.starts_with(f));
            if inside && self.folder_paths.insert(p.clone()) {
                self.files.push(p.clone());
                added = true;
            }
        }
        if let Some(first) = done.first().filter(|p| self.folder_paths.contains(*p)) {
            self.current = Some(first.clone());
            self.lib.last_image = Some(first.clone());
            if self.grid {
                self.grid_viewer = None;
                self.grid_jump_to = Some(first.clone());
            }
        }
        if added {
            self.sort_files();
        }
        self.rebuild();
        self.changed();
        if !done.is_empty() {
            self.notify(format!("Restored {} image(s) from Recycle Bin", done.len()));
        }
        if !errors.is_empty() {
            self.dialog = Some(Dialog::Error(errors.join("\n")));
        }
    }
    /// A button that does one thing when clicked and another when held
    /// (Settings → grid hold time): (clicked, the hold just fired).
    fn hold_click(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        key: &'static str,
    ) -> (bool, bool) {
        let fired = self.holds.get(key).is_some_and(|h| h.1);
        let clicked = response.clicked() && !fired;
        let mut long = false;
        if response.is_pointer_button_down_on() {
            let hold = Duration::from_millis(self.lib.settings.grid_hold_ms);
            let (since, done) = self.holds.entry(key).or_insert((Instant::now(), false));
            if !*done {
                let held = since.elapsed();
                if held >= hold {
                    *done = true;
                    long = true;
                } else {
                    ctx.request_repaint_after(hold - held);
                }
            }
        } else {
            self.holds.remove(key);
        }
        (clicked, long)
    }
    /// Round notice: counts the images shown in the image view with In
    /// order / Shuffle (browsed to, or drawn when shown another way); once
    /// every image of the list has been shown, a notice (or a dialog) says
    /// so and a new round starts.
    fn count_round(&mut self, path: &Path) {
        use crate::model::Order;
        if self.grid || self.round_last.as_deref() == Some(path) {
            return;
        }
        self.round_last = Some(path.to_path_buf());
        self.images_shown += 1;
        let order = if self.slideshow {
            self.lib.settings.slideshow_order
        } else {
            self.lib.settings.order
        };
        if self.round_order != Some(order) {
            self.round_order = Some(order);
            self.round_seen.clear();
        }
        if !self.lib.settings.round_notice {
            // Off: the round starts over when turned on again.
            self.round_seen.clear();
            return;
        }
        if order == Order::Random || self.visible.len() < 2 || self.visible.position(path).is_none()
        {
            return;
        }
        self.round_seen.insert(path.to_path_buf());
        // Images deleted or filtered out meanwhile do not count.
        if self.round_seen.len() >= self.visible.len()
            && self
                .round_seen
                .iter()
                .filter(|p| self.visible.position(p).is_some())
                .count()
                >= self.visible.len()
        {
            let n = self.visible.len();
            self.round_seen.clear();
            self.round_seen.insert(path.to_path_buf());
            if self.lib.settings.round_notice_dialog {
                if self.dialog.is_none() {
                    self.dialog = Some(Dialog::RoundDone(n));
                }
            } else {
                self.round_fade_out = None;
                self.round_message = Some((
                    format!("All {n} images shown: a new round begins"),
                    Instant::now(),
                    self.images_shown,
                ));
                self.tx.ctx.request_repaint();
            }
        }
    }
    /// Open folder: a click picks a folder, holding lists the favorites.
    fn folder_button(&mut self, ctx: &egui::Context, response: &egui::Response) {
        let (click, long) = self.hold_click(ctx, response, "folder");
        if click {
            self.pick("open");
        }
        if long {
            self.open_popup(Popup::Favorites, response.rect);
        }
    }
    fn open_popup(&mut self, popup: Popup, button: Rect) {
        self.popup = Some((popup, button));
        if popup == Popup::GoTo {
            self.goto_text.clear();
            self.goto_focus = true;
        }
    }
    /// Go to image `n` (the image view) or page `n` (the grid page, pages in
    /// order), counted from 1 and kept within the list.
    /// The saved place in a favorite folder (Settings → Library: favorite
    /// folders open where browsing left them; kept per profile).
    fn favorite_place(&self, folder: &Path) -> Option<&FolderPlace> {
        let s = &self.lib.settings;
        if !s.remember_favorite_places || !s.favorite_folders.iter().any(|f| f.path == folder) {
            return None;
        }
        s.favorite_places.get(&s.active_profile)?.get(folder)
    }
    fn forget_place(&mut self) {
        let s = &mut self.lib.settings;
        let Some(folder) = &self.folder else {
            return;
        };
        if let Some(places) = s.favorite_places.get_mut(&s.active_profile) {
            if places.remove(folder).is_some() {
                self.dirty = true;
            }
        }
        self.place_recorded = None;
    }
    /// A favorite folder opened: browsing goes on from its saved image (or
    /// the one now at its place in the list, if it is gone); with Shuffle
    /// the round goes on (images shown before are not shown again, images
    /// added since still come).
    fn resume_place(&mut self, place: FolderPlace, direct: bool) {
        let Some(last) = self.visible.len().checked_sub(1) else {
            return;
        };
        let target = if direct {
            self.current.clone()
        } else if self.visible.position(&place.image).is_some() {
            Some(place.image.clone())
        } else {
            self.visible.get(place.index.min(last)).cloned()
        };
        let Some(target) = target else {
            return;
        };
        let shuffle = crate::model::Order::Shuffle;
        let seen = if self.lib.settings.order == shuffle && place.order == shuffle {
            place.seen
        } else {
            vec![]
        };
        self.nav.resume(&self.visible, &seen, &target);
        self.lib.last_image = Some(target.clone());
        if self.grid {
            self.grid_jump_to = Some(target.clone());
        }
        self.current = Some(target);
        self.place_recorded = None;
    }
    /// Keeps where browsing is in a favorite folder (once per image shown).
    fn record_place(&mut self) {
        if !self.lib.settings.remember_favorite_places
            || self.loading
            || self.view_pending
            || self.restore_place.is_some()
        {
            return;
        }
        let (Some(folder), Some(current)) = (self.folder.clone(), self.current.clone()) else {
            return;
        };
        if self.place_recorded.as_ref() == Some(&current)
            || !self
                .lib
                .settings
                .favorite_folders
                .iter()
                .any(|f| f.path == folder)
        {
            return;
        }
        let order = self.lib.settings.order;
        let place = FolderPlace {
            index: self.visible.position(&current).unwrap_or(0),
            image: current.clone(),
            order,
            seen: if order == crate::model::Order::Shuffle {
                self.nav.round_seen()
            } else {
                vec![]
            },
        };
        let s = &mut self.lib.settings;
        s.favorite_places
            .entry(s.active_profile.clone())
            .or_default()
            .insert(folder, place);
        self.place_recorded = Some(current);
        self.dirty = true;
    }
    fn go_to(&mut self, n: usize) {
        if self.visible.is_empty() || self.view_pending {
            return;
        }
        let n = n.max(1);
        if self.grid && self.grid_viewer.is_none() {
            if !self.sequential_pages() {
                self.notify("Go to a page works with the In order page order");
                return;
            }
            let count = self.grid_count();
            let last = (self.visible.len() - 1) / count;
            self.preview = None;
            self.grid_ghosts = None;
            self.grid_start = (n - 1).min(last) * count;
            return;
        }
        let i = (n - 1).min(self.visible.len() - 1);
        let Some(p) = self.visible.get(i).cloned() else {
            return;
        };
        if i == 0 {
            // Image 1: a favorite folder's saved place starts over.
            self.forget_place();
            self.nav.start_over(&self.visible);
        }
        self.current = Some(p.clone());
        self.lib.last_image = Some(p.clone());
        if self.grid_viewer.is_some() {
            self.grid_viewer = Some(p);
        }
        self.last_slide = Instant::now();
        self.changed();
    }
    /// Favorite folders (Open folder held) and Go to (held), above their
    /// button. A press elsewhere, Escape or the action closes them.
    /// `late`: the Move dialog's Favourites, drawn after the dialogs so
    /// that it is above the dialog.
    fn button_popups(&mut self, ctx: &egui::Context, late: bool) {
        let Some((popup, button)) = self.popup else {
            return;
        };
        // Over the Move dialog it is drawn after the dialogs; from the Move
        // grid's bar like the other popups.
        let dialog = matches!(self.dialog, Some(Dialog::Move(_)));
        if (popup == Popup::Destination && dialog) != late {
            return;
        }
        if popup == Popup::Destination && !dialog && self.move_grid.is_none() {
            self.popup = None;
            return;
        }
        let mut done = false;
        // Just above the bar the button is on, not over its frame: only bars
        // on screen now count (a hidden bar keeps its last place in memory).
        let shown = |id: Id| {
            [egui::Order::Middle, egui::Order::Foreground]
                .into_iter()
                .any(|order| {
                    ctx.memory(|m| m.areas().visible_last_frame(&egui::LayerId::new(order, id)))
                })
                .then(|| ctx.memory(|m| m.area_rect(id)))
                .flatten()
        };
        let top = [
            Id::new("toolbar"),
            Id::new("grid_icon_bar"),
            Id::new("group_bar"),
            Id::new(("grid_overflow", true)),
            Id::new(("grid_overflow", false)),
        ]
        .into_iter()
        .filter_map(shown)
        .chain(self.controls_rect.filter(|_| self.on_grid_page()))
        .filter(|r| r.contains(button.center()))
        .map(|r| r.top())
        .fold(button.top(), f32::min);
        let anchor = Pos2::new(button.center().x, top);
        // As wide as the longest entry.
        let font = egui::TextStyle::Button.resolve(&ctx.style());
        let entries: Vec<String> = if popup == Popup::Profiles {
            self.profile_names()
                .into_iter()
                .chain(["Manage in Settings…".into()])
                .collect()
        } else {
            self.lib
                .settings
                .favorite_folders
                .iter()
                .map(|f| format!("🗀 {}", f.label()))
                .chain([
                    "☆ Remove the open folder".into(),
                    "Manage in Settings…".into(),
                ])
                .collect()
        };
        let widest = entries
            .into_iter()
            .map(|t| ctx.fonts(|f| f.layout_no_wrap(t, font.clone(), Color32::WHITE).size().x))
            .fold(0., f32::max);
        // Above the button, or below it when there is no room above.
        let below = Pos2::new(button.center().x, button.bottom());
        let width = (widest + 2. * ctx.style().spacing.button_padding.x + 4.).clamp(160., 420.);
        let rect = match popup {
            Popup::Destination => near(ctx, Id::new("favorite_destination"), anchor, below, |ui| {
                ui.set_width(width);
                ui.label(RichText::new("Move to a favorite folder").strong());
                ui.separator();
                let favorites = self.lib.settings.favorite_folders.clone();
                if favorites.is_empty() {
                    ui.label(RichText::new("No favorite folders yet.").color(MUTED));
                }
                let style = self.lib.settings.favorite_style.clone();
                let dest = self.lib.settings.destination.clone();
                for f in favorites {
                    let here = f.path == dest;
                    let r = match style.as_str() {
                        "text" => plain_text_link(ui, format!("/{}", f.label()), here, !self.busy),
                        "icon" => plain_text_link(ui, format!("🗀 {}", f.label()), here, !self.busy),
                        _ => {
                            ui.add_enabled(!self.busy, egui::Button::new(f.label()).selected(here))
                        }
                    }
                    .on_hover_text(f.path.display().to_string());
                    if self.hover.watch(r, false).clicked() {
                        let _ = self.tx.send(Event::Destination(f.path.clone()));
                        done = true;
                    }
                }
            }),
            Popup::Profiles => near(ctx, Id::new("profiles_popup"), anchor, below, |ui| {
                ui.set_width(width);
                ui.label(RichText::new("Profile").strong());
                ui.separator();
                let active = self.lib.settings.active_profile.clone();
                for name in self.profile_names() {
                    let key = if name == NO_PROFILE {
                        String::new()
                    } else {
                        name.clone()
                    };
                    let r = plain_text_link(ui, name, key == active, true);
                    if self.hover.watch(r, false).clicked() {
                        self.pending_profile = Some(key);
                        done = true;
                    }
                }
                ui.separator();
                let r = plain_text_link(ui, "Manage in Settings…".into(), false, true);
                if self.hover.watch(r, false).clicked() {
                    self.settings = true;
                    self.settings_tab = SETTINGS_PROFILES;
                    done = true;
                }
            }),
            Popup::Favorites => near(ctx, Id::new("favorite_folders"), anchor, below, |ui| {
                ui.set_width(width);
                ui.label(RichText::new("Favorite folders").strong());
                let open = self.folder.clone();
                let index = open.as_ref().and_then(|f| {
                    self.lib
                        .settings
                        .favorite_folders
                        .iter()
                        .position(|x| x.path == *f)
                });
                match (&open, index) {
                    (Some(_), Some(i)) => {
                        if self
                            .hover
                            .button(ui, "☆ Remove the open folder", false, true, false)
                        {
                            self.lib.settings.favorite_folders.remove(i);
                            self.changed();
                            done = true;
                        }
                    }
                    (Some(f), None) => {
                        if self
                            .hover
                            .button(ui, "★ Add the open folder", false, true, false)
                        {
                            self.lib
                                .settings
                                .favorite_folders
                                .push(crate::model::FavoriteFolder {
                                    name: String::new(),
                                    path: f.clone(),
                                });
                            self.changed();
                            done = true;
                        }
                    }
                    (None, _) => {}
                }
                ui.separator();
                let favorites = self.lib.settings.favorite_folders.clone();
                if favorites.is_empty() {
                    ui.label(RichText::new("No favorite folders yet.").color(MUTED));
                }
                let style = self.lib.settings.favorite_style.clone();
                for f in favorites {
                    let here = open.as_ref() == Some(&f.path);
                    // Buttons, or plain "/name", or a folder icon and the name.
                    let r = match style.as_str() {
                        "text" => plain_text_link(ui, format!("/{}", f.label()), here, !self.busy),
                        "icon" => plain_text_link(ui, format!("🗀 {}", f.label()), here, !self.busy),
                        _ => {
                            ui.add_enabled(!self.busy, egui::Button::new(f.label()).selected(here))
                        }
                    }
                    .on_hover_text(f.path.display().to_string());
                    if self.hover.watch(r, false).clicked() {
                        self.open(f.path.clone());
                        done = true;
                    }
                }
                ui.separator();
                if self
                    .hover
                    .button(ui, "Manage in Settings…", false, true, false)
                {
                    self.settings = true;
                    self.settings_tab = SETTINGS_FOLDERS;
                    self.settings_scroll = Some("favorites");
                    done = true;
                }
            }),
            Popup::GoTo => {
                let in_grid = self.grid && self.grid_viewer.is_none();
                let total = if in_grid {
                    self.visible.len().div_ceil(self.grid_count().max(1))
                } else {
                    self.visible.len()
                };
                let mut go = None;
                let rect = near(ctx, Id::new("go_to"), anchor, below, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(if in_grid { "Go to page" } else { "Go to image" });
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.goto_text)
                                .desired_width(70.)
                                .hint_text("1"),
                        );
                        if self.goto_focus {
                            edit.request_focus();
                            self.goto_focus = false;
                        }
                        self.goto_text.retain(|c| c.is_ascii_digit());
                        ui.label(format!("of {total}"));
                        let enter =
                            edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let number = self.goto_text.parse::<usize>().ok();
                        if self.hover.button(ui, "Go", false, number.is_some(), false) || enter {
                            go = number;
                            done = true;
                        }
                    });
                });
                if let Some(n) = go {
                    self.go_to(n);
                }
                rect
            }
        };
        let elsewhere = ctx.input(|i| {
            i.key_pressed(egui::Key::Escape)
                || i.pointer.any_pressed()
                    && i.pointer
                        .interact_pos()
                        .is_some_and(|p| !rect.contains(p) && !button.contains(p))
        });
        self.popup_rect = Some(rect);
        if done || elsewhere {
            self.popup = None;
            self.popup_rect = None;
        }
        if done {
            // Chosen from a button in a group bar: the group bar closes with
            // the popup, in the same frame (not first the group, then the
            // popup sliding down to the bar).
            let inside = |id: Id| {
                ctx.memory(|m| m.area_rect(id))
                    .is_some_and(|r| r.contains(button.center()))
            };
            if self.group_open.is_some() && inside(Id::new("group_bar")) {
                self.group_open = None;
            }
            if self.grid_overflow
                && (inside(Id::new(("grid_overflow", true)))
                    || inside(Id::new(("grid_overflow", false))))
            {
                self.grid_overflow = false;
            }
        }
    }
    fn backup(&mut self, slot: Option<String>) {
        if self.backup_busy {
            return;
        }
        if !self.persist_allowed {
            self.notify("Restore a valid backup before saving this library");
            return;
        }
        let slot = slot.or_else(|| storage::due_slot(&self.lib, chrono::Local::now()));
        let Some(writer) = &mut self.writer else {
            return;
        };
        match writer.backup(&mut self.lib, slot) {
            Ok(()) => {
                self.backup_busy = true;
                self.dirty = false;
            }
            Err(e) => {
                self.next_backup_attempt = Instant::now() + Duration::from_secs(5);
                self.notify(format!("{e:#}"));
            }
        }
    }
    fn restore_backup(&mut self, path: PathBuf) {
        let Some(writer) = &mut self.writer else {
            return;
        };
        match writer.restore(&mut self.lib, path, !self.persist_allowed) {
            Ok(()) => {
                self.busy = true;
                self.restoring = true;
                self.dirty = false;
                self.folders.cancel();
                self.pending_open = None;
                self.view_dirty = true;
                self.view_pending = true;
                self.loading = false;
                self.index_progress = None;
            }
            Err(e) => self.notify(format!("{e:#}")),
        }
    }
    fn finish_close(&mut self, ctx: &egui::Context) {
        self.closing = true;
        self.images.stop();
        let cache = self.images.disk_cache.clone();
        let proxies = self.images.proxy_cache.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let watcher = self.hand_watcher_to_background();
        std::thread::spawn(move || {
            // The engine finishes its current image and writes its keywords.
            let mut watcher = Some(watcher);
            let stopped = std::thread::spawn(move || {
                if let Some(watcher) = watcher.take() {
                    watcher()
                }
            });
            // Disposable cache must not prevent exit if the disk is unavailable.
            let _ = cache.wait_idle(Duration::from_secs(5));
            let _ = proxies.wait_idle(Duration::from_secs(3));
            let deadline = Instant::now() + Duration::from_secs(5);
            while !stopped.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = tx.send(Event::ThumbnailCacheFlushed);
            ctx.request_repaint();
        });
    }
    fn flush_before_close(&mut self, ctx: &egui::Context) {
        if !self.persist_allowed || self.writer.is_none() {
            self.finish_close(ctx);
            return;
        }
        if self.lib.settings.forget_deletion_on_close {
            self.lib.last_deletion = None;
        }
        let writer = self.writer.as_mut().unwrap();
        match writer.flush(&mut self.lib) {
            Ok(()) => {
                self.closing = true;
                self.dirty = false;
            }
            Err(e) => {
                self.closing = false;
                self.dialog = Some(Dialog::Error(format!("Cannot save before closing: {e:#}")));
            }
        }
    }
    /// Fullscreen with Settings → Display → Fullscreen shows only the image.
    fn image_only(&self) -> bool {
        self.fullscreen && self.lib.settings.fullscreen_image_only
    }
    fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
    }
    fn toggle_slideshow(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        let start = !self.slideshow;
        if start && self.grid {
            self.toggle_grid();
        }
        self.slideshow = start;
        self.paused = false;
        self.last_slide = Instant::now();
    }
    fn toggle_grid(&mut self) {
        if self.move_grid.is_some() {
            // Closing the Move grid goes back to the image view as it was.
            self.exit_move_grid();
            return;
        }
        self.wheel.reset();
        self.grid = !self.grid;
        if self.grid {
            self.slideshow = false;
            self.paused = false;
            self.grid_normal = self.initial_grid_normal();
            if self.lib.settings.grid_open_at_current {
                self.grid_jump_to = self.current.clone();
            }
        }
        // Filters per view: the grid and the image view keep their own.
        if self.lib.settings.filter_per_view {
            std::mem::swap(&mut self.filter, &mut self.other_filter);
            if self.filter.matches_same(&self.other_filter) {
                // Same filter on both sides: nothing to rebuild.
            } else {
                self.rebuild();
            }
        }
        self.grid_viewer = None;
        self.viewer_press = None;
        self.grid_ghosts = None;
        self.preview = None;
        self.hover_candidate = None;
        self.lib.last_grid = self.grid;
        self.bar = false;
        self.panel = Panel::None;
        self.delete_selection.clear();
        self.tag_selection.clear();
        self.tag_mode = false;
        self.tag_mode_paused = false;
        self.move_mode = false;
        self.move_selection.clear();
        self.grid_overflow = false;
        self.changed();
    }
    /// Images a Favorite/Elite choice applies to: the grid's tag selection while
    /// batch tagging, otherwise the current image.
    /// Favorite / Elite chooser above the tag palette's Favorite button; the
    /// palette stays open.
    fn tag_favorite_chooser(&mut self, ctx: &egui::Context) {
        let Some(anchor) = self.tag_favorite_popup else {
            return;
        };
        if !(self.panel == Panel::Tags || self.tag_locked) {
            self.tag_favorite_popup = None;
            return;
        }
        let mut choice = None;
        above(ctx, Id::new("tag_favorite_chooser"), anchor, |ui| {
            ui.horizontal(|ui| {
                let allow = !self.busy && !self.view_pending && !self.favorite_targets().is_empty();
                if self.hover.button(ui, "♥ Favorite", false, allow, false) {
                    choice = Some(Some(false));
                }
                if self.hover.button(ui, "★ Elite", false, allow, false) {
                    choice = Some(Some(true));
                }
                if self.hover.button(ui, "Cancel", false, true, false) {
                    choice = Some(None);
                }
            });
        });
        if let Some(choice) = choice {
            if let Some(elite) = choice {
                self.mark_favorite(elite);
            }
            self.tag_favorite_popup = None;
        }
    }
    fn favorite_targets(&self) -> Vec<PathBuf> {
        if self.grid && self.tag_mode {
            self.tag_selection.iter().cloned().collect()
        } else if self.grid && self.grid_viewer.is_none() {
            // The grid page has no "current" image: only a selection counts.
            vec![]
        } else {
            self.current.iter().cloned().collect()
        }
    }
    /// The grid's Tags button (also the toolbar's Tags icon on the grid
    /// page): tag mode with its palette, or back to deleting.
    fn toggle_grid_tags(&mut self) {
        if self.grid_normal {
            self.toggle_grid_normal();
        }
        self.tag_mode = !self.tag_mode;
        if !self.tag_mode {
            self.remember_selection();
            self.tag_selection.clear();
        }
        self.panel = if self.tag_mode {
            Panel::Tags
        } else {
            Panel::None
        };
    }
    fn mark_favorite(&mut self, elite: bool) {
        let targets = self.favorite_targets();
        self.mark_paths(&targets, elite);
    }
    fn mark_paths(&mut self, targets: &[PathBuf], elite: bool) {
        if targets.is_empty() {
            return;
        }
        for path in targets {
            let m = self.lib.photos.entry(path.clone()).or_default();
            m.favorite = true;
            m.elite |= elite;
        }
        self.changed();
        let what = if elite {
            "Elite and Favorites"
        } else {
            "Favorites"
        };
        self.notify(if targets.len() == 1 {
            format!("Added to {what}")
        } else {
            format!("Added {} images to {what}", targets.len())
        });
    }
    fn switch_panel(&mut self, panel: Panel) {
        self.tag_favorite_popup = None;
        self.panel = if self.panel == panel {
            Panel::None
        } else {
            panel
        };
    }
    fn central(&mut self, ctx: &egui::Context) {
        let grid_tiles = self.grid && self.grid_viewer.is_none();
        if grid_tiles || self.current.is_none() || self.visible.is_empty() {
            self.magnifier.reset();
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(12, 15, 20)))
            .show(ctx, |ui| {
                let image_view = !self.files.is_empty() && !self.visible.is_empty() && !grid_tiles;
                if image_view {
                    let canvas = ui.max_rect();
                    let mut image_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt("image-canvas")
                            .max_rect(canvas),
                    );
                    self.draw_image(&mut image_ui, ctx, canvas);
                }
                if image_view && self.image_only() {
                    return;
                }
                ui.add_space(14.);
                let title = self.top_title(grid_tiles);
                let raise = self.lib.settings.title_raise.clamp(-6., 6.);
                ui.horizontal(|ui| {
                    ui.add_space(20.);
                    // Room is left for Info, ? and Settings at the right.
                    let width = (ui.available_width() - 240.).max(160.);
                    title_line(ui, title, width, raise);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(16.);
                        if self
                            .hover
                            .button(ui, "Settings", self.settings, !self.busy, false)
                        {
                            self.settings = !self.settings;
                        }
                        if self.hover.button(ui, "?", false, true, false) {
                            self.help = true;
                        }
                        if self
                            .hover
                            .button(ui, "Info", self.info, self.current.is_some(), false)
                        {
                            self.info = !self.info;
                        }
                    });
                });
                ui.add_space(12.);
                if self.files.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() * 0.24).max(30.));
                        ui.label(
                            RichText::new("A little less interface.")
                                .size(36.)
                                .color(Color32::from_rgb(226, 233, 239)),
                        );
                        ui.label(RichText::new("A little more image.").size(36.).color(GREEN));
                        ui.add_space(16.);
                        ui.label(
                            RichText::new(if self.loading {
                                "Reading images and keywords…"
                            } else {
                                "Drop a folder or image here to begin."
                            })
                            .color(MUTED),
                        );
                        ui.add_space(20.);
                        ui.horizontal(|ui| {
                            let gap = (ui.available_width() - 290.).max(0.) / 2.;
                            ui.add_space(gap);
                            if self
                                .hover
                                .button(ui, "Open folder", false, !self.busy, false)
                            {
                                self.pick("open");
                            }
                            if self
                                .hover
                                .button(ui, "Open image", false, !self.busy, false)
                            {
                                self.pick("file");
                            }
                        });
                        ui.add_space(28.);
                        ui.label(
                            RichText::new(
                                "Scroll to browse  ·  Move to the bottom edge for controls",
                            )
                            .size(12.)
                            .color(MUTED),
                        );
                    });
                    return;
                }
                if self.visible.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            if self.view_pending {
                                ui.spinner();
                                ui.label("Updating view…");
                                return;
                            }
                            ui.label("No images match this filter.");
                            if self.hover.button(ui, "Clear filter", false, true, false) {
                                self.filter = Filter::default();
                                self.rebuild();
                            }
                        });
                    });
                    return;
                }
                if grid_tiles {
                    self.magnifier.reset();
                    self.draw_grid(ui);
                }
            });
    }
    /// The top bar's text after NOIRVIEW: the shown image's name, or on the
    /// grid page its first and last image (or the first and "…"), with the
    /// folder and the number of images when chosen (Settings → Library).
    fn top_title(&mut self, grid_tiles: bool) -> Option<String> {
        let name = |p: &Path| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        };
        let forward = self.lib.settings.path_forward_slashes;
        let folder = |p: &Path| {
            p.parent().map(|dir| {
                let text = format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR);
                if forward {
                    text.replace('\\', "/")
                } else {
                    text
                }
            })
        };
        if !grid_tiles {
            let p = self.current.as_deref()?;
            let dir = folder(p).filter(|_| self.lib.settings.image_title_folder);
            return Some(dir.unwrap_or_default() + &name(p));
        }
        let (page, _) = self.page_paths();
        let first = page.first()?;
        let s = &self.lib.settings;
        let mut title = String::new();
        if s.grid_caption_folder {
            if let Some(dir) = folder(first) {
                title.push_str(&dir);
            }
        }
        title.push_str(&name(first));
        if s.grid_caption_range {
            if let Some(last) = page.last().filter(|l| *l != first) {
                title.push_str(" – ");
                title.push_str(&name(last));
            }
        } else if page.len() > 1 {
            title.push_str(" – ...");
        }
        if s.grid_caption_count {
            title.push_str(&format!(" ({} images)", self.visible.len()));
        }
        Some(title)
    }
    fn draw_image(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, area: Rect) {
        let Some(path) = self.current.clone() else {
            return;
        };
        self.record_recent(&path);
        self.count_round(&path);
        let key = self.images.key(path.clone(), false);
        let blocked = self.settings
            || self.dialog.is_some()
            || self.info
            || self.help
            || self.keyword_editor.is_some()
            || self.keyword_loading.is_some()
            || self.new_tag_open
            || self.busy
            || self.view_pending
            || ctx.memory(|m| m.any_popup_open());
        if blocked {
            self.magnifier.reset();
        }
        if self.grid_viewer.is_some() && !blocked {
            self.grid_viewer_input(ctx, area);
        }
        let magnifier_delay = self.magnifier_delay(ctx);
        if self.view_timer.as_ref().is_none_or(|t| t.path != path) {
            self.view_timer = Some(ViewTimer {
                path: path.clone(),
                opened: Instant::now(),
                first: None,
                better: None,
                sharp: None,
            });
        }
        if let Some((texture, dimensions, full_detail)) = self
            .images
            .display_texture(&path, self.lib.settings.jpeg_preview)
        {
            let side = texture.size().into_iter().max().unwrap_or(0);
            let mut arrived = None;
            if let Some(t) = &mut self.view_timer {
                if full_detail && t.sharp.is_none() {
                    t.sharp = Some(t.opened.elapsed());
                    arrived = t.summary();
                } else if !full_detail {
                    t.reduced(side);
                }
            }
            if let Some(summary) = arrived {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.log_line(&format!("view {name}: {summary}"));
            }
            // An animated GIF plays over the first frame once its frames exist.
            // Not during the memory emergency brake.
            let frame = if self.lib.settings.animate_gif && !self.memory_critical() {
                let filter = if self.lib.settings.upscale == Upscale::Nearest {
                    egui::TextureOptions::NEAREST
                } else {
                    egui::TextureOptions::LINEAR
                };
                let (frame, note) = self.gif_player.frame(
                    ctx,
                    &path,
                    self.lib.settings.animation_mb,
                    filter,
                    Instant::now(),
                );
                if let Some(note) = note {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    let line = format!("Animation {name}: {note}");
                    self.log_line(&line);
                    self.notify(line);
                }
                frame
            } else {
                None
            };
            let shown = frame.unwrap_or(texture.id());
            self.previous_image = self
                .lib
                .settings
                .keep_previous_image
                .then(|| (path.clone(), texture.clone(), dimensions));
            let full_detail = full_detail || frame.is_some();
            let rect = crate::view::image_rect(area, dimensions, self.lib.settings.view_mode());
            ui.painter().with_clip_rect(area).image(
                shown,
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
                Color32::WHITE,
            );
            self.image_rect = Some(rect.intersect(area));
            // Marked to move: a mark at the image's top-left corner.
            if self.marking && !self.grid && self.marked.contains(&path) {
                // At the image's corner, below the title line.
                let corner = rect.intersect(area).left_top() + Vec2::splat(14.);
                paint_move_mark(
                    ui.painter(),
                    Pos2::new(corner.x, corner.y.max(area.top() + 44.)),
                );
            }
            let response = ui.interact(
                rect.intersect(area),
                ui.id().with("photo"),
                Sense::click_and_drag(),
            );
            if response.double_clicked() && self.grid_viewer.is_none() {
                self.toggle_fullscreen(ctx);
            }
            response.context_menu(|ui| {
                if self.hover.button(
                    ui,
                    "Move to Recycle Bin",
                    false,
                    !self.busy && !self.view_pending,
                    true,
                ) {
                    self.request_delete(vec![path.clone()]);
                    ui.close_menu();
                }
                if self
                    .hover
                    .button(ui, "Toggle fullscreen", false, true, false)
                {
                    self.toggle_fullscreen(ctx);
                    ui.close_menu();
                }
                if self.hover.button(
                    ui,
                    "Edit keywords",
                    false,
                    !self.busy && !self.view_pending,
                    false,
                ) {
                    self.edit_keywords(path.clone());
                    ui.close_menu();
                }
                if self
                    .hover
                    .button(ui, "Image information", false, true, false)
                {
                    self.info = true;
                    ui.close_menu();
                }
                self.shared_menu_items(ui, &path);
            });
            let allowed = response.contains_pointer() && !blocked;
            if self.magnifier.update(
                &path,
                ctx,
                allowed,
                self.lib.settings.magnifier && !blocked,
                magnifier_delay,
            ) {
                if let Some(pos) = ctx
                    .input(|i| i.pointer.interact_pos())
                    .filter(|_| full_detail)
                {
                    magnifier::paint(ui.painter(), shown, rect, area, pos, &self.lib.settings);
                    // The lens (or the whole-window zoom) shows where the
                    // pointer is; the pointer itself would cover the detail.
                    ctx.set_cursor_icon(egui::CursorIcon::None);
                }
            }
            // A first view that was really on screen stays reported for a
            // moment after full detail replaced it.
            const REPORT: Duration = Duration::from_secs(4);
            let report = self
                .view_timer
                .as_ref()
                .filter(|t| full_detail && t.had_first_view())
                .filter(|t| t.sharp.is_some_and(|s| t.opened.elapsed() < s + REPORT))
                .and_then(|t| Some((t.summary()?, t.sharp? + REPORT)));
            if let Some((_, until)) = &report {
                let opened = self.view_timer.as_ref().map(|t| t.opened);
                let left = opened.map_or(REPORT, |o| until.saturating_sub(o.elapsed()));
                ctx.request_repaint_after(left);
            }
            if (!full_detail || report.is_some()) && !self.image_only() {
                let (text, color) = if let Some((summary, _)) = report {
                    (summary, MUTED)
                } else if let Some(error) = self.images.failure(&key) {
                    (format!("Full image unavailable: {error}"), RED)
                } else {
                    (
                        format!("{} {side} px · loading full detail…", first_view_name(side)),
                        MUTED,
                    )
                };
                // On a plate: the note must stay readable over any image.
                let galley =
                    ui.painter()
                        .layout_no_wrap(text, egui::FontId::proportional(12.), color);
                let pos = area.left_bottom() + Vec2::new(8., -12. - galley.size().y);
                ui.painter().rect_filled(
                    Rect::from_min_size(pos, galley.size()).expand2(Vec2::new(6., 3.)),
                    4.,
                    Color32::from_black_alpha(170),
                );
                ui.painter().galley(pos, galley, color);
            }
        } else {
            self.magnifier.update(
                &path,
                ctx,
                false,
                self.lib.settings.magnifier && !blocked,
                magnifier_delay,
            );
            let failure = self.images.failure(&key);
            // The previous image stays until this one is ready (only while it
            // is still in the list: not after it was deleted or filtered out).
            let previous = self
                .previous_image
                .as_ref()
                .filter(|(p, _, _)| {
                    self.lib.settings.keep_previous_image
                        && failure.is_none()
                        && *p != path
                        && self.visible.position(p).is_some()
                })
                .map(|(_, texture, dimensions)| (texture.id(), *dimensions));
            if let Some((texture, dimensions)) = previous {
                let rect = crate::view::image_rect(area, dimensions, self.lib.settings.view_mode());
                ui.painter().with_clip_rect(area).image(
                    texture,
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
                    Color32::WHITE,
                );
                ui.put(
                    Rect::from_min_size(
                        area.left_bottom() + Vec2::new(10., -34.),
                        Vec2::splat(20.),
                    ),
                    egui::Spinner::new(),
                );
            } else {
                ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                    ui.centered_and_justified(|ui| {
                        if let Some(error) = failure {
                            ui.label(RichText::new(error).color(RED));
                        } else {
                            ui.spinner();
                        }
                    });
                });
            }
        }
        if self.image_only() {
            return;
        }
        let index = self.visible.position(&path).unwrap_or(0);
        let meta = self.lib.photos.get(&path).cloned().unwrap_or_default();
        let caption = format!(
            "{} / {}   {}{}",
            index + 1,
            self.visible.len(),
            if meta.elite {
                "★ Elite"
            } else if meta.favorite {
                "♥ Favorite"
            } else {
                ""
            },
            if self.slideshow {
                if self.paused {
                    "    Slideshow paused"
                } else {
                    "    Slideshow"
                }
            } else {
                ""
            }
        );
        ui.painter().text(
            Pos2::new(area.center().x, area.bottom() - 14.),
            egui::Align2::CENTER_CENTER,
            caption,
            egui::FontId::proportional(12.),
            MUTED,
        );
    }
    fn draw_grid(&mut self, ui: &mut egui::Ui) {
        use crate::grid_layout::{layout, Area};
        // Tag mode lasts while its palette is on screen: closing the palette
        // (Save, Escape, another panel) ends it and drops the tag selection.
        let palette = self.panel == Panel::Tags || self.tag_locked;
        if self.tag_mode && !palette {
            self.tag_mode = false;
            self.remember_selection();
            self.tag_selection.clear();
        }
        let count = self.grid_count();
        // Opened from the image view: start at that image's page.
        if !self.view_pending {
            if let Some(path) = self.grid_jump_to.take() {
                let ordered = self.sequential_pages();
                if let Some(i) = self.visible.position(&path).filter(|_| ordered) {
                    self.grid_start = i / count * count;
                }
            }
        }
        let s = &self.lib.settings;
        if !self.grid_icons() {
            let controls = if s.grid_controls_top {
                egui::TopBottomPanel::top("grid_controls")
            } else {
                egui::TopBottomPanel::bottom("grid_controls")
            };
            controls
                .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(16, 10)))
                .show_separator_line(false)
                .show_inside(ui, |ui| self.grid_controls(ui, count, false));
        } else if self.grid_bar_fixed() {
            // Room below the tiles for the always shown icon bar.
            let height = ui
                .ctx()
                .memory(|m| m.area_rect(Id::new("grid_icon_bar")))
                .map_or(62., |r| r.height());
            // The same room wherever the bar is: moved up, it goes over the
            // tiles and their size does not change.
            egui::TopBottomPanel::bottom("grid_icon_bar_room")
                .exact_height(height + 16.)
                .frame(egui::Frame::NONE)
                .show_separator_line(false)
                .show_inside(ui, |_| {});
        }
        // The controls may have changed the page or its size: pick the tiles
        // afterwards so this frame draws (and times) the page now selected.
        let count = self.grid_count();
        let all = self.move_grid_all();
        if !all {
            self.grid_start = self
                .grid_start
                .min(self.visible.len().saturating_sub(1) / count * count);
        }
        let ghosts = self
            .grid_ghosts
            .as_ref()
            .filter(|g| g.count == count && !all);
        let deleted: HashSet<PathBuf> = ghosts.map(|g| g.deleted.clone()).unwrap_or_default();
        let ghost_textures = ghosts.map(|g| g.textures.clone()).unwrap_or_default();
        let paths: Vec<_> = match ghosts {
            Some(g) => g.page.clone(),
            None => self.page_paths().0,
        };
        let rect = ui.available_rect_before_wrap().shrink2(Vec2::new(16., 12.));
        const GAP: f32 = 12.;
        const CAPTION: f32 = 34.;
        let area = Area {
            width: rect.width(),
            height: rect.height(),
            gap: GAP,
            caption: CAPTION,
        };
        self.grid_area = Some(area);
        // Laid out for the page size, not the paths on a short last page, so
        // cells keep the same size on every page.
        let l = layout(count, area);
        let cell = Vec2::new(l.image_w, l.image_h + CAPTION);
        let used = Vec2::new(
            l.columns as f32 * cell.x + (l.columns - 1) as f32 * GAP,
            l.rows as f32 * cell.y + (l.rows - 1) as f32 * GAP,
        );
        let clip = ui.clip_rect();
        // The Move grid's one page going on: the page's rows from the first
        // shown, sliding into place after a step (rows above or below drawn
        // while they slide out).
        let mut first_row = 0isize;
        let (paths, origin) = if all {
            let columns = l.columns.max(1);
            let row_height = cell.y + GAP;
            self.grid_columns = columns;
            self.grid_row_height = row_height;
            let len = self.visible.len();
            let last = len.div_ceil(columns).saturating_sub(l.rows) * columns;
            self.grid_start = (self.grid_start / columns * columns).min(last);
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            self.move_grid_scroll *= (1. - dt * 14.).clamp(0., 1.);
            if self.move_grid_scroll.abs() < 0.5 {
                self.move_grid_scroll = 0.;
            } else {
                ui.ctx().request_repaint();
            }
            let offset = self.move_grid_scroll;
            let before = (offset.max(0.) / row_height).ceil() as usize;
            let after = ((-offset).max(0.) / row_height).ceil() as usize;
            let first = self.grid_start.saturating_sub(before * columns);
            let end = (self.grid_start + (l.rows + after) * columns).min(len);
            first_row = -(((self.grid_start - first) / columns) as isize);
            ui.set_clip_rect(rect.expand2(Vec2::new(16., 4.)).intersect(clip));
            let paths: Vec<PathBuf> = self
                .visible
                .iter()
                .skip(first)
                .take(end - first)
                .cloned()
                .collect();
            (paths, rect.center() - used / 2. + Vec2::new(0., offset))
        } else {
            (paths, rect.center() - used / 2.)
        };
        let mut take_off = None;
        let mut loaded = true;
        self.grid_tiles.clear();
        let mut hovered = None;
        let mut ring = None;
        let mut all_rings: Vec<(Pos2, f32)> = vec![];
        let hover_preview = self.lib.settings.grid_hover_preview && self.preview.is_none();
        for (i, path) in paths.iter().enumerate() {
            let (col, row) = (i % l.columns, (i / l.columns) as isize + first_row);
            let min = origin + Vec2::new(col as f32 * (cell.x + GAP), row as f32 * (cell.y + GAP));
            let image_rect = Rect::from_min_size(min, Vec2::new(l.image_w, l.image_h));
            let response = ui.interact(
                image_rect,
                ui.id().with(("grid_tile", path)),
                Sense::click(),
            );
            let response = self.hover.watch(response, false);
            if hover_preview
                && self.lib.settings.hover_circle
                && self.lib.settings.hover_circle_always
            {
                let radius = image_rect.size().min_elem()
                    * self.lib.settings.hover_circle_pct.clamp(10., 100.)
                    / 200.;
                all_rings.push((image_rect.center(), radius));
            }
            if hover_preview && response.hovered() && !ui.input(|i| i.pointer.any_down()) {
                let s = &self.lib.settings;
                if s.hover_circle {
                    // Only a centred circle starts the preview; a faint ring shows it.
                    let radius =
                        image_rect.size().min_elem() * s.hover_circle_pct.clamp(10., 100.) / 200.;
                    let center = image_rect.center();
                    let distance = ui
                        .ctx()
                        .pointer_hover_pos()
                        .map_or(f32::INFINITY, |p| p.distance(center));
                    let inside = distance <= radius;
                    ring = Some((center, radius, distance));
                    if inside {
                        hovered = Some(path.clone());
                    }
                } else {
                    hovered = Some(path.clone());
                }
            }
            let deleting = self.delete_selection.contains(path);
            let tagging = self.tag_selection.contains(path);
            let moving = self.move_mode && self.move_selection.contains(path);
            let gone = deleted.contains(path);
            let painter = ui.painter_at(rect.expand(4.));
            painter.rect_filled(image_rect, 10., Color32::from_rgb(21, 26, 33));
            let key = self.images.key(path.clone(), true);
            let texture = if gone {
                ghost_textures.get(path).cloned()
            } else {
                self.images.texture(&key)
            };
            if let Some((texture, dim)) = texture {
                let factor = ((image_rect.width() - 12.) / dim[0] as f32)
                    .min((image_rect.height() - 12.) / dim[1] as f32);
                let dest = Rect::from_center_size(
                    image_rect.center(),
                    Vec2::new(dim[0] as f32, dim[1] as f32) * factor,
                );
                painter.image(
                    texture.id(),
                    dest,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
                    Color32::WHITE,
                );
            } else {
                let failed = self.images.failure(&key).is_some();
                loaded &= failed || gone;
                painter.text(
                    image_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    if failed {
                        "Cannot decode"
                    } else {
                        "Loading…"
                    },
                    egui::FontId::proportional(12.),
                    MUTED,
                );
            }
            // A copy from this tile's menu flashes it briefly.
            if let Some((_, since)) = self.copy_flash.as_ref().filter(|(p, _)| p == path) {
                let k = 1. - since.elapsed().as_secs_f32() / 0.35;
                if k > 0. {
                    painter.rect_filled(
                        image_rect,
                        10.,
                        Color32::from_white_alpha((k * 110.) as u8),
                    );
                    ui.ctx().request_repaint();
                } else {
                    self.copy_flash = None;
                }
            }
            // Marked to move in the single view: the same mark (it changes
            // nothing in the grid).
            let mark = if self.move_grid.is_some() {
                self.lib.settings.move_grid_marks
            } else {
                self.lib.settings.grid_move_marks
            };
            if mark && self.marked.contains(path) {
                let at = if self.move_grid.is_some() {
                    // Where set (corner and offset), inside the image box.
                    let s = &self.lib.settings;
                    corner_place(
                        image_rect.shrink(6.),
                        move_mark_size(&painter),
                        &s.move_mark_corner,
                        s.move_mark_offset,
                    )
                } else {
                    image_rect.left_top() + Vec2::splat(8.)
                };
                paint_move_mark(&painter, at);
            }
            // The Move grid: × at the corner takes the image off the marks.
            if self.move_grid.is_some() {
                let s = &self.lib.settings;
                let size = s.move_x_size.clamp(12., 48.);
                // In the corner chosen and moved in as set, inside the image
                // box at any size.
                let at = corner_place(
                    image_rect.shrink(6.),
                    Vec2::splat(size),
                    &s.move_x_corner,
                    s.move_x_offset,
                );
                let x = Rect::from_min_size(at, Vec2::splat(size));
                let r = ui
                    .interact(x, ui.id().with(("move_x", path)), Sense::click())
                    .on_hover_text("Take off the marked images");
                let hot = r.hovered();
                let opacity = s.move_x_opacity.clamp(0.1, 1.);
                let c = if hot {
                    [200, 60, 80, 255]
                } else {
                    s.move_x_color
                };
                let fill = Color32::from_rgba_unmultiplied(
                    c[0],
                    c[1],
                    c[2],
                    (c[3] as f32 * opacity) as u8,
                );
                painter.circle_filled(x.center(), size / 2., fill);
                painter.circle_stroke(
                    x.center(),
                    size / 2.,
                    Stroke::new(1.0_f32, Color32::from_white_alpha((160. * opacity) as u8)),
                );
                painter.text(
                    x.center(),
                    egui::Align2::CENTER_CENTER,
                    "×",
                    egui::FontId::proportional(size * 0.75),
                    Color32::from_white_alpha((255. * opacity) as u8),
                );
                if self.hover.watch(r, false).clicked() {
                    take_off = Some(path.clone());
                }
            }
            if deleting || tagging || moving {
                let color = if deleting {
                    RED
                } else if moving {
                    MOVE_BLUE
                } else {
                    GREEN
                };
                painter.rect_stroke(
                    image_rect,
                    10.,
                    Stroke::new(3.0_f32, color),
                    egui::StrokeKind::Inside,
                );
            }
            if gone {
                // Deleted on this page: shown until the page changes, inert.
                painter.rect_filled(
                    image_rect,
                    10.,
                    Color32::from_rgba_unmultiplied(190, 30, 50, 120),
                );
                painter.text(
                    image_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Deleted",
                    egui::FontId::proportional(14.),
                    Color32::WHITE,
                );
            } else if response.clicked() && !self.busy && !self.view_pending && self.grid_normal {
                self.open_grid_viewer(path.clone());
            } else if response.clicked() && !self.busy && !self.view_pending {
                if self.move_mode {
                    if deleting {
                        self.notify("Cannot move this image: it is marked for deletion");
                    } else {
                        if !self.move_selection.insert(path.clone()) {
                            self.move_selection.remove(path);
                        }
                        self.marks_changed(false);
                    }
                } else if self.tag_mode {
                    if deleting {
                        self.notify("Cannot tag this image: it is marked for deletion");
                    } else if !self.tag_selection.insert(path.clone()) {
                        self.tag_selection.remove(path);
                    }
                } else if !self.delete_selection.insert(path.clone()) {
                    self.delete_selection.remove(path);
                }
            }
            if !gone {
                self.grid_tiles.push((image_rect, path.clone()));
                self.grid_context_menu(&response, path);
            }
            let caption = Rect::from_min_size(
                image_rect.left_bottom() + Vec2::new(0., 3.),
                Vec2::new(l.image_w, CAPTION - 3.),
            );
            let mut caption_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(caption)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            caption_ui.spacing_mut().item_spacing.y = 1.;
            caption_ui.add(
                egui::Label::new(
                    RichText::new(path.file_name().unwrap_or_default().to_string_lossy())
                        .size(11.)
                        .color(MUTED),
                )
                .truncate(),
            );
            if let Some(m) = self
                .lib
                .photos
                .get(path)
                .filter(|_| self.lib.settings.grid_show_marks)
                .filter(|m| m.favorite || m.elite || !m.tags.is_empty())
            {
                let mut job = egui::text::LayoutJob::default();
                let font = egui::FontId::proportional(11.);
                let marks = match (m.elite, m.favorite) {
                    (true, _) if self.lib.settings.grid_marks_icons => "★",
                    (false, true) if self.lib.settings.grid_marks_icons => "♥",
                    (true, _) => "★ Elite",
                    (false, true) => "♥ Favorite",
                    _ => "",
                };
                if !marks.is_empty() {
                    // The heart is red, the rest in the marks' yellow.
                    let (heart, rest) = match marks.strip_prefix('♥') {
                        Some(rest) => ("♥", rest),
                        None => ("", marks),
                    };
                    if !heart.is_empty() {
                        job.append(heart, 0., egui::TextFormat::simple(font.clone(), HEART_RED));
                    }
                    job.append(rest, 0., egui::TextFormat::simple(font.clone(), TAG_YELLOW));
                }
                if !m.tags.is_empty() {
                    let tags = m.tags.iter().cloned().collect::<Vec<_>>().join(" · ");
                    let gap = if marks.is_empty() { 0. } else { 6. };
                    job.append(&tags, gap, egui::TextFormat::simple(font, GREEN));
                }
                caption_ui.add(egui::Label::new(job).truncate());
            }
        }
        if all {
            ui.set_clip_rect(clip);
        }
        if let Some(path) = take_off {
            self.take_off_marked(&path);
        }
        if !all_rings.is_empty() {
            // Always shown: the faint ring of every tile (the hovered one is
            // drawn below at its own strength).
            let faint = ring_alpha(0.001, 1., false, self.lib.settings.hover_circle_opacity);
            for (center, radius) in &all_rings {
                if ring.is_some_and(|(c, _, _)| c == *center) {
                    continue;
                }
                if let Some(alpha) = faint {
                    ui.painter().circle_stroke(
                        *center,
                        *radius,
                        Stroke::new(
                            self.lib.settings.hover_circle_width.clamp(0.5, 4.),
                            Color32::from_white_alpha(alpha),
                        ),
                    );
                }
            }
        }
        if let Some((center, radius, distance)) = ring {
            let s = &self.lib.settings;
            let faint = s
                .hover_circle_always
                .then(|| ring_alpha(0.001, 1., false, s.hover_circle_opacity))
                .flatten();
            let near = ring_alpha(
                distance - radius,
                s.hover_circle_reveal_px,
                s.hover_circle_fade,
                s.hover_circle_opacity,
            );
            if let Some(alpha) = near.max(faint) {
                ui.painter().circle_stroke(
                    center,
                    radius,
                    Stroke::new(
                        s.hover_circle_width.clamp(0.5, 4.),
                        Color32::from_white_alpha(alpha),
                    ),
                );
            }
        }
        // Page load timer: from the first frame of a page until every tile is
        // decoded (or failed). Cached pages finish on their first frame.
        self.grid_overlays(ui, rect, hovered);
        // Keyed by the tiles themselves, so a list still filling in at startup
        // or a changed page size restarts the measurement.
        if self
            .grid_timer
            .as_ref()
            .is_none_or(|(page, _, _)| *page != paths)
        {
            self.grid_timer = Some((paths.clone(), Instant::now(), None));
        }
        if let Some((_, started, done @ None)) = &mut self.grid_timer {
            if loaded {
                *done = Some((started.elapsed(), self.images.prepared_in(&paths)));
            }
        }
    }
    /// Page size in use (see `grid_layout::effective`).
    fn grid_count(&self) -> usize {
        crate::grid_layout::effective(
            self.page_size_setting(),
            self.round_full_setting(),
            self.visible.len(),
            self.grid_area,
        )
    }
    /// The Move grid shows all marked images on one page that goes on
    /// downwards: the next page (or row) slides up instead of a page turn.
    fn move_grid_all(&self) -> bool {
        self.move_grid.is_some() && self.lib.settings.move_grid_all
    }
    /// Pages follow the list in order (In order pages, or the Move grid's
    /// one page).
    fn sequential_pages(&self) -> bool {
        self.lib.settings.grid_page_order == crate::model::Order::Sequential || self.move_grid_all()
    }
    /// Page size and rounding: the Move grid has its own.
    fn page_size_setting(&self) -> usize {
        if self.move_grid.is_some() {
            self.lib.settings.move_grid_count
        } else {
            self.lib.settings.thumbnail_count
        }
    }
    fn round_full_setting(&self) -> bool {
        if self.move_grid.is_some() {
            self.lib.settings.move_grid_round_full
        } else {
            self.lib.settings.grid_round_full
        }
    }
    /// The grid's controls as an icon bar, always shown in the Move grid.
    fn grid_icons(&self) -> bool {
        self.move_grid.is_some() || self.lib.settings.grid_bar_icons
    }
    fn grid_bar_fixed(&self) -> bool {
        self.move_grid.is_some() || self.lib.settings.grid_bar_fixed
    }
    /// The big image from the grid can be browsed.
    fn big_browse(&self) -> bool {
        if self.move_grid.is_some() {
            self.lib.settings.move_grid_browse
        } else {
            self.lib.settings.grid_viewer_browse
        }
    }
    fn toggle_grid_normal(&mut self) {
        self.grid_normal = !self.grid_normal;
        self.lib.settings.grid_last_normal = self.grid_normal;
        self.changed();
        let keep = self.lib.settings.normal_keeps_selection;
        if self.grid_normal && !keep {
            self.remember_selection();
            self.delete_selection.clear();
            self.tag_selection.clear();
        }
        if self.grid_normal && self.tag_mode {
            // Normal mode has no tag palette; with the selection kept, tag
            // mode resumes on the way back instead of losing its selection.
            self.tag_mode = false;
            self.tag_mode_paused = keep;
            if !keep {
                self.tag_selection.clear();
            }
            if self.panel == Panel::Tags {
                self.panel = Panel::None;
            }
        } else if !self.grid_normal && std::mem::take(&mut self.tag_mode_paused) {
            self.tag_mode = true;
            self.panel = Panel::Tags;
        }
    }
    /// Large centred message for Select all / Clear / Restore selection.
    fn draw_big_toast(&mut self, ctx: &egui::Context) {
        if let Some((message, started)) = &self.big_toast {
            let left = Duration::from_millis(1600).saturating_sub(started.elapsed());
            if left.is_zero() {
                self.big_toast = None;
            } else {
                let size = self.lib.settings.selection_toast_size;
                let screen = ctx.screen_rect().size();
                let margin = size * 0.6;
                // The setting is the outer size; keep it inside the window.
                let inner = Vec2::new(
                    self.lib.settings.selection_toast_width.min(screen.x - 40.),
                    self.lib.settings.selection_toast_height.min(screen.y - 40.),
                ) - Vec2::splat(2. * margin);
                let inner = inner.max(Vec2::splat(10.));
                egui::Area::new(Id::new("big_toast"))
                    .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                    .order(egui::Order::Tooltip)
                    .interactable(false)
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style())
                            .corner_radius(16.)
                            .inner_margin(margin)
                            .show(ui, |ui| {
                                ui.set_min_size(inner);
                                ui.set_max_width(inner.x);
                                ui.allocate_ui_with_layout(
                                    inner,
                                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                                    |ui| {
                                        ui.label(RichText::new(message).size(size).color(GREEN));
                                    },
                                );
                            });
                    });
                ctx.request_repaint_after(left.min(Duration::from_millis(100)));
            }
        }
    }
    /// The round notice: size, colour and place from Settings → Interaction.
    fn draw_round_message(&mut self, ctx: &egui::Context) {
        let Some((message, started, shown_at)) = self.round_message.clone() else {
            return;
        };
        let s = &self.lib.settings;
        let left = Duration::from_secs_f32(s.round_notice_secs.clamp(0.5, 60.))
            .saturating_sub(started.elapsed());
        // Or gone once this many new images have been shown.
        let enough = s.round_notice_images > 0
            && self.images_shown.saturating_sub(shown_at) >= u64::from(s.round_notice_images);
        let fade = if s.round_notice_fade {
            Duration::from_millis(u64::from(s.round_notice_fade_ms.min(5000)))
        } else {
            Duration::ZERO
        };
        let (alpha, left) = if left.is_zero() || enough {
            // Fading out, or gone.
            let since = self
                .round_fade_out
                .get_or_insert_with(Instant::now)
                .elapsed();
            if since >= fade {
                self.round_message = None;
                self.round_fade_out = None;
                return;
            }
            (
                1. - since.as_secs_f32() / fade.as_secs_f32(),
                Duration::ZERO,
            )
        } else if started.elapsed() < fade {
            (started.elapsed().as_secs_f32() / fade.as_secs_f32(), left)
        } else {
            (1., left)
        };
        if self.image_only() {
            return;
        }
        let (align, offset) = match s.round_notice_place.as_str() {
            "top" => (egui::Align2::CENTER_TOP, Vec2::new(0., 65.)),
            "bottom" => (egui::Align2::CENTER_BOTTOM, Vec2::new(0., -110.)),
            _ => (egui::Align2::CENTER_CENTER, Vec2::ZERO),
        };
        let c = s.round_notice_color;
        let opacity = s.round_notice_opacity.clamp(0.05, 1.) * alpha.clamp(0., 1.);
        let color =
            Color32::from_rgba_unmultiplied(c[0], c[1], c[2], (c[3] as f32 * opacity) as u8);
        let size = s.round_notice_size.clamp(10., 120.);
        egui::Area::new(Id::new("round_message"))
            .anchor(align, offset)
            .order(egui::Order::Tooltip)
            .interactable(false)
            .show(ctx, |ui| {
                let fill = ui.visuals().window_fill;
                egui::Frame::popup(ui.style())
                    .fill(fill.gamma_multiply(opacity))
                    .shadow(egui::epaint::Shadow::NONE)
                    .corner_radius(14.)
                    .inner_margin(size * 0.5)
                    .show(ui, |ui| {
                        ui.set_max_width(ctx.screen_rect().width() - 80.);
                        ui.label(RichText::new(message).size(size).color(color));
                    });
            });
        if alpha < 1. {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(left.min(Duration::from_millis(250)));
        }
    }
    fn initial_grid_normal(&self) -> bool {
        match self.lib.settings.grid_start_mode {
            crate::model::GridStartMode::Selection => false,
            crate::model::GridStartMode::Normal => true,
            crate::model::GridStartMode::LastUsed => self.lib.settings.grid_last_normal,
        }
    }
    /// Feedback for the selection buttons: grid flash and/or a large toast.
    fn selection_feedback(&mut self, button: &'static str, message: String) {
        if self.lib.settings.selection_flash {
            self.flash_button = Some((button, Instant::now()));
        }
        if self.lib.settings.selection_toast {
            self.big_toast = Some((message, Instant::now()));
        }
        self.tx.ctx.request_repaint();
    }
    /// Keep the selection about to be replaced or cleared for Restore selection.
    fn remember_selection(&mut self) {
        if !self.delete_selection.is_empty() || !self.tag_selection.is_empty() {
            self.last_selection = Some((self.delete_selection.clone(), self.tag_selection.clone()));
        }
    }
    /// A grid control button that briefly lights up after it acted.
    /// Clears the active selection: only the images on this page (`view`),
    /// or all of it.
    fn clear_selection(&mut self, view: bool) {
        self.remember_selection();
        let message = if view {
            let page: HashSet<PathBuf> = self.page_paths().0.into_iter().collect();
            let selection = self.active_selection();
            let before = selection.len();
            selection.retain(|p| !page.contains(p));
            format!(
                "Cleared {} selected images on this page",
                before - selection.len()
            )
        } else {
            self.active_selection().clear();
            "Selection cleared".into()
        };
        self.selection_feedback("clear", message);
    }
    /// A grid control: a text button, or an icon in the grid icon bar (with
    /// its colour from the settings and `text` as the hover text). Buttons
    /// named by `selection_feedback` flash.
    #[allow(clippy::too_many_arguments)]
    fn grid_button(
        &mut self,
        ui: &mut egui::Ui,
        icons: bool,
        key: &'static str,
        icon: Icon,
        text: &str,
        active: bool,
        enabled: bool,
        danger: bool,
    ) -> egui::Response {
        let response = if icons {
            let color = self
                .lib
                .settings
                .grid_bar_colors
                .get(key)
                .map(|c| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]));
            ui.push_id(key, |ui| {
                self.hover
                    .icon_response(ui, icon, text, active, enabled, color, danger)
            })
            .inner
        } else {
            let rgba = |c: &[u8; 4]| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
            let fill = self.lib.settings.grid_text_colors.get(key).map(rgba);
            let color = self.lib.settings.grid_text_font_colors.get(key);
            let text = match color.filter(|_| enabled) {
                Some(c) => RichText::new(text)
                    .color(Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])),
                None => RichText::new(text),
            };
            // A box colour fills the button (the selected look stays for an
            // active one).
            let mut button = egui::Button::new(text).selected(active);
            if let Some(fill) = fill.filter(|_| !active) {
                button = button.fill(fill);
            }
            let response = ui.add_enabled(enabled, button);
            self.hover.watch(response, danger)
        };
        if let Some((which, since)) = self.flash_button {
            if which == key {
                let k = 1. - since.elapsed().as_secs_f32() / 0.45;
                if k <= 0. {
                    self.flash_button = None;
                } else {
                    ui.painter().rect_filled(
                        response.rect,
                        ui.style().visuals.widgets.inactive.corner_radius,
                        Color32::from_white_alpha((k * 170.) as u8),
                    );
                    ui.ctx().request_repaint();
                }
            }
        }
        response
    }
    /// Settings → Library → Window size diagnostics: draws the window's
    /// edges, a 100-point grid and the pointer as egui sees them, with the
    /// sizes winit, the surface and Windows report; changes and clicks go to
    /// data/noirview-window.log.
    fn window_diagnostics(&mut self, ctx: &egui::Context) {
        if !self.lib.settings.window_diagnostics {
            return;
        }
        let d = eframe::noirview_diag::window_diagnostics();
        let screen = ctx.screen_rect();
        let ppp = ctx.pixels_per_point();
        let (native, inner, outer, pointer, clicks) = ctx.input(|i| {
            let v = i.viewport();
            let clicks: Vec<Pos2> = i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        ..
                    } => Some(*pos),
                    _ => None,
                })
                .collect();
            (
                v.native_pixels_per_point,
                v.inner_rect,
                v.outer_rect,
                i.pointer.latest_pos(),
                clicks,
            )
        });
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Debug,
            Id::new("window_diagnostics"),
        ));
        let faint = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 46));
        let label = Color32::from_rgb(170, 200, 255);
        let small = egui::FontId::monospace(10.);
        let mut x = 100.;
        while x < screen.max.x {
            painter.vline(x, screen.y_range(), faint);
            painter.text(
                Pos2::new(x + 2., screen.min.y + 2.),
                egui::Align2::LEFT_TOP,
                format!("{x:.0}"),
                small.clone(),
                label,
            );
            x += 100.;
        }
        let mut y = 100.;
        while y < screen.max.y {
            painter.hline(screen.x_range(), y, faint);
            painter.text(
                Pos2::new(screen.min.x + 2., y + 2.),
                egui::Align2::LEFT_TOP,
                format!("{y:.0}"),
                small.clone(),
                label,
            );
            y += 100.;
        }
        painter.rect_stroke(
            screen,
            0.,
            Stroke::new(3.0_f32, Color32::RED),
            egui::StrokeKind::Inside,
        );
        for corner in [
            screen.left_top(),
            screen.right_top(),
            screen.left_bottom(),
            screen.right_bottom(),
        ] {
            painter.rect_filled(
                Rect::from_center_size(corner, Vec2::splat(24.)),
                0.,
                Color32::RED,
            );
        }
        let yellow = Color32::from_rgb(255, 220, 0);
        if let Some(p) = pointer {
            painter.hline(screen.x_range(), p.y, Stroke::new(1.0_f32, yellow));
            painter.vline(p.x, screen.y_range(), Stroke::new(1.0_f32, yellow));
            painter.circle_stroke(p, 8., Stroke::new(2.0_f32, yellow));
        }
        let px = |v: Vec2| format!("{:.1} × {:.1}", v.x * ppp, v.y * ppp);
        let pair = |v: [u32; 2]| format!("{} × {}", v[0], v[1]);
        let rect = |r: Option<[i32; 4]>| {
            r.map_or("-".into(), |r| {
                format!(
                    "({}, {})–({}, {}) = {} × {}",
                    r[0],
                    r[1],
                    r[2],
                    r[3],
                    r[2] - r[0],
                    r[3] - r[1]
                )
            })
        };
        let egui_rect = |r: Option<Rect>| {
            r.map_or("-".into(), |r| {
                format!(
                    "({:.1}, {:.1}) {:.1} × {:.1} pt",
                    r.min.x,
                    r.min.y,
                    r.width(),
                    r.height()
                )
            })
        };
        let sizes = format!(
            "egui screen {:.1} × {:.1} pt (→ {} px) ppp {:.4} native {}\n\
             winit at paint: inner {} outer {} scale {:.4} paint ppp {:.4}\n\
             surface {} · resized events {} (last {}) · resyncs {} · scale events {} ({:.4})\n\
             Windows client {} · window {} · DPI {}\n\
             egui inner {} · outer {}",
            screen.width(),
            screen.height(),
            px(screen.size()),
            ppp,
            native.map_or("-".into(), |n| format!("{n:.4}")),
            pair(d.paint_inner),
            pair(d.paint_outer),
            d.paint_scale,
            d.paint_ppp,
            pair(d.surface),
            d.resized_events,
            pair(d.last_resized),
            d.resyncs,
            d.scale_events,
            d.last_scale_event,
            rect(d.client_rect),
            rect(d.window_rect),
            d.dpi.map_or("-".into(), |v| v.to_string()),
            egui_rect(inner),
            egui_rect(outer),
        );
        let cursor = d
            .cursor
            .map_or("-".into(), |c| format!("{:.1}, {:.1} px", c[0], c[1]));
        let pointer_text = pointer.map_or("-".into(), |p| {
            format!(
                "{:.1}, {:.1} pt (→ {:.1}, {:.1} px)",
                p.x,
                p.y,
                p.x * ppp,
                p.y * ppp
            )
        });
        let text = format!(
            "NoirView window diagnostics — Settings → Library → Window size diagnostics turns this off\n\
             {sizes}\npointer {pointer_text} · winit cursor {cursor} · frame {}",
            d.frames
        );
        let galley = painter.layout_no_wrap(text, egui::FontId::monospace(12.), Color32::WHITE);
        let at = Pos2::new(screen.center().x - galley.size().x / 2., screen.min.y + 30.);
        painter.rect_filled(
            Rect::from_min_size(at, galley.size()).expand(8.),
            4.,
            Color32::from_rgba_unmultiplied(0, 0, 0, 215),
        );
        painter.galley(at, galley, Color32::WHITE);
        if sizes != self.window_logged {
            self.window_line(&sizes.replace('\n', " | "));
            self.window_logged = sizes;
        }
        for c in clicks {
            self.window_line(&format!(
                "click {:.1}, {:.1} pt (→ {:.1}, {:.1} px) winit cursor {cursor}",
                c.x,
                c.y,
                c.x * ppp,
                c.y * ppp
            ));
        }
    }
    /// One line in data/noirview-window.log (kept under 2 MiB).
    fn window_line(&self, line: &str) {
        let Some(dir) = self.store.as_ref().and_then(|s| s.path.parent()) else {
            return;
        };
        let path = dir.join("noirview-window.log");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
            let _ = std::fs::rename(&path, dir.join("noirview-window.old.log"));
        }
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let now = chrono::Local::now().format("%H:%M:%S%.3f");
            let _ = writeln!(f, "{now} {line}");
        }
    }
    fn log_line(&self, line: &str) {
        if !self.lib.settings.diagnostic_log {
            return;
        }
        let Some(path) = self
            .store
            .as_ref()
            .and_then(|s| s.path.parent())
            .map(|p| p.join("noirview.log"))
        else {
            return;
        };
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let _ = writeln!(f, "{secs} {line}");
        }
    }
    /// Right button: a short click opens the context menu (egui), a drag is a
    /// gesture. The configured gesture closes the hover preview or the big
    /// image, or (optionally) opens the tile it started on.
    fn gesture_frame(&mut self, ctx: &egui::Context) {
        let (pressed, down, pos) = ctx.input(|i| {
            (
                i.pointer.button_pressed(egui::PointerButton::Secondary),
                i.pointer.button_down(egui::PointerButton::Secondary),
                i.pointer.latest_pos(),
            )
        });
        if !self.lib.settings.gestures {
            self.gesture_track = None;
            return;
        }
        if pressed {
            self.gesture_track = pos.map(crate::gesture::Track::new);
        }
        if let (Some(track), Some(p)) = (&mut self.gesture_track, pos) {
            track.moved(p);
        }
        let s = self.lib.settings.clone();
        if down {
            if let Some(track) = self.gesture_track.as_ref().filter(|t| t.dragging()) {
                if s.gesture_trail && track.points.len() > 1 {
                    let c = s.gesture_trail_color;
                    ctx.layer_painter(egui::LayerId::new(
                        egui::Order::Tooltip,
                        Id::new("gesture_trail"),
                    ))
                    .add(egui::Shape::line(
                        track.points.clone(),
                        Stroke::new(
                            s.gesture_trail_width,
                            Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]),
                        ),
                    ));
                }
            }
            return;
        }
        // Button up: finish a track that was in progress.
        let Some(track) = self.gesture_track.take() else {
            return;
        };
        if !track.dragging() {
            return; // a click: the context menu handles it
        }
        let end = pos.unwrap_or(*track.points.last().unwrap());
        let (dx, dy, ms) = track.finish(end);
        let matched = s
            .gesture
            .matches(dx, dy, ms, s.gesture_min_px, s.gesture_max_ms);
        let start = track.points[0];
        let tile = (self.grid && s.gesture_opens_tile)
            .then(|| self.grid_tiles.iter().find(|(r, _)| r.contains(start)))
            .flatten()
            .map(|(_, p)| p.clone());
        let action = if !matched {
            "none"
        } else if self.preview.is_some() {
            self.preview = None;
            "close preview"
        } else if self.grid_viewer.is_some() {
            self.close_grid_viewer();
            "close big image"
        } else if let Some(path) = tile {
            self.open_grid_viewer(path);
            "open tile"
        } else {
            "no target"
        };
        self.log_line(&format!(
            "gesture dx={dx:.0} dy={dy:.0} ms={ms} matched={matched} action={action}"
        ));
    }
    /// Sort the file list by the chosen key. Name/type sort here; metadata keys
    /// are read on a worker and arrive as Event::Sorted.
    fn sort_files(&mut self) {
        self.sort_request += 1;
        let (key, desc) = (
            self.lib.settings.sort_key,
            self.lib.settings.sort_descending,
        );
        if key.needs_metadata() {
            let mut files = self.files.clone();
            let (tx, request) = (self.tx.clone(), self.sort_request);
            std::thread::spawn(move || {
                crate::sysinfo::lower_current_thread_priority();
                crate::sorting::sort(&mut files, key, desc);
                let _ = tx.send(Event::Sorted(request, files));
            });
        } else {
            crate::sorting::sort(&mut self.files, key, desc);
        }
    }
    fn open_grid_viewer(&mut self, path: PathBuf) {
        self.lib.last_image = Some(path.clone());
        self.current = Some(path.clone());
        self.grid_viewer = Some(path);
        self.viewer_press = None;
        self.magnifier.reset();
        self.panel = Panel::None;
        self.changed();
    }
    fn close_grid_viewer(&mut self) {
        self.grid_viewer = None;
        self.viewer_press = None;
        self.magnifier.reset();
        // The big image's panels and toolbar do not carry over to the grid
        // page (a locked toolbar stays).
        if !self.tag_mode {
            self.panel = Panel::None;
        }
        self.bar = false;
    }
    /// Big-image view from the grid: a short click (or the configured gesture)
    /// returns to the grid; a longer hold is left to the magnifier.
    fn grid_viewer_input(&mut self, ctx: &egui::Context, area: Rect) {
        let (pressed, released, pos) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.interact_pos(),
            )
        });
        let over_popup = ctx.memory(|m| m.any_popup_open());
        // A press on a floating control (the trash button, the toolbar) is
        // that control's, not a click on the image that returns to the grid.
        let over_control = pos
            .and_then(|p| ctx.layer_id_at(p))
            .is_some_and(|layer| layer.order != egui::Order::Background);
        if pressed && !over_popup && !over_control {
            if let Some(p) = pos.filter(|p| area.contains(*p)) {
                self.viewer_press = Some((Instant::now(), p));
            }
        }
        if released {
            if let (Some((started, origin)), Some(end)) = (self.viewer_press.take(), pos) {
                let d = end - origin;
                let held = started.elapsed().as_millis();
                if crate::gesture::is_click(held, d.x, d.y, self.lib.settings.grid_hold_ms) {
                    // With a panel open (e.g. tags) the click closes it first.
                    if self.panel != Panel::None {
                        self.panel = Panel::None;
                    } else {
                        self.close_grid_viewer();
                    }
                }
            }
        }
    }
    /// How long a press waits before magnifying: no wait in the ordinary
    /// viewer; in the big image from the grid the hold time, unless the
    /// pointer has moved (not a click) or the user chose to magnify at once.
    fn magnifier_delay(&self, ctx: &egui::Context) -> Duration {
        let s = &self.lib.settings;
        if self.grid_viewer.is_none() || s.magnify_at_once {
            return Duration::ZERO;
        }
        let moved = s.magnify_on_move
            && self.viewer_press.is_some_and(|(_, origin)| {
                ctx.input(|i| i.pointer.interact_pos())
                    .is_some_and(|p| p.distance(origin) > crate::gesture::CLICK_SLOP)
            });
        if moved {
            Duration::ZERO
        } else {
            Duration::from_millis(s.grid_hold_ms)
        }
    }
    fn active_selection(&mut self) -> &mut HashSet<PathBuf> {
        if self.tag_mode {
            &mut self.tag_selection
        } else {
            &mut self.delete_selection
        }
    }
    fn select_paths(&mut self, paths: Vec<PathBuf>) {
        let tagging = self.tag_mode;
        let marked = self.delete_selection.clone();
        let selection = self.active_selection();
        // Images marked for deletion cannot receive tags.
        selection.extend(
            paths
                .into_iter()
                .filter(|p| !tagging || !marked.contains(p)),
        );
    }
    /// Right-click menu of a grid tile. Favorite/Elite apply to the active
    /// selection when the clicked image belongs to it, otherwise to that image.
    fn grid_context_menu(&mut self, response: &egui::Response, path: &PathBuf) {
        let selection = if self.tag_mode {
            &self.tag_selection
        } else {
            &self.delete_selection
        };
        let targets: Vec<PathBuf> = if selection.contains(path) {
            selection.iter().cloned().collect()
        } else {
            vec![path.clone()]
        };
        let suffix = if targets.len() > 1 {
            format!(" ({})", targets.len())
        } else {
            String::new()
        };
        let idle = !self.busy && !self.view_pending;
        response.context_menu(|ui| {
            if self
                .hover
                .button(ui, "Toggle fullscreen", self.fullscreen, true, false)
            {
                self.toggle_fullscreen(ui.ctx());
                ui.close_menu();
            }
            if self.hover.button(ui, "Close grid", false, true, false) {
                self.toggle_grid();
                ui.close_menu();
            }
            ui.separator();
            if self
                .hover
                .button(ui, "Move this image to Recycle Bin", false, idle, true)
            {
                self.request_delete(vec![path.clone()]);
                ui.close_menu();
            }
            let marked = self.delete_selection.len();
            if self.hover.button(
                ui,
                format!("Move selected to Recycle Bin ({marked})"),
                false,
                idle && marked > 0,
                true,
            ) {
                self.request_delete(self.delete_selection.iter().cloned().collect());
                ui.close_menu();
            }
            ui.separator();
            if self.hover.button(
                ui,
                format!("♥ Add to Favorites{suffix}"),
                false,
                idle,
                false,
            ) {
                self.mark_paths(&targets, false);
                ui.close_menu();
            }
            if self
                .hover
                .button(ui, format!("★ Add to Elite{suffix}"), false, idle, false)
            {
                self.mark_paths(&targets, true);
                ui.close_menu();
            }
            self.shared_menu_items(ui, path);
        });
    }
    /// Menu items shared by the image view and the grid: generation data,
    /// the recently viewed images and Exit.
    fn shared_menu_items(&mut self, ui: &mut egui::Ui, path: &Path) {
        if self.generation.as_ref().is_none_or(|(p, _)| p != path) {
            self.generation = Some((path.to_path_buf(), crate::comfy::read(path)));
        }
        // Only what the image really has: an empty prompt or seed is greyed
        // out (some files carry a generation block without them).
        let generation = self
            .generation
            .as_ref()
            .and_then(|(_, g)| g.clone())
            .filter(|g| {
                !g.positive.is_empty()
                    || !g.negative.is_empty()
                    || g.seed.as_ref().is_some_and(|s| !s.is_empty())
            });
        ui.separator();
        let copy = self.hover.button(
            ui,
            "Copy prompt(s) and seed",
            false,
            generation.is_some(),
            false,
        );
        let copy = copy.then_some(()).and(generation.as_ref());
        if let Some(g) = copy {
            ui.ctx().copy_text(g.clipboard());
            self.notify("Prompt and seed copied");
            self.copy_flash = Some((path.to_path_buf(), Instant::now()));
            ui.close_menu();
        }
        for (label, text, done) in [
            (
                "Copy positive prompt",
                generation.as_ref().map(|g| g.positive.clone()),
                "Positive prompt copied",
            ),
            (
                "Copy negative prompt",
                generation.as_ref().map(|g| g.negative.clone()),
                "Negative prompt copied",
            ),
        ] {
            let text = text.filter(|t| !t.is_empty());
            if self.hover.button(ui, label, false, text.is_some(), false) {
                if let Some(text) = text {
                    ui.ctx().copy_text(text);
                    self.notify(done);
                    self.copy_flash = Some((path.to_path_buf(), Instant::now()));
                }
                ui.close_menu();
            }
        }
        let seed = generation
            .as_ref()
            .and_then(|g| g.seed.clone())
            .filter(|s| !s.is_empty());
        if self
            .hover
            .button(ui, "Copy seed", false, seed.is_some(), false)
        {
            if let Some(seed) = seed {
                self.notify(format!("Seed {seed} copied"));
                ui.ctx().copy_text(seed);
                self.copy_flash = Some((path.to_path_buf(), Instant::now()));
            }
            ui.close_menu();
        }
        if self.hover.button(
            ui,
            "Show prompt and seed…",
            false,
            generation.is_some(),
            false,
        ) {
            // That image's, until closed.
            self.prompt_window = Some(path.to_path_buf());
            self.prompt_follows = false;
            ui.close_menu();
        }
        ui.separator();
        if self.hover.button(ui, "Copy file", false, true, false) {
            self.copy_file(path);
            ui.close_menu();
        }
        if self.hover.button(ui, "Copy image", false, true, false) {
            self.copy_image(path);
            ui.close_menu();
        }
        if self.hover.button(
            ui,
            format!("Recently viewed ({})", self.recent.len()),
            self.recent_open,
            !self.recent.is_empty(),
            false,
        ) {
            self.recent_open = true;
            ui.close_menu();
        }
        let restorable = self.restorable();
        if self.hover.button(
            ui,
            format!("Restore deleted ({restorable})"),
            false,
            restorable > 0 && !self.busy && !self.view_pending,
            false,
        ) {
            self.request_restore();
            ui.close_menu();
        }
        ui.menu_button("Profile", |ui| {
            let active = self.lib.settings.active_profile.clone();
            for name in self.profile_names() {
                let key = if name == NO_PROFILE {
                    String::new()
                } else {
                    name.clone()
                };
                if self
                    .hover
                    .button(ui, name.as_str(), key == active, true, false)
                {
                    self.pending_profile = Some(key);
                    ui.close_menu();
                }
            }
        });
        ui.separator();
        if self.hover.button(ui, "Exit NoirView", false, true, true) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            ui.close_menu();
        }
    }
    /// Shows an image from the recently viewed list.
    fn show_recent(&mut self, path: PathBuf) {
        if self.visible.position(&path).is_none() {
            // Another folder, or hidden by the filter: open it there.
            self.open(path);
        } else if self.grid {
            self.open_grid_viewer(path);
        } else if !self.busy && !self.view_pending {
            self.current = Some(path.clone());
            self.lib.last_image = Some(path);
            self.changed();
        }
    }
    fn recent_window(&mut self, ctx: &egui::Context) {
        if !self.recent_open {
            return;
        }
        let mut open = true;
        let mut close = false;
        let mut show = None;
        egui::Window::new("Recently viewed")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                const THUMB: Vec2 = Vec2::new(120., 80.);
                for path in self.recent.clone() {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    ui.horizontal(|ui| {
                        let (rect, response) = ui.allocate_exact_size(THUMB, Sense::click());
                        ui.painter()
                            .rect_filled(rect, 6., Color32::from_rgb(21, 26, 33));
                        let key = self.images.key(path.clone(), true);
                        if let Some((texture, dim)) = self.images.texture(&key) {
                            let size = Vec2::new(dim[0] as f32, dim[1] as f32);
                            let scale = (THUMB.x / size.x).min(THUMB.y / size.y);
                            ui.painter().image(
                                texture.id(),
                                Rect::from_center_size(rect.center(), size * scale),
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
                                Color32::WHITE,
                            );
                        } else {
                            ui.put(rect, egui::Spinner::new());
                        }
                        if self
                            .hover
                            .watch(response.on_hover_text("Show this image"), false)
                            .clicked()
                        {
                            show = Some(path.clone());
                        }
                        ui.vertical(|ui| {
                            ui.set_width(300.);
                            ui.add(egui::Label::new(&name).truncate())
                                .on_hover_text(path.display().to_string());
                            ui.horizontal(|ui| {
                                if self.hover.button(ui, "Copy name", false, true, false) {
                                    ui.ctx().copy_text(name.clone());
                                    self.notify(format!("{name} copied"));
                                }
                                if self.hover.button(ui, "Show", false, true, false) {
                                    show = Some(path.clone());
                                }
                            });
                        });
                    });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if self.hover.button(ui, "Copy all names", false, true, false) {
                        let names: Vec<String> = self
                            .recent
                            .iter()
                            .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into())
                            .collect();
                        ui.ctx().copy_text(names.join("\n"));
                        self.notify(format!("{} names copied", names.len()));
                    }
                    if self.hover.button(ui, "Close", false, true, false) {
                        close = true;
                    }
                });
            });
        if let Some(path) = show {
            self.show_recent(path);
            close = true;
        }
        if !open || close {
            self.recent_open = false;
        }
    }
    /// Remembers a shown image in the recently viewed list.
    fn record_recent(&mut self, path: &Path) {
        if self.recent.front().is_some_and(|p| p == path) {
            return;
        }
        self.recent.retain(|p| p != path);
        self.recent.push_front(path.to_path_buf());
        self.recent.truncate(RECENT);
    }
    /// Sort key and direction; used in the grid controls and in Settings.
    /// The sort keys as a text list: "Name ⏶" … The active one has a green
    /// ✔; clicking it turns its direction, clicking another chooses it.
    fn sort_choices(&mut self, ui: &mut egui::Ui) -> Option<(crate::sorting::SortKey, bool)> {
        let (key, desc) = (
            self.lib.settings.sort_key,
            self.lib.settings.sort_descending,
        );
        let mut chosen = None;
        ui.vertical(|ui| {
            for k in crate::sorting::SortKey::ALL {
                let active = k == key;
                let shown = active && desc;
                let mut job = egui::text::LayoutJob::default();
                let font = egui::TextStyle::Button.resolve(ui.style());
                let color = ui.visuals().text_color();
                job.append(
                    &sort_label(k, shown),
                    0.,
                    egui::TextFormat::simple(font.clone(), color),
                );
                if active {
                    job.append("  ✔", 0., egui::TextFormat::simple(font, GREEN));
                }
                let response = ui.add_enabled(
                    !self.busy,
                    egui::Button::new(job)
                        .frame(false)
                        .wrap_mode(egui::TextWrapMode::Extend),
                );
                let response = self.hover.watch(response, false).on_hover_text(if active {
                    "Click to turn the direction (⏶ ascending, ⏷ descending)"
                } else {
                    "Sort by this (⏶ ascending; click again for ⏷ descending)"
                });
                if response.clicked() {
                    chosen = Some((k, if active { !desc } else { false }));
                }
            }
        });
        chosen
    }
    fn set_sort(&mut self, key: crate::sorting::SortKey, desc: bool) {
        let s = &mut self.lib.settings;
        if (key, desc) != (s.sort_key, s.sort_descending) {
            s.sort_key = key;
            s.sort_descending = desc;
            self.changed();
            self.sort_files();
            if !key.needs_metadata() {
                self.rebuild();
            }
        }
    }
    /// Hover preview (open after resting on a tile; closes from the top strip,
    /// the gesture or Escape) and the selection flash, drawn over the tiles.
    fn grid_overlays(&mut self, ui: &mut egui::Ui, area: Rect, hovered: Option<PathBuf>) {
        let s = self.lib.settings.clone();
        match (hovered, &self.hover_candidate) {
            (Some(p), Some((c, since))) if *c == p => {
                let delay = Duration::from_millis(s.hover_preview_ms);
                let rested = since.elapsed();
                if rested >= delay {
                    self.preview = Some(p);
                    self.preview_close = None;
                    self.hover_candidate = None;
                } else {
                    ui.ctx().request_repaint_after(delay - rested);
                }
            }
            (Some(p), _) => self.hover_candidate = Some((p, Instant::now())),
            (None, _) => self.hover_candidate = None,
        }
        let Some(path) = self.preview.clone() else {
            self.preview_close = None;
            self.preview_origin = None;
            return;
        };
        // Optionally the preview closes once the pointer has moved away from
        // where it opened.
        if s.preview_close_move_px > 0. {
            if let Some(p) = ui.ctx().pointer_hover_pos() {
                if self.preview_origin.as_ref().is_none_or(|(o, _)| *o != path) {
                    self.preview_origin = Some((path.clone(), p));
                }
                let moved = self.preview_origin.as_ref().is_some_and(|(_, at)| {
                    let d = p - *at;
                    let travelled = match s.preview_close_move_dir.as_str() {
                        "up" => -d.y,
                        "down" => d.y,
                        "left" => -d.x,
                        "right" => d.x,
                        _ => d.length(),
                    };
                    travelled > s.preview_close_move_px
                });
                if moved {
                    self.preview = None;
                    self.preview_close = None;
                    self.preview_origin = None;
                    return;
                }
            }
        }
        let frame = Rect::from_center_size(
            area.center(),
            area.size() * (s.preview_size / 100.).clamp(0.4, 1.),
        );
        // Swallow pointer input so tiles underneath do not react.
        ui.interact(
            frame,
            ui.id().with("hover_preview"),
            Sense::click_and_drag(),
        );
        let painter = ui.painter();
        painter.rect_filled(frame, 14., Color32::from_rgba_unmultiplied(8, 10, 14, 245));
        painter.rect_stroke(
            frame,
            14.,
            Stroke::new(1.0_f32, Color32::from_white_alpha(40)),
            egui::StrokeKind::Inside,
        );
        let inner = frame.shrink(14.);
        let image_area = Rect::from_min_max(inner.min, inner.max - Vec2::new(0., 22.));
        if let Some((texture, dim, _)) = self.images.display_texture(&path, true) {
            let factor =
                (image_area.width() / dim[0] as f32).min(image_area.height() / dim[1] as f32);
            let dest = Rect::from_center_size(
                image_area.center(),
                Vec2::new(dim[0] as f32, dim[1] as f32) * factor,
            );
            painter.image(
                texture.id(),
                dest,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
                Color32::WHITE,
            );
        } else {
            painter.text(
                image_area.center(),
                egui::Align2::CENTER_CENTER,
                "Loading…",
                egui::FontId::proportional(14.),
                MUTED,
            );
        }
        painter.text(
            Pos2::new(inner.center().x, inner.bottom() - 8.),
            egui::Align2::CENTER_CENTER,
            path.file_name().unwrap_or_default().to_string_lossy(),
            egui::FontId::proportional(12.),
            MUTED,
        );
        // A left click (option), or resting on an enabled invisible strip at
        // the top / left / right of the window, closes the preview.
        if s.preview_close_click && ui.input(|i| i.pointer.primary_clicked()) {
            self.preview = None;
            self.preview_close = None;
            return;
        }
        let screen = ui.ctx().screen_rect();
        let px = s.preview_close_px;
        let at_edge = ui.ctx().pointer_hover_pos().is_some_and(|p| {
            (s.preview_close_top && p.y <= screen.top() + px)
                || (s.preview_close_left && p.x <= screen.left() + px)
                || (s.preview_close_right && p.x >= screen.right() - px)
        });
        if !at_edge {
            self.preview_close = None;
            return;
        }
        let since = *self.preview_close.get_or_insert_with(Instant::now);
        let delay = Duration::from_millis(s.preview_close_ms);
        if since.elapsed() >= delay {
            self.preview = None;
            self.preview_close = None;
        } else {
            ui.ctx().request_repaint_after(delay - since.elapsed());
        }
    }
    /// Tiles of the current page and of the next one, in the page order.
    fn page_paths(&mut self) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let count = self.grid_count();
        let order = self.lib.settings.grid_page_order;
        if self.sequential_pages() {
            let page = |start: usize| {
                self.visible
                    .iter()
                    .skip(start)
                    .take(count)
                    .cloned()
                    .collect::<Vec<_>>()
            };
            return (page(self.grid_start), page(self.grid_start + count));
        }
        self.grid_pages.sync(&self.visible, count, order);
        let (page, next) = self.grid_pages.current(&self.visible, count, order);
        // Images deleted meanwhile drop out of remembered pages.
        let present = |v: Vec<PathBuf>| {
            v.into_iter()
                .filter(|p| self.visible.position(p).is_some())
                .collect::<Vec<_>>()
        };
        (present(page), present(next))
    }
    fn grid_page(&mut self, next: bool) {
        self.preview = None;
        let count = self.grid_count();
        if self.move_grid_all() {
            // One page going on: a page (or a row) further, sliding.
            let columns = self.grid_columns.max(1);
            let step = if self.lib.settings.move_grid_row {
                columns
            } else {
                count.div_ceil(columns) * columns
            };
            let rows = count.div_ceil(columns);
            let last = self.visible.len().div_ceil(columns).saturating_sub(rows) * columns;
            let start = self.grid_start / columns * columns;
            let to = if next {
                (start + step).min(last)
            } else {
                start.saturating_sub(step)
            };
            if to != start {
                let moved = (to as f32 - start as f32) / columns as f32;
                self.move_grid_scroll += moved * self.grid_row_height;
                self.grid_start = to;
            }
            return;
        }
        if !self.sequential_pages() {
            // Everything fits on one page: there is no other page to show.
            if self.visible.len() <= count {
                return;
            }
            self.grid_ghosts = None;
            // Optionally both directions draw a new page (no history).
            self.grid_pages
                .step(next || self.lib.settings.grid_random_both);
            return;
        }
        // Only the page still shown: ghosts of a page left meanwhile (Go to,
        // a jump) would turn from the wrong place.
        if let Some(g) = self
            .grid_ghosts
            .take()
            .filter(|g| g.start == self.grid_start)
        {
            // Images after a deleted one moved up into this page: the next page
            // starts where the shown (pre-deletion) page really ended.
            self.grid_start = if next {
                (g.start + g.count - g.deleted.len()).min(self.visible.len().saturating_sub(1))
            } else {
                g.start.saturating_sub(count)
            };
            return;
        }
        let wrap = self.lib.settings.grid_page_wrap && self.visible.len() > count;
        if next {
            if self.grid_start + count < self.visible.len() {
                self.grid_start += count;
            } else if wrap {
                self.grid_start = 0;
            }
        } else if self.grid_start == 0 && wrap {
            // The last page.
            self.grid_start = (self.visible.len() - 1) / count * count;
        } else {
            self.grid_start = self.grid_start.saturating_sub(count);
        }
    }
    fn set_grid_count(&mut self, count: usize) {
        let count = count.clamp(1, crate::grid_layout::MAX_COUNT);
        if count != self.page_size_setting() {
            self.grid_ghosts = None;
            if self.move_grid.is_some() {
                self.lib.settings.move_grid_count = count;
            } else {
                self.lib.settings.thumbnail_count = count;
            }
            // Keep the first image of the current page on screen.
            let shown = self.grid_count();
            self.grid_start = self.grid_start / shown * shown;
            self.changed();
        }
    }
    /// The controls of the grid page in their order (settings decide some).
    /// The grid's controls in their order (Settings → Display → Bar
    /// buttons): the bar's items with the status at the end, then the
    /// group's, and how many are the bar's. Items only shown in some cases
    /// (page arrows, page-size buttons, …) are left out when not.
    fn grid_items(&self, icons: bool) -> (Vec<GridItem>, usize) {
        use GridItem::*;
        let s = &self.lib.settings;
        // The floating toolbar's other controls when it is hidden.
        let hidden = if icons {
            s.grid_hide_toolbar
        } else {
            s.grid_hide_toolbar_text
        };
        let shown = |item: GridItem| match item {
            PrevPage | NextPage => s.grid_page_arrows,
            Presets => !s.grid_presets.is_empty(),
            Folder | FloatingTrash => icons || hidden,
            Lock => icons && !s.grid_bar_fixed && self.move_grid.is_none(),
            _ => true,
        };
        let all = self.move_grid_all();
        let moving = self.move_grid.is_some();
        let shown = |item: GridItem| match item {
            // One page going on downwards: the arrows step it.
            PrevPage | NextPage if all => true,
            Folder | FloatingTrash if moving => false,
            _ => shown(item),
        };
        let item = |key: &str| {
            if moving {
                move_grid_entry(key)
            } else {
                GRID_LAYOUT.iter().find(|b| b.0 == key)
            }
            .map(|b| b.4)
            .filter(|i| shown(*i))
        };
        let items = if moving {
            self.bar_items("move_grid", &move_grid_defaults())
        } else {
            self.bar_items("grid", &grid_defaults())
        };
        let (on_bar, grouped) = split_places(&items);
        let mut out: Vec<GridItem> = on_bar.iter().filter_map(|k| item(k)).collect();
        out.push(Status);
        let bar = out.len();
        out.extend(grouped.iter().filter_map(|k| item(k)));
        (out, bar)
    }
    /// The grid's controls (text buttons or the icon bar) on one line. Items
    /// that do not fit go behind a » button at the right, which shows them
    /// in a bar above (like a browser's bookmark bar).
    fn grid_controls(&mut self, ui: &mut egui::Ui, count: usize, icons: bool) {
        let (items, on_bar) = self.grid_items(icons);
        let spacing = ui.spacing().item_spacing.x;
        let available = ui.available_width();
        let row_height = CONTROLS_ROW;
        let width = |v: &Self, item: GridItem| {
            v.grid_item_widths
                .get(&(icons, item))
                .copied()
                .unwrap_or(40.)
        };
        // The group's items are always behind », with what does not fit.
        let total: f32 = items[..on_bar]
            .iter()
            .map(|i| width(self, *i) + spacing)
            .sum();
        let fit = if total <= available + spacing && on_bar == items.len() {
            items.len()
        } else {
            const MORE: f32 = 40.;
            let mut used = MORE;
            items[..on_bar]
                .iter()
                .take_while(|i| {
                    used += width(self, **i) + spacing;
                    used <= available
                })
                .count()
        };
        let mut more = None;
        let row = ui
            .horizontal(|ui| {
                // The row's height first: shorter widgets centre on it.
                ui.set_min_height(row_height);
                for item in &items[..fit] {
                    let rect = ui
                        .allocate_ui_with_layout(
                            Vec2::new(ui.available_width(), row_height),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| self.grid_item(ui, *item, icons, count),
                        )
                        .response
                        .rect;
                    self.grid_item_widths.insert((icons, *item), rect.width());
                }
                if fit < items.len() {
                    let open = self.grid_overflow;
                    let response = self.grid_button(
                        ui,
                        true,
                        "more",
                        Icon::More,
                        "More controls",
                        open,
                        true,
                        false,
                    );
                    if response.clicked() {
                        self.grid_overflow = !open;
                    }
                    more = Some(response.rect);
                }
            })
            .response
            .rect;
        // The bar around the row (the icon bar's frame, or the text panel's
        // margin): menus and the overflow bar open above it.
        // The padding of the overflow bar: the icon bar's frame margin.
        let margin = Vec2::new(12., 8.);
        // The icon bar's frame; the text controls have no frame, so their
        // top is the buttons' top (they are centred in the taller row).
        let bar = if icons {
            row.expand2(margin)
        } else {
            let inset = ((CONTROLS_ROW - button_height(ui)) / 2.).max(0.);
            row.expand2(Vec2::new(0., -inset))
        };
        self.controls_rect = Some(bar);
        match more {
            Some(button) if self.grid_overflow => {
                let ctx = ui.ctx().clone();
                let rest = items[fit..].to_vec();
                // Rows of the measured widths, never wider than the bar.
                let max = (bar.width() - 2. * margin.x).max(200.);
                let mut rows: Vec<Vec<GridItem>> = vec![vec![]];
                let mut used = 0.;
                for item in rest {
                    let w = width(self, item) + spacing;
                    if used + w > max && !rows.last().unwrap().is_empty() {
                        rows.push(vec![]);
                        used = 0.;
                    }
                    used += w;
                    rows.last_mut().unwrap().push(item);
                }
                let s = self.lib.settings.clone();
                let response = egui::Area::new(Id::new(("grid_overflow", icons)))
                    .pivot(egui::Align2::RIGHT_BOTTOM)
                    .fixed_pos(bar.right_top() - Vec2::new(0., 6.))
                    .constrain(true)
                    .order(egui::Order::Foreground)
                    .show(&ctx, |ui| {
                        let frame = if icons {
                            egui::Frame::NONE.fill(Color32::from_rgba_unmultiplied(
                                s.bar_color[0],
                                s.bar_color[1],
                                s.bar_color[2],
                                s.bar_color[3],
                            ))
                        } else {
                            egui::Frame::popup(ui.style())
                        };
                        frame
                            .corner_radius(if icons { 18. } else { 12. })
                            .inner_margin(egui::Margin::symmetric(margin.x as i8, margin.y as i8))
                            .stroke(Stroke::new(1.0_f32, Color32::from_white_alpha(22)))
                            .show(ui, |ui| {
                                for row in &rows {
                                    ui.horizontal(|ui| {
                                        ui.set_min_height(row_height);
                                        for item in row {
                                            let rect = ui
                                                .allocate_ui_with_layout(
                                                    Vec2::new(ui.available_width(), row_height),
                                                    egui::Layout::left_to_right(
                                                        egui::Align::Center,
                                                    ),
                                                    |ui| self.grid_item(ui, *item, icons, count),
                                                )
                                                .response
                                                .rect;
                                            self.grid_item_widths
                                                .insert((icons, *item), rect.width());
                                        }
                                    });
                                }
                            });
                    })
                    .response;
                ctx.move_to_top(response.layer_id);
                // A press elsewhere closes it (not one inside, e.g. a menu).
                let menu = response.rect;
                let popup = self.popup_rect.filter(|_| self.popup.is_some());
                let elsewhere = ctx.input(|i| {
                    i.pointer.any_pressed()
                        && i.pointer.interact_pos().is_some_and(|p| {
                            !menu.contains(p)
                                && !button.contains(p)
                                && !popup.is_some_and(|r| r.contains(p))
                        })
                });
                if elsewhere && self.grid_menu.is_none() && self.clear_menu.is_none() {
                    self.grid_overflow = false;
                }
            }
            Some(_) => {}
            None => self.grid_overflow = false,
        }
        self.grid_menu_ui(ui.ctx());
    }
    /// One control of the grid's controls.
    fn grid_item(&mut self, ui: &mut egui::Ui, item: GridItem, icons: bool, count: usize) {
        use GridItem::*;
        let fixed = self.lib.settings.grid_bar_fixed_width;
        let ordered = self.sequential_pages();
        // The Move grid selects in normal mode too (a click shows the image).
        let selecting =
            (!self.grid_normal || self.move_grid.is_some()) && !self.busy && !self.view_pending;
        let b = |v: &mut Self, ui: &mut egui::Ui, key, icon, text: &str, active, enabled| {
            v.grid_button(ui, icons, key, icon, text, active, enabled, false)
                .clicked()
        };
        // Page arrows and the page-size bookmark look like the toolbar's in
        // both styles.
        let icon = |v: &mut Self, ui: &mut egui::Ui, key, icon, text: &str, enabled| {
            v.grid_button(ui, true, key, icon, text, false, enabled, false)
                .clicked()
        };
        match item {
            PrevPage | NextPage => {
                let (has_previous, has_next) = if ordered {
                    let wrap = self.lib.settings.grid_page_wrap && self.visible.len() > count;
                    (
                        self.grid_start > 0 || wrap,
                        self.grid_start + count < self.visible.len() || wrap,
                    )
                } else {
                    let more = self.visible.len() > count;
                    let back = if self.lib.settings.grid_random_both {
                        more
                    } else {
                        more && !self.grid_pages.at_start()
                    };
                    (back, more)
                };
                if item == PrevPage {
                    if icon(
                        self,
                        ui,
                        "prev_page",
                        Icon::Previous,
                        "Previous page",
                        has_previous,
                    ) {
                        self.grid_page(false);
                    }
                } else if icon(self, ui, "next_page", Icon::Next, "Next page", has_next) {
                    self.grid_page(true);
                }
            }
            Normal => {
                if b(
                    self,
                    ui,
                    "normal",
                    Icon::Eye,
                    "Normal",
                    self.grid_normal,
                    true,
                ) {
                    self.toggle_grid_normal();
                }
            }
            HoverPreview => {
                let preview = self.lib.settings.grid_hover_preview;
                if b(
                    self,
                    ui,
                    "hover_preview",
                    Icon::Preview,
                    "Hover preview",
                    preview,
                    true,
                ) {
                    self.lib.settings.grid_hover_preview = !preview;
                    if preview {
                        self.preview = None;
                    }
                    self.changed();
                }
            }
            Sort | Pages => {
                let (menu, key, icon, text) = if item == Sort {
                    let s = &self.lib.settings;
                    let text = format!("Sort: {}", sort_label(s.sort_key, s.sort_descending));
                    (GridMenu::Sort, "sort", Icon::Sort, text)
                } else {
                    let s = &self.lib.settings;
                    let text = format!(
                        "Pages: {}",
                        order_choice_label(s.grid_page_order, s.grid_random_both)
                    );
                    (GridMenu::Pages, "pages", Icon::Pages, text)
                };
                let open = self.grid_menu.is_some_and(|(m, _)| m == menu);
                let enabled = item == Pages || !self.busy;
                let r = self.grid_button(ui, icons, key, icon, &text, open, enabled, false);
                if r.clicked() {
                    self.grid_menu = (!open).then_some((menu, r.rect));
                }
            }
            Trash => {
                let marked = self.delete_selection.len();
                let r = self.grid_button(
                    ui,
                    icons,
                    "trash",
                    Icon::Trash,
                    &format!("Trash ({marked})"),
                    false,
                    selecting && marked > 0,
                    true,
                );
                if icons && marked > 0 {
                    count_badge(ui, r.rect, marked, RED);
                }
                if r.clicked() {
                    self.request_delete(self.delete_selection.iter().cloned().collect());
                }
            }
            SelectBoth => {
                let r = self.grid_button(
                    ui,
                    icons,
                    "select",
                    Icon::SelectPage,
                    "Select this page (hold: all)",
                    false,
                    selecting,
                    false,
                );
                let (click, long) = self.hold_click(ui.ctx(), &r, "select_both");
                if click || long {
                    let paths = if long {
                        self.visible.to_vec()
                    } else {
                        self.page_paths().0
                    };
                    self.remember_selection();
                    self.active_selection().clear();
                    self.select_paths(paths);
                    let n = self.active_selection().len();
                    self.selection_feedback(
                        "select",
                        if long {
                            format!("Selected all {n} images")
                        } else {
                            format!("Selected {n} images on this page")
                        },
                    );
                }
            }
            Destination => {
                let dest = &self.lib.settings.destination;
                let tip = if dest.as_os_str().is_empty() {
                    "Destination: none (click: choose; hold: favorite folders)".to_owned()
                } else {
                    format!(
                        "Destination: {} (click: choose; hold: favorite folders)",
                        dest.display()
                    )
                };
                let open = self.popup.is_some_and(|(p, _)| p == Popup::Destination);
                let r = self.grid_button(
                    ui,
                    icons,
                    "destination",
                    Icon::Folder,
                    &tip,
                    open,
                    !self.busy,
                    false,
                );
                let rect = r.rect;
                let (click, long) = self.hold_click(ui.ctx(), &r, "destination");
                if click {
                    self.pick("destination");
                }
                if long {
                    self.open_popup(Popup::Destination, rect);
                }
            }
            KeepNames => {
                let keep = self.lib.settings.move_keep_names;
                if b(
                    self,
                    ui,
                    "keep_names",
                    Icon::KeepNames,
                    if keep {
                        "Keeping the original file names (click: sequence names)"
                    } else {
                        "Sequence names (click: keep the original file names)"
                    },
                    keep,
                    true,
                ) {
                    self.lib.settings.move_keep_names = !keep;
                    self.changed();
                }
            }
            CancelMarked => {
                if b(
                    self,
                    ui,
                    "cancel_marked",
                    Icon::CloseGrid,
                    "Cancel (back to the image; the marks stay)",
                    false,
                    true,
                ) {
                    self.exit_move_grid();
                }
            }
            Profile => {
                let open = self.popup.is_some_and(|(p, _)| p == Popup::Profiles);
                let r = self.grid_button(
                    ui,
                    icons,
                    "profile",
                    Icon::Profile,
                    "Profile",
                    open,
                    true,
                    false,
                );
                let rect = r.rect;
                if r.on_hover_text("Profiles").clicked() {
                    self.toggle_profiles_popup(rect);
                }
            }
            GoTo => {
                let ordered_pages = self.sequential_pages();
                let r = self.grid_button(
                    ui,
                    icons,
                    "goto",
                    Icon::GoTo,
                    "Page 1",
                    self.popup.is_some_and(|(p, _)| p == Popup::GoTo),
                    ordered_pages && self.visible.len() > count,
                    false,
                );
                let r = r.on_hover_text("Go to page 1; hold to type a page");
                let (click, long) = self.hold_click(ui.ctx(), &r, "goto");
                if click {
                    self.go_to(1);
                }
                if long {
                    self.open_popup(Popup::GoTo, r.rect);
                }
            }
            Undo => {
                let restorable = self.restorable();
                let r = self.grid_button(
                    ui,
                    icons,
                    "undo",
                    Icon::Undo,
                    &format!("Restore deleted ({restorable})"),
                    false,
                    restorable > 0 && !self.busy,
                    false,
                );
                if icons && restorable > 0 {
                    count_badge(ui, r.rect, restorable, MOVE_BLUE);
                }
                if r.clicked() {
                    self.request_restore();
                }
            }
            Tags => {
                let r = self.grid_button(
                    ui,
                    icons,
                    "tags",
                    Icon::Tag,
                    "Tags",
                    self.tag_mode,
                    selecting,
                    false,
                );
                // The tag palette opens centred on this button.
                self.tags_anchor = Some(r.rect.center().x);
                if r.clicked() {
                    self.toggle_grid_tags();
                }
            }
            SelectView => {
                if b(
                    self,
                    ui,
                    "select_view",
                    Icon::SelectPage,
                    "Select all (view)",
                    false,
                    selecting,
                ) {
                    let page = self.page_paths().0;
                    self.remember_selection();
                    self.active_selection().clear();
                    self.select_paths(page);
                    let n = self.active_selection().len();
                    self.selection_feedback(
                        "select_view",
                        format!("Selected {n} images on this page"),
                    );
                }
            }
            SelectFolder => {
                if b(
                    self,
                    ui,
                    "select_folder",
                    Icon::SelectAll,
                    "Select all (folder)",
                    false,
                    selecting,
                ) {
                    self.remember_selection();
                    self.active_selection().clear();
                    self.select_paths(self.visible.to_vec());
                    let n = self.active_selection().len();
                    self.selection_feedback(
                        "select_folder",
                        format!("Selected all {n} images in the folder"),
                    );
                }
            }
            Clear => {
                let any = !self.active_selection().is_empty();
                let response = self.grid_button(
                    ui,
                    icons,
                    "clear",
                    Icon::ClearSelection,
                    "Clear selection",
                    false,
                    selecting && any,
                    false,
                );
                let response = if icons {
                    response.on_hover_text("Hold for: this page only / the whole folder")
                } else {
                    response
                };
                self.clear_selection_hold(ui, &response, selecting && any);
            }
            Restore => {
                let restorable = selecting && self.last_selection.is_some();
                if b(
                    self,
                    ui,
                    "restore",
                    Icon::Restore,
                    "Restore selection",
                    false,
                    restorable,
                ) {
                    if let Some((trash, tags)) = self.last_selection.take() {
                        let n = trash.len() + tags.len();
                        self.delete_selection = trash
                            .into_iter()
                            .filter(|p| self.visible.position(p).is_some())
                            .collect();
                        self.tag_selection = tags
                            .into_iter()
                            .filter(|p| self.visible.position(p).is_some())
                            .collect();
                        if !self.tag_selection.is_empty() && !self.tag_mode {
                            self.tag_mode = true;
                            self.panel = Panel::Tags;
                        }
                        self.selection_feedback("restore", format!("Restored {n} selected images"));
                    }
                }
            }
            CloseGrid => {
                if b(
                    self,
                    ui,
                    "close_grid",
                    Icon::CloseGrid,
                    "Close grid",
                    false,
                    true,
                ) {
                    self.toggle_grid();
                }
            }
            Fullscreen => {
                if b(
                    self,
                    ui,
                    "fullscreen",
                    Icon::Fullscreen,
                    "Fullscreen",
                    self.fullscreen,
                    true,
                ) {
                    self.toggle_fullscreen(ui.ctx());
                }
            }
            Size => {
                vline(ui);
                if !icons {
                    ui.label("Images");
                }
                let round = self.round_full_setting();
                let available = self.visible.len();
                if let Some(area) = self.grid_area {
                    let text = if icons {
                        "Fewer images per page"
                    } else {
                        "−"
                    };
                    if b(self, ui, "fewer", Icon::Minus, text, false, count > 1) {
                        self.set_grid_count(crate::grid_layout::step(
                            count, false, round, available, area,
                        ));
                    }
                }
                let mut edited = count;
                let value =
                    egui::DragValue::new(&mut edited).range(1..=crate::grid_layout::MAX_COUNT);
                // Fixed width: the bar does not grow when 9 becomes 10.
                let response = if fixed {
                    ui.add_sized([40., button_height(ui)], value)
                } else {
                    ui.add(value)
                };
                self.hover.watch(response, false);
                if edited != count {
                    self.set_grid_count(edited);
                }
                if let Some(area) = self.grid_area {
                    let text = if icons { "More images per page" } else { "+" };
                    let more = count < crate::grid_layout::MAX_COUNT;
                    if b(self, ui, "more", Icon::Plus, text, false, more) {
                        self.set_grid_count(crate::grid_layout::step(
                            count, true, round, available, area,
                        ));
                    }
                }
                let presets = &self.lib.settings.grid_presets;
                let room = !presets.contains(&count) && presets.len() < 8;
                if icon(
                    self,
                    ui,
                    "add_size",
                    Icon::AddPageSize,
                    "Add a button for this page size",
                    room,
                ) {
                    let p = &mut self.lib.settings.grid_presets;
                    p.push(count);
                    p.sort_unstable();
                    self.changed();
                }
            }
            Presets => {
                // Quick page sizes; right-click removes one.
                for n in self.lib.settings.grid_presets.clone() {
                    let response = ui.add(egui::Button::new(n.to_string()).selected(n == count));
                    let response = self.hover.watch(response, false);
                    if response.clicked() {
                        self.set_grid_count(n);
                    }
                    if response.secondary_clicked() {
                        if self.lib.settings.confirm_preset_remove {
                            self.dialog = Some(Dialog::RemovePreset(n));
                        } else {
                            self.lib.settings.grid_presets.retain(|p| *p != n);
                            self.changed();
                        }
                    }
                    response.on_hover_text("Right-click to remove");
                }
            }
            Move | MoveMarked => {
                vline(ui);
                let movable = !self.visible.is_empty() && !self.busy && !self.view_pending;
                let marked = self.move_grid.is_some();
                let text = match (icons, self.move_mode) {
                    (_, true) => "Move the selected images (hold to stop selecting)",
                    (true, false) if marked => "Move all marked images (hold: Move selected)",
                    (true, false) => "Move all folder images (hold: Move selected)",
                    (false, false) => "Move all…",
                };
                // Move selected mode turns the whole button blue, like the
                // chosen tiles' borders.
                let response = if self.move_mode {
                    if icons {
                        self.hover.icon_response(
                            ui,
                            Icon::Move,
                            text,
                            true,
                            movable,
                            Some(MOVE_BLUE),
                            false,
                        )
                    } else {
                        let label = RichText::new("Move selected…").color(MOVE_BLUE);
                        let response =
                            ui.add_enabled(movable, egui::Button::new(label).selected(true));
                        self.hover.watch(response, false).on_hover_text(text)
                    }
                } else {
                    self.grid_button(ui, icons, "move", Icon::Move, text, false, movable, false)
                };
                if icons && self.move_mode && !self.move_selection.is_empty() {
                    count_badge(ui, response.rect, self.move_selection.len(), MOVE_BLUE);
                }
                self.move_button(ui, &response);
            }
            Filter => {
                let filtering = self.filter.active();
                let text = match (icons, filtering) {
                    (_, true) => "Filter on: click to clear it",
                    (true, false) => "Filter",
                    (false, false) => "Filter…",
                };
                let open = filtering || self.panel == Panel::Filter;
                if b(self, ui, "filter", Icon::Filter, text, open, true) {
                    self.filter_clicked();
                }
            }
            Folder => {
                let text = if icons {
                    "Open folder"
                } else {
                    "Open folder…"
                };
                let r = self.grid_button(
                    ui,
                    icons,
                    "folder",
                    Icon::Folder,
                    text,
                    self.popup.is_some_and(|(p, _)| p == Popup::Favorites),
                    !self.busy,
                    false,
                );
                let r = r.on_hover_text("Open a folder; hold for the favorite folders");
                self.folder_button(ui.ctx(), &r);
            }
            FloatingTrash => {
                let float = self.delete_float;
                if b(
                    self,
                    ui,
                    "floating_trash",
                    Icon::FloatingTrash,
                    "Floating trash button",
                    float,
                    true,
                ) {
                    self.delete_float = !float;
                }
            }
            Lock => {
                let locked = self.bar_locked();
                let (icon, text) = if locked {
                    (Icon::Lock, "Unlock the bar")
                } else {
                    (
                        Icon::Unlock,
                        "Lock the bar: it stays open at the bottom edge",
                    )
                };
                if b(self, ui, "lock", icon, text, locked, true) {
                    self.set_bar_locked(!locked);
                    self.changed();
                }
            }
            Status => {
                vline(ui);
                self.grid_status(ui, count, icons || fixed, fixed);
            }
        }
    }
    /// Page number, timings, background caching and the click hint. Compact:
    /// one short line (timings on hover); fixed: widths that do not change
    /// from page to page.
    fn grid_status(&mut self, ui: &mut egui::Ui, count: usize, compact: bool, fixed: bool) {
        let ordered = self.sequential_pages();
        let pages = self.visible.len().div_ceil(count).max(1);
        let suffix = if ordered {
            String::new()
        } else {
            format!(
                " ({})",
                page_order_label(self.lib.settings.grid_page_order).to_lowercase()
            )
        };
        // After a deletion a page may start between multiples of the size.
        let n = if ordered {
            (self.grid_start.div_ceil(count) + 1).min(pages)
        } else {
            // Random pages have no fixed place: count them round by round
            // through the folder's number of pages.
            (self.grid_pages.number() - 1) % pages + 1
        };
        // 01, 02, … (as many digits as the page count, at least two).
        let digits = pages.to_string().len().max(2);
        let font = egui::TextStyle::Body.resolve(ui.style());
        let fixed_label = |ui: &mut egui::Ui, text: String, widest: String| {
            let width = ui.fonts(|f| f.layout_no_wrap(widest, font.clone(), MUTED).size().x);
            let label = egui::Label::new(RichText::new(text).color(MUTED));
            if fixed {
                ui.allocate_ui_with_layout(
                    Vec2::new(width + 2., ui.spacing().interact_size.y),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        // Takes the room of the widest value, not its own.
                        ui.set_min_width(width + 2.);
                        ui.add(label)
                    },
                )
                .inner
            } else {
                ui.add(label)
            }
        };
        // As wide as the last page's number needs.
        let page = fixed_label(
            ui,
            format!("Page {n:0digits$} / {pages:0digits$}{suffix}"),
            // The most pages there can be (one image per page).
            {
                let most = self.visible.len().max(1);
                let digits = most.to_string().len().max(2);
                format!("Page {most:0digits$} / {most:0digits$}{suffix}")
            },
        );
        if compact {
            if let Some((_, _, Some((took, prepared)))) = self.grid_timer {
                let mut text = format!("Loaded in {} ms", took.as_millis());
                if let Some(prepared) = prepared {
                    text += &format!(" · prepared in {} ms", prepared.as_millis());
                }
                page.on_hover_text(text);
            }
        } else if let Some((_, _, Some((took, prepared)))) = self.grid_timer {
            ui.label(RichText::new(format!("· loaded in {} ms", took.as_millis())).color(MUTED))
                .on_hover_text("Time from page change until every thumbnail on the page was shown");
            if let Some(prepared) = prepared {
                ui.label(
                    RichText::new(format!("· prepared in {} ms", prepared.as_millis()))
                        .color(MUTED),
                )
                .on_hover_text("Time the workers spent preparing this page's thumbnails (reading, decoding, resizing, upload), also when the page was prepared in advance. Overlapping work is counted once.");
            }
        }
        if let Some(p) = self.images.pregen_progress() {
            if p.full {
                ui.label(RichText::new("· thumbnail cache full").color(MUTED))
                    .on_hover_text("Increase Settings → Library → Thumbnail disk cache to cache the whole folder");
            } else if p.done < p.total {
                // Progress advances in the background; refresh it gently.
                ui.ctx().request_repaint_after(Duration::from_millis(500));
                let paused = if crate::pregen::quiet() {
                    ""
                } else {
                    " (paused)"
                };
                let (text, widest) = if compact {
                    (
                        format!("· cache {}%{paused}", p.done * 100 / p.total.max(1)),
                        format!("· cache 100%{paused}"),
                    )
                } else {
                    (
                        format!("· caching thumbnails {} / {}{paused}", p.done, p.total),
                        format!("· caching thumbnails {} / {}{paused}", p.total, p.total),
                    )
                };
                fixed_label(ui, text, widest)
                    .on_hover_text("Aggressive image loading: background pass over the open folder. Pauses while you use NoirView, while the window is not active and during keyword indexing.");
            }
        }
        if !compact {
            let hint = if self.grid_normal {
                "· Click an image to view it"
            } else if self.tag_mode {
                "· Select images to tag"
            } else {
                "· Select images to delete"
            };
            fixed_label(ui, hint.to_owned(), "· Select images to delete".to_owned());
        }
    }
    /// Sort and page-order choices of the grid icon bar, above their icon.
    fn grid_menu_ui(&mut self, ctx: &egui::Context) {
        use crate::model::Order;
        let Some((menu, button)) = self.grid_menu else {
            return;
        };
        let mut done = false;
        let rect = above(
            ctx,
            Id::new("grid_bar_menu"),
            self.bar_anchor(button),
            |ui| {
                ui.horizontal(|ui| match menu {
                    GridMenu::Sort => {
                        // Stays open: a second click turns the direction.
                        if let Some((k, desc)) = self.sort_choices(ui) {
                            self.set_sort(k, desc);
                        }
                    }
                    GridMenu::Pages => {
                        let s = &self.lib.settings;
                        let current = (s.grid_page_order, s.grid_random_both);
                        for (o, no_history) in ORDER_CHOICES {
                            let active = o == current.0
                                && (o == Order::Sequential || no_history == current.1);
                            let label = order_choice_label(o, no_history);
                            if self.hover.button(ui, label, active, true, false) {
                                self.set_page_order(o);
                                if o != Order::Sequential {
                                    self.lib.settings.grid_random_both = no_history;
                                }
                                self.changed();
                                done = true;
                            }
                        }
                    }
                });
            },
        );
        let elsewhere = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| !rect.contains(p) && !button.contains(p))
        });
        if done || elsewhere {
            self.grid_menu = None;
        }
    }
    fn move_all_clicked(&mut self) {
        let s = &self.lib.settings;
        let plan = MovePlan::new(
            self.files.clone(),
            s.move_keep_names,
            false,
            &s.destination,
            (s.move_digits, s.move_fill_gaps),
        );
        self.dialog = Some(Dialog::Move(plan));
    }
    /// A click on the grid's Move button.
    fn move_clicked(&mut self) {
        if self.move_grid.is_some() && !(self.move_mode && !self.move_selection.is_empty()) {
            if self.move_mode {
                self.move_mode = false;
            } else {
                self.move_marked();
            }
        } else if !self.move_mode {
            self.move_all_clicked();
        } else if self.move_selection.is_empty() {
            self.move_mode = false;
        } else {
            let sources: Vec<PathBuf> = self
                .visible
                .iter()
                .filter(|p| self.move_selection.contains(*p))
                .cloned()
                .collect();
            let s = &self.lib.settings;
            let mut plan = MovePlan::new(
                sources,
                s.move_keep_names,
                true,
                &s.destination,
                (s.move_digits, s.move_fill_gaps),
            );
            plan.marked = self.move_grid.is_some();
            self.dialog = Some(Dialog::Move(plan));
        }
    }
    /// The grid's Move button: a click moves the folder (or, in Move selected
    /// mode, the chosen images); holding switches Move selected mode.
    fn move_button(&mut self, ui: &mut egui::Ui, response: &egui::Response) {
        let long = self.move_hold.is_some_and(|(_, done)| done);
        if response.clicked() && !long {
            self.move_clicked();
        }
        if response.is_pointer_button_down_on() {
            let hold = Duration::from_millis(self.lib.settings.grid_hold_ms);
            let (since, done) = self.move_hold.get_or_insert((Instant::now(), false));
            if !*done {
                let held = since.elapsed();
                if held >= hold {
                    *done = true;
                    self.toggle_move_mode();
                } else {
                    ui.ctx().request_repaint_after(hold - held);
                }
            }
        } else {
            self.move_hold = None;
        }
    }
    fn toggle_move_mode(&mut self) {
        self.move_mode = !self.move_mode;
        if self.move_mode {
            self.grid_normal = false;
            if self.tag_mode {
                self.toggle_grid_tags();
            }
        } else {
            self.move_selection.clear();
            self.marks_changed(false);
        }
    }
    /// The Filter control: clears an active filter, otherwise opens the
    /// filter panel.
    fn filter_clicked(&mut self) {
        if self.filter.active() {
            self.filter = Filter::default();
            self.panel = Panel::None;
            self.rebuild();
        } else {
            self.draft_filter = Filter::default();
            self.switch_panel(Panel::Filter);
        }
    }
    /// The toolbar's browsing order choice, above its icon.
    fn order_chooser(&mut self, ctx: &egui::Context) {
        use crate::model::Order;
        let Some(button) = self.order_menu.filter(|_| self.toolbar_shown()) else {
            self.order_menu = None;
            return;
        };
        let mut done = false;
        let rect = above(ctx, Id::new("order_chooser"), button.center_top(), |ui| {
            ui.horizontal(|ui| {
                let s = &self.lib.settings;
                let current = (s.order, s.random_wheel_both);
                for (o, no_history) in ORDER_CHOICES {
                    let active =
                        o == current.0 && (o == Order::Sequential || no_history == current.1);
                    let label = order_choice_label(o, no_history);
                    if self.hover.button(ui, label, active, true, false) {
                        self.lib.settings.order = o;
                        if o != Order::Sequential {
                            self.lib.settings.random_wheel_both = no_history;
                        }
                        self.sync_history_limit();
                        self.changed();
                        done = true;
                    }
                }
            });
        });
        let elsewhere = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| !rect.contains(p) && !button.contains(p))
        });
        if done || elsewhere {
            self.order_menu = None;
        }
    }
    fn set_page_order(&mut self, order: crate::model::Order) {
        if order != self.lib.settings.grid_page_order {
            self.lib.settings.grid_page_order = order;
            self.grid_ghosts = None;
            self.changed();
        }
    }
    /// Clear selection: a click clears everything; holding opens the
    /// choices (this page / the whole folder) above the button.
    fn clear_selection_hold(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        enabled: bool,
    ) {
        let long = self.clear_hold.is_some_and(|(_, opened)| opened);
        if response.clicked() && !long {
            self.clear_selection(false);
        }
        if response.is_pointer_button_down_on() {
            let hold = Duration::from_millis(self.lib.settings.grid_hold_ms);
            let (since, opened) = self.clear_hold.get_or_insert((Instant::now(), false));
            if !*opened {
                let held = since.elapsed();
                if held >= hold {
                    *opened = true;
                    self.clear_menu = Some(response.rect);
                } else {
                    ui.ctx().request_repaint_after(hold - held);
                }
            }
        } else {
            self.clear_hold = None;
        }
        if !enabled {
            self.clear_menu = None;
        }
        if let Some(button) = self.clear_menu {
            let mut chosen = None;
            let menu = above(
                ui.ctx(),
                Id::new("clear_selection_choices"),
                self.bar_anchor(button),
                |ui| {
                    ui.horizontal(|ui| {
                        if self
                            .hover
                            .button(ui, "Clear selected (view)", false, true, false)
                        {
                            chosen = Some(true);
                        }
                        if self
                            .hover
                            .button(ui, "Clear selected (folder)", false, true, false)
                        {
                            chosen = Some(false);
                        }
                    });
                },
            );
            // A press anywhere else closes the choices.
            let elsewhere = ui.ctx().input(|i| {
                i.pointer.any_pressed()
                    && i.pointer
                        .interact_pos()
                        .is_some_and(|p| !menu.contains(p) && !button.contains(p))
            });
            if let Some(view) = chosen {
                self.clear_selection(view);
            }
            if chosen.is_some() || elsewhere {
                self.clear_menu = None;
            }
        }
    }
    /// The grid's controls as an icon bar in the toolbar's style: always
    /// shown, or opened at the bottom edge like the toolbar.
    fn grid_icon_bar(&mut self, ctx: &egui::Context) {
        if !self.grid_icon_bar_shown() {
            return;
        }
        let s = self.lib.settings.clone();
        let id = Id::new("grid_icon_bar");
        let previous = ctx.memory(|m| m.area_rect(id));
        let height = previous.map_or(62., |r| r.height());
        let highest = (ctx.screen_rect().height() - height - 8.).max(0.);
        let bottom = s.grid_bar_bottom.clamp(0., highest);
        let width = (ctx.screen_rect().width() - 48.).max(300.);
        egui::Area::new(id)
            .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0., -bottom))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(
                        s.bar_color[0],
                        s.bar_color[1],
                        s.bar_color[2],
                        s.bar_color[3],
                    ))
                    .corner_radius(18.)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .stroke(Stroke::new(1.0_f32, Color32::from_white_alpha(22)))
                    .show(ui, |ui| {
                        ui.set_max_width(width);
                        let count = self.grid_count();
                        self.grid_controls(ui, count, true);
                    });
            });
    }
    fn on_grid_page(&self) -> bool {
        self.grid && self.grid_viewer.is_none()
    }
    /// Above a control of the grid's bars: over the bar's edge, not over the
    /// button inside it.
    fn bar_anchor(&self, button: Rect) -> Pos2 {
        let top = self
            .controls_rect
            .map_or(button.top(), |bar| bar.top().min(button.top()));
        Pos2::new(button.center().x, top)
    }
    fn grid_icon_bar_shown(&self) -> bool {
        self.on_grid_page() && self.grid_icons() && (self.grid_bar_fixed() || self.bar)
    }
    /// The floating toolbar is drawn: open, and not replaced by the grid's
    /// icon bar on the grid page.
    fn toolbar_shown(&self) -> bool {
        let s = &self.lib.settings;
        // The Move grid has only its own bar.
        let hidden = self.move_grid.is_some()
            || if s.grid_bar_icons {
                s.grid_hide_toolbar
            } else {
                s.grid_hide_toolbar_text
            };
        self.bar && !(self.on_grid_page() && hidden)
    }
    /// The top of the highest bar at the bottom (toolbar or grid icon bar),
    /// for placing panels above them.
    fn bars_top(&self, ctx: &egui::Context) -> Option<f32> {
        let rect = |name: &str| ctx.memory(|m| m.area_rect(Id::new(name)));
        let toolbar = rect("toolbar").filter(|_| self.toolbar_shown());
        let grid = rect("grid_icon_bar").filter(|_| self.grid_icon_bar_shown());
        // The text controls at the bottom of the grid page.
        let s = &self.lib.settings;
        let text = self
            .controls_rect
            .filter(|_| self.on_grid_page() && !self.grid_icons() && !s.grid_controls_top);
        [toolbar, grid, text]
            .into_iter()
            .flatten()
            .map(|r| r.top())
            .reduce(f32::min)
    }
    /// The toolbar of the big image opened from the grid: delete (always
    /// asks), tags, fullscreen, prompt and seed, lock.
    fn big_image_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let items = self.bar_items("big", &big_defaults());
        let (on_bar, grouped) = split_places(&items);
        for key in &on_bar {
            self.big_item(ui, ctx, key);
        }
        self.group_button(ui, "big", !grouped.is_empty());
    }
    /// One button of the big image's bar.
    fn big_item(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, key: &str) {
        let Some(path) = self.grid_viewer.clone() else {
            return;
        };
        let idle = !self.busy && !self.view_pending;
        match key {
            "trash" => {
                if self
                    .hover
                    .icon_response(
                        ui,
                        Icon::Trash,
                        "Move this image to Recycle Bin",
                        false,
                        idle,
                        None,
                        true,
                    )
                    .clicked()
                {
                    self.dialog = Some(Dialog::Delete(vec![path.clone()]));
                }
            }
            "tags" => {
                if self
                    .hover
                    .icon(ui, Icon::Tag, "Tags", self.panel == Panel::Tags, idle)
                {
                    self.switch_panel(Panel::Tags);
                }
            }
            "fullscreen" => {
                if self
                    .hover
                    .icon(ui, Icon::Fullscreen, "Fullscreen", self.fullscreen, true)
                {
                    self.toggle_fullscreen(ctx);
                }
            }
            "prompt" => {
                let open = self.prompt_window.is_some();
                if self
                    .hover
                    .icon(ui, Icon::Prompt, "Prompt and seed", open, true)
                {
                    self.prompt_window = if open { None } else { Some(path) };
                    self.prompt_follows = true;
                }
            }
            "image_only" => self.image_only_button(ui),
            "profile" => self.profile_button(ui, ctx, true),
            "lock" => self.lock_button(ui),
            _ => {}
        }
    }
    /// Prompt, negative prompt and seed of an image, each with a copy button.
    fn prompt_window(&mut self, ctx: &egui::Context) {
        let Some(path) = self.prompt_window.clone() else {
            return;
        };
        // Opened from the big image's bar: closes with the big image and
        // follows the image shown. From a menu: that image's, until closed.
        if self.prompt_follows && self.grid && self.grid_viewer.is_none() {
            self.prompt_window = None;
            return;
        }
        let path = if self.prompt_follows {
            self.current.clone().unwrap_or(path)
        } else {
            path
        };
        if self.generation.as_ref().is_none_or(|(p, _)| *p != path) {
            self.generation = Some((path.clone(), crate::comfy::read(&path)));
        }
        let generation = self.generation.as_ref().and_then(|(_, g)| g.clone());
        let mut open = true;
        let mut close = false;
        let mut copied = None;
        egui::Window::new("Prompt and seed")
            .open(&mut open)
            .default_width(520.)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(path.file_name().unwrap_or_default().to_string_lossy())
                        .color(MUTED),
                );
                let Some(g) = generation else {
                    ui.label("No prompt or seed found in this image.");
                    if self.hover.button(ui, "Close", false, true, false) {
                        close = true;
                    }
                    return;
                };
                egui::ScrollArea::vertical()
                    .max_height(480.)
                    .show(ui, |ui| {
                        for (title, text) in [
                            ("Positive prompt", g.positive.clone()),
                            ("Negative prompt", g.negative.clone()),
                        ] {
                            ui.allocate_ui_with_layout(
                                Vec2::new(ui.available_width(), button_height(ui)),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.strong(title);
                                    if self
                                        .hover
                                        .button(ui, "Copy", false, !text.is_empty(), false)
                                    {
                                        copied = Some((text.clone(), format!("{title} copied")));
                                    }
                                },
                            );
                            ui.label(if text.is_empty() {
                                "—".to_owned()
                            } else {
                                text
                            });
                            ui.separator();
                        }
                        ui.allocate_ui_with_layout(
                            Vec2::new(ui.available_width(), button_height(ui)),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.strong("Seed");
                                let seed = g.seed.clone().unwrap_or_default();
                                ui.label(if seed.is_empty() { "—" } else { &seed });
                                if self
                                    .hover
                                    .button(ui, "Copy", false, !seed.is_empty(), false)
                                {
                                    copied = Some((seed.clone(), format!("Seed {seed} copied")));
                                }
                            },
                        );
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if self
                        .hover
                        .button(ui, "Copy prompt(s) and seed", false, true, false)
                    {
                        copied = Some((g.clipboard(), "Prompt and seed copied".into()));
                    }
                    if self.hover.button(ui, "Close", false, true, false) {
                        close = true;
                    }
                });
            });
        // A copy closes the window.
        if let Some((text, note)) = copied {
            ctx.copy_text(text);
            self.notify(note);
            close = true;
        }
        if !open || close {
            self.prompt_window = None;
        }
    }
    /// Copy file: the file itself (to paste in Explorer or a message).
    fn copy_file(&mut self, path: &Path) {
        match crate::clipboard::copy_files(&[path.to_path_buf()]) {
            Ok(()) => self.notify("File copied"),
            Err(e) => self.notify(format!("Copy file failed: {e:#}")),
        }
    }
    /// Copy image: the pixels at full size (decoded in the background).
    fn copy_image(&mut self, path: &Path) {
        let tx = self.tx.clone();
        let path = path.to_path_buf();
        self.notify("Copying image…");
        std::thread::spawn(move || {
            let result = image::ImageReader::open(&path)
                .and_then(|r| r.with_guessed_format())
                .map_err(anyhow::Error::from)
                .and_then(|r| r.decode().map_err(anyhow::Error::from))
                .map(|img| {
                    let rgba = img.to_rgba8();
                    egui::ColorImage::from_rgba_unmultiplied(
                        [rgba.width() as usize, rgba.height() as usize],
                        rgba.as_raw(),
                    )
                })
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::CopyImage(result));
            tx.ctx.request_repaint();
        });
    }
    /// The toolbar's distance from the bottom: the grid page and the image
    /// views each keep their own.
    fn bar_bottom(&self) -> f32 {
        let s = &self.lib.settings;
        if self.grid && self.grid_viewer.is_none() {
            s.bar_bottom_grid()
        } else {
            s.bar_bottom
        }
    }
    /// The lock of the current view (or the universal lock).
    fn bar_locked(&self) -> bool {
        let s = &self.lib.settings;
        if !s.bar_lock_universal && self.grid && self.grid_viewer.is_none() {
            s.bar_locked_grid
        } else {
            s.bar_locked
        }
    }
    fn set_bar_locked(&mut self, locked: bool) {
        let grid = self.grid && self.grid_viewer.is_none();
        let s = &mut self.lib.settings;
        if !s.bar_lock_universal && grid {
            s.bar_locked_grid = locked;
        } else {
            s.bar_locked = locked;
        }
    }
    fn toolbar(&mut self, ctx: &egui::Context) {
        if !self.toolbar_shown() {
            self.group_open = None;
            return;
        }
        let s = self.lib.settings.clone();
        let id = Id::new("toolbar");
        let previous = ctx.memory(|m| m.area_rect(id));
        // Kept on screen whatever the saved distance and window size.
        let height = previous.map_or(s.bar_height, |r| r.height());
        let highest = (ctx.screen_rect().height() - height - 8.).max(0.);
        let mut bottom = self.bar_bottom();
        // Stacked above the grid's icon bar when both are shown.
        if let Some(grid) = ctx
            .memory(|m| m.area_rect(Id::new("grid_icon_bar")))
            .filter(|_| self.grid_icon_bar_shown())
        {
            bottom = bottom.max(ctx.screen_rect().bottom() - grid.top() + 8.);
        }
        let bottom = bottom.clamp(0., highest);
        egui::Area::new(id)
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                Vec2::new(s.bar_offset, -bottom),
            )
            .order(egui::Order::Middle) // windows (Settings) can come in front
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(
                        s.bar_color[0],
                        s.bar_color[1],
                        s.bar_color[2],
                        s.bar_color[3],
                    ))
                    .corner_radius(18.)
                    .inner_margin(12.)
                    .stroke(Stroke::new(1.0_f32, Color32::from_white_alpha(22)))
                    .show(ui, |ui| {
                        // The big image opened from the grid has its own,
                        // narrower bar.
                        if self.grid && self.grid_viewer.is_some() {
                            ui.horizontal(|ui| self.big_image_bar(ui, ctx));
                            return;
                        }
                        ui.set_min_width(
                            s.bar_width.min(ctx.screen_rect().width() - 48.).max(300.),
                        );
                        ui.set_max_width(
                            s.bar_width.min(ctx.screen_rect().width() - 48.).max(300.),
                        );
                        ui.set_min_height(s.bar_height.max(36.) - 24.);
                        let items = self.bar_items("toolbar", &tool_defaults());
                        let (on_bar, grouped) = split_places(&items);
                        ui.horizontal_wrapped(|ui| {
                            for (i, key) in on_bar.iter().enumerate() {
                                self.tool_item(ui, ctx, key);
                                // Image browsing apart from the rest.
                                if *key == "next" && i + 1 < on_bar.len() {
                                    ui.separator();
                                }
                            }
                            self.group_button(ui, "toolbar", !grouped.is_empty());
                        });
                    });
            });
        let big = self.grid && self.grid_viewer.is_some();
        match self.group_open {
            Some("toolbar") if !big => {
                let items = self.bar_items("toolbar", &tool_defaults());
                let (_, grouped) = split_places(&items);
                self.group_bar(ctx, id, |v, ui| {
                    for key in &grouped {
                        v.tool_item(ui, ctx, key);
                    }
                });
            }
            Some("big") if big => {
                let items = self.bar_items("big", &big_defaults());
                let (_, grouped) = split_places(&items);
                self.group_bar(ctx, id, |v, ui| {
                    for key in &grouped {
                        v.big_item(ui, ctx, key);
                    }
                });
            }
            // The other view's group bar.
            Some(_) => self.group_open = None,
            None => {}
        }
    }
    /// One button of the floating toolbar.
    fn tool_item(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, key: &str) {
        let s = self.lib.settings.clone();
        match key {
            "prev" => {
                if self.hover.icon(
                    ui,
                    Icon::Previous,
                    "Previous image",
                    false,
                    !self.visible.is_empty(),
                ) {
                    self.navigate(false);
                }
            }
            "next" => {
                if self.hover.icon(
                    ui,
                    Icon::Next,
                    "Next image",
                    false,
                    !self.visible.is_empty(),
                ) {
                    self.navigate(true);
                }
            }
            "tags" => {
                if self.hover.icon(
                    ui,
                    Icon::Tag,
                    "Tags",
                    self.panel == Panel::Tags,
                    self.current.is_some() || self.grid,
                ) {
                    if self.grid && self.grid_viewer.is_none() {
                        self.toggle_grid_tags();
                    } else {
                        self.switch_panel(Panel::Tags);
                    }
                }
            }
            "floating_trash" => {
                if self.hover.icon(
                    ui,
                    Icon::Trash,
                    "Show floating trash button",
                    self.delete_float,
                    self.current.is_some(),
                ) {
                    self.delete_float = !self.delete_float;
                }
            }
            "undo" => {
                let restorable = self.restorable();
                let r = self.hover.icon_response(
                    ui,
                    Icon::Undo,
                    "Restore the last deleted images from Recycle Bin (Ctrl+Z)",
                    false,
                    restorable > 0 && !self.busy,
                    None,
                    false,
                );
                if restorable > 0 {
                    count_badge(ui, r.rect, restorable, MOVE_BLUE);
                }
                if r.clicked() {
                    self.request_restore();
                }
            }
            "favorites" => {
                let left = ui.cursor().min.x;
                if self.hover.icon(
                    ui,
                    Icon::Heart,
                    "Favorites",
                    self.panel == Panel::Favorites,
                    self.current.is_some(),
                ) {
                    self.switch_panel(Panel::Favorites);
                }
                // Centre of the 38 px icon, for aligning the Favorites bar.
                self.favorite_anchor = Some(left + 19.);
            }
            "filter" => {
                let filtering = self.filter.active();
                if self.hover.icon(
                    ui,
                    Icon::Filter,
                    if filtering {
                        "Filter on: click to clear it"
                    } else {
                        "Filter images"
                    },
                    filtering,
                    true,
                ) {
                    self.filter_clicked();
                }
            }
            "order" => {
                let order = self.lib.settings.order;
                let response = self.hover.icon_response(
                    ui,
                    Icon::Order,
                    &format!("Browsing order: {}", order.label()),
                    self.order_menu.is_some(),
                    true,
                    None,
                    false,
                );
                if response.clicked() {
                    self.order_menu = match self.order_menu {
                        Some(_) => None,
                        None => Some(response.rect),
                    };
                }
            }
            "slideshow" => {
                if self.hover.icon(
                    ui,
                    Icon::Slideshow,
                    "Slideshow",
                    self.slideshow,
                    !self.visible.is_empty(),
                ) {
                    self.toggle_slideshow();
                }
            }
            "grid" => {
                if self.hover.icon(
                    ui,
                    Icon::Grid,
                    "Thumbnail grid",
                    self.grid,
                    !self.visible.is_empty(),
                ) {
                    self.toggle_grid();
                }
            }
            "music" => {
                if self.hover.icon(
                    ui,
                    Icon::Music,
                    if self.music.playing {
                        "Pause ♫"
                    } else {
                        "Play ♫"
                    },
                    self.music.playing,
                    !s.music.is_empty(),
                ) {
                    self.music.toggle();
                }
            }
            "fullscreen" => {
                if self.hover.icon(
                    ui,
                    Icon::Fullscreen,
                    "Toggle fullscreen",
                    self.fullscreen,
                    true,
                ) {
                    self.toggle_fullscreen(ctx);
                }
            }
            "image_only" => self.image_only_button(ui),
            "move" => {
                if self.hover.icon(
                    ui,
                    Icon::Move,
                    "Move all folder images",
                    false,
                    !self.files.is_empty() && !self.busy && !self.view_pending,
                ) {
                    self.move_all_clicked();
                }
            }
            "folder" => {
                let open = self.popup.is_some_and(|(p, _)| p == Popup::Favorites);
                let r = self.hover.icon_response(
                    ui,
                    Icon::Folder,
                    "Open folder (hold: favorite folders)",
                    open,
                    !self.busy,
                    None,
                    false,
                );
                self.folder_button(ctx, &r);
            }
            "round" => {
                let on = self.lib.settings.round_notice;
                let seen = self
                    .round_seen
                    .iter()
                    .filter(|p| self.visible.position(p).is_some())
                    .count();
                let text = format!(
                    "Notice when all images are shown (In order, Shuffle): {} · {seen} / {} seen",
                    if on { "on" } else { "off" },
                    self.visible.len()
                );
                if self.hover.icon(ui, Icon::Round, &text, on, true) {
                    self.lib.settings.round_notice = !on;
                    if on {
                        // Off: the count starts over.
                        self.round_seen.clear();
                        self.round_message = None;
                        self.round_fade_out = None;
                    }
                    self.changed();
                }
            }
            "move_one" => self.move_one_button(ui, ctx),
            "profile" => self.profile_button(ui, ctx, true),
            "goto" => {
                let open = self.popup.is_some_and(|(p, _)| p == Popup::GoTo);
                let r = self.hover.icon_response(
                    ui,
                    Icon::GoTo,
                    if self.grid && self.grid_viewer.is_none() {
                        "Go to page 1 (hold: type a page)"
                    } else {
                        "Go to image 1 (hold: type a number)"
                    },
                    open,
                    !self.visible.is_empty(),
                    None,
                    false,
                );
                let (click, long) = self.hold_click(ctx, &r, "goto");
                if click {
                    self.go_to(1);
                }
                if long {
                    self.open_popup(Popup::GoTo, r.rect);
                }
            }
            "settings" => {
                if self
                    .hover
                    .icon(ui, Icon::Settings, "Settings", self.settings, true)
                {
                    self.settings = !self.settings;
                }
            }
            "lock" => self.lock_button(ui),
            _ => {}
        }
    }
    /// The toolbar's lock (toolbar and the big image's bar).
    fn lock_button(&mut self, ui: &mut egui::Ui) {
        let locked = self.bar_locked();
        if self.hover.icon(
            ui,
            if locked { Icon::Lock } else { Icon::Unlock },
            if locked {
                "Unlock the toolbar"
            } else {
                "Lock the toolbar: it stays open and in place"
            },
            locked,
            true,
        ) {
            self.set_bar_locked(!locked);
            self.changed();
        }
    }
    /// Fullscreen shows only the image: on / off.
    fn image_only_button(&mut self, ui: &mut egui::Ui) {
        let on = self.lib.settings.fullscreen_image_only;
        if self.hover.icon(
            ui,
            Icon::ImageOnly,
            if on {
                "Fullscreen shows only the image: on"
            } else {
                "Fullscreen shows only the image: off"
            },
            on,
            true,
        ) {
            self.lib.settings.fullscreen_image_only = !on;
            self.changed();
        }
    }
    /// The » button that opens a bar's group bar (when it has buttons).
    fn group_button(&mut self, ui: &mut egui::Ui, bar: &'static str, any: bool) {
        if !any {
            if self.group_open == Some(bar) {
                self.group_open = None;
            }
            return;
        }
        let open = self.group_open == Some(bar);
        if self.hover.icon(ui, Icon::More, "More buttons", open, true) {
            self.group_open = if open { None } else { Some(bar) };
        }
    }
    /// A bar's group bar, just above the bar's right end (`under` is the
    /// bar's area).
    /// A press outside both closes it.
    fn group_bar(
        &mut self,
        ctx: &egui::Context,
        under: Id,
        add: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        let Some(bar) = ctx.memory(|m| m.area_rect(under)) else {
            return;
        };
        let s = self.lib.settings.clone();
        // At the » end of the bar, like the grid's overflow.
        let response = egui::Area::new(Id::new("group_bar"))
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .fixed_pos(bar.right_top() - Vec2::new(0., 6.))
            .constrain(true)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(
                        s.bar_color[0],
                        s.bar_color[1],
                        s.bar_color[2],
                        s.bar_color[3],
                    ))
                    .corner_radius(18.)
                    .inner_margin(12.)
                    .stroke(Stroke::new(1.0_f32, Color32::from_white_alpha(22)))
                    .show(ui, |ui| {
                        ui.set_max_width((bar.width() - 24.).max(200.));
                        ui.horizontal_wrapped(|ui| add(self, ui));
                    });
            })
            .response;
        let group = response.rect;
        // A press in a popup opened from the group (Profiles, Go to, …)
        // leaves the group to close with that popup.
        let popup = self.popup_rect.filter(|_| self.popup.is_some());
        let elsewhere = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer.interact_pos().is_some_and(|p| {
                    !group.contains(p) && !bar.contains(p) && !popup.is_some_and(|r| r.contains(p))
                })
        });
        if elsewhere {
            self.group_open = None;
        }
    }
    /// A bar's buttons in their order and places (Settings → Display → Bar
    /// buttons), leaving out those its settings turn off.
    fn bar_items(
        &self,
        bar: &str,
        defaults: &[(&'static str, crate::bar_layout::Place)],
    ) -> Vec<(&'static str, crate::bar_layout::Place)> {
        let s = &self.lib.settings;
        crate::bar_layout::arrange(defaults, s.bar_buttons.get(bar).map(Vec::as_slice))
            .into_iter()
            .filter(|(key, _)| !(*key == "favorites" && !s.show_favorites_button))
            .collect()
    }
    fn panels(&mut self, ctx: &egui::Context) {
        if self.panel == Panel::None && !self.tag_locked {
            self.panel_bottom = None;
            return;
        }
        // Above the bars at the bottom where they really are; when the bars
        // hide, the open panel stays where it was.
        let bottom = match self.bars_top(ctx) {
            Some(top) => {
                let bottom = ctx.screen_rect().bottom() - top + 4.;
                self.panel_bottom = Some(bottom);
                bottom
            }
            None => *self.panel_bottom.get_or_insert(self.bar_bottom() + 90.),
        };
        let width = self
            .lib
            .settings
            .bar_width
            .min(ctx.screen_rect().width() - 48.)
            .max(300.);
        let tag_width = (self
            .lib
            .settings
            .tag_width()
            .min(ctx.screen_rect().width() - 48.)
            - 28.)
            .max(1.);
        let available_tags_height = (ctx.screen_rect().height() - bottom - 76.).max(40.);
        let transient = self.panel;
        let extra = if transient != Panel::None {
            let mut x = self.lib.settings.bar_offset;
            // Favorites under the toolbar's heart; on the grid page the tag
            // palette under the grid's Tags button.
            let anchor = match self.panel {
                Panel::Favorites => self.favorite_anchor,
                Panel::Tags if self.on_grid_page() => self.tags_anchor,
                _ => None,
            };
            {
                if let Some(anchor) = anchor {
                    let screen = ctx.screen_rect();
                    let half = ctx
                        .data(|d| d.get_temp::<Vec2>(Id::new(("panel_size", "action"))))
                        .map_or(0., |size| size.x / 2.);
                    let limit = (screen.width() / 2. - half - 8.).max(0.);
                    x = (anchor - screen.center().x).clamp(-limit, limit);
                }
            }
            let response = egui::Area::new(Id::new("action_panel"))
                .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(x, -bottom))
                .order(egui::Order::Middle)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .corner_radius(14.)
                        .inner_margin(14.)
                        .show(ui, |ui| {
                            let panel_width = if matches!(self.panel, Panel::Tags | Panel::Filter) {
                                tag_width
                            } else {
                                width
                            };
                            ui.set_max_width(panel_width);
                            match self.panel {
                                Panel::Tags => self.tag_controls(ui, available_tags_height),
                                Panel::Favorites => {
                                    let mut choice = None;
                                    ui.horizontal(|ui| {
                                        let allow = !self.busy
                                            && !self.view_pending
                                            && !self.favorite_targets().is_empty();
                                        if self.hover.button(ui, "♥ Favorite", false, allow, false)
                                        {
                                            choice = Some(Some(false));
                                        }
                                        if self.hover.button(ui, "★ Elite", false, allow, false) {
                                            choice = Some(Some(true));
                                        }
                                        if self.hover.button(ui, "Cancel", false, true, false) {
                                            choice = Some(None);
                                        }
                                    });
                                    if let Some(choice) = choice {
                                        if let Some(elite) = choice {
                                            self.mark_favorite(elite);
                                        }
                                        self.panel = Panel::None;
                                    }
                                }
                                Panel::Filter => {
                                    ui.horizontal(|ui| {
                                        ui.label("Match tags");
                                        for (mode, label) in [
                                            (FilterMode::All, "All"),
                                            (FilterMode::Any, "Any"),
                                            (FilterMode::Exclude, "Exclude"),
                                        ] {
                                            if self.hover.button(
                                                ui,
                                                label,
                                                self.draft_filter.mode == mode,
                                                true,
                                                false,
                                            ) {
                                                self.draft_filter.mode = mode;
                                            }
                                        }
                                    });
                                    ui.push_id("filter_palette", |ui| {
                                        tag_picker_ui(
                                            ui,
                                            &mut self.hover,
                                            &mut self.filter_picker,
                                            &self.lib,
                                            &mut self.draft_filter.tags,
                                            TagView {
                                                width: tag_width,
                                                max_height: (available_tags_height
                                                    * if self.tag_locked { 0.5 } else { 1. }
                                                    - 120.)
                                                    .max(40.),
                                                controls: None,
                                                enabled: true,
                                            },
                                        );
                                    });
                                    ui.horizontal_wrapped(|ui| {
                                        let f = &mut self.draft_filter;
                                        if self.hover.button(
                                            ui,
                                            "♥ Favorite",
                                            f.favorite,
                                            true,
                                            false,
                                        ) {
                                            f.favorite = !f.favorite;
                                        }
                                        if self.hover.button(ui, "★ Elite", f.elite, true, false)
                                        {
                                            f.elite = !f.elite;
                                        }
                                        // The images marked to move (single
                                        // view), and in the grid those
                                        // chosen to move.
                                        if self.hover.button(ui, "✔ Marked", f.marked, true, false)
                                        {
                                            f.marked = !f.marked;
                                        }
                                        if self.grid
                                            && self.hover.button(
                                                ui,
                                                "➡ Selected to move",
                                                f.chosen,
                                                true,
                                                false,
                                            )
                                        {
                                            f.chosen = !f.chosen;
                                        }
                                        if self.hover.button(ui, "Apply filter", false, true, false)
                                        {
                                            self.filter = self.draft_filter.clone();
                                            self.rebuild();
                                            self.panel = Panel::None;
                                        }
                                        if self.hover.button(ui, "Cancel", false, true, false) {
                                            self.panel = Panel::None;
                                        }
                                    });
                                }
                                Panel::None => {}
                            }
                        });
                });
            settle_panel(ctx, "action", response.response.rect.size());
            response.response.rect.height() + 12.
        } else {
            0.
        };
        if self.tag_locked && transient != Panel::Tags {
            let response = egui::Area::new(Id::new("locked_tags"))
                .anchor(
                    egui::Align2::CENTER_BOTTOM,
                    Vec2::new(self.lib.settings.bar_offset, -bottom - extra),
                )
                .order(egui::Order::Middle)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .corner_radius(14.)
                        .inner_margin(14.)
                        .show(ui, |ui| {
                            ui.set_max_width(tag_width);
                            self.tag_controls(ui, (available_tags_height - extra).max(40.));
                        });
                });
            settle_panel(ctx, "locked_tags", response.response.rect.size());
        }
    }
    fn tag_controls(&mut self, ui: &mut egui::Ui, max_height: f32) {
        self.tag_picker
            .restrict_selection(&self.lib, &mut self.selected_tags);
        let width = (self
            .lib
            .settings
            .tag_width()
            .min(ui.ctx().screen_rect().width() - 48.)
            - 28.)
            .max(1.);
        let enabled = !self.busy && !self.view_pending;
        let controls = TagControls {
            favorites: !self.favorite_targets().is_empty(),
            additional: self.tag_picker.show_additional,
            locked: self.tag_locked,
            enabled,
        };
        let actions = ui
            .push_id("tag_palette", |ui| {
                tag_picker_ui(
                    ui,
                    &mut self.hover,
                    &mut self.tag_picker,
                    &self.lib,
                    &mut self.selected_tags,
                    TagView {
                        width,
                        max_height,
                        controls: Some(controls),
                        enabled,
                    },
                )
            })
            .inner;
        for action in actions {
            match action {
                TagAction::Create => {
                    self.new_tag_focus = !self.new_tag_open;
                    self.new_tag_open = true;
                }
                TagAction::Favorites => {
                    let anchor = ui
                        .ctx()
                        .data(|d| d.get_temp::<Pos2>(Id::new("tag_favorite_top")));
                    self.tag_favorite_popup = match self.tag_favorite_popup {
                        Some(_) => None,
                        None => anchor,
                    };
                }
                TagAction::Additional => {
                    self.tag_picker.show_additional = !self.tag_picker.show_additional;
                    self.tag_picker
                        .restrict_selection(&self.lib, &mut self.selected_tags);
                }
                TagAction::Lock => self.tag_locked = !self.tag_locked,
                TagAction::Remove => {
                    if !self.selected_tags.is_empty() {
                        self.dialog = Some(Dialog::RemoveTags(self.selected_tags.clone()));
                    }
                }
                TagAction::Save => {
                    if !self.selected_tags.is_empty() {
                        self.save_tags(None);
                    }
                }
            }
            ui.ctx().request_repaint();
        }
    }
    /// The toolbar's floating Move button. A click shows or hides the
    /// floating Move button; holding starts marking images to move (the
    /// floating button marks them). While marking, a click opens the Move
    /// dialog for the marked images and holding stops marking.
    fn move_one_button(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let tip = if self.marking {
            format!(
                "Move the {} marked images (hold: stop marking)",
                self.marked.len()
            )
        } else {
            "Show the floating Move button (hold: mark images to move)".to_owned()
        };
        let r = self.hover.icon_response(
            ui,
            Icon::FloatingMove,
            &tip,
            self.move_float || self.marking,
            self.current.is_some(),
            self.marking.then_some(MOVE_BLUE),
            false,
        );
        if self.marking && !self.marked.is_empty() {
            count_badge(ui, r.rect, self.marked.len(), MOVE_BLUE);
        }
        let (click, long) = self.hold_click(ctx, &r, "move_one");
        if long {
            if self.marking {
                self.stop_marking();
            } else {
                self.marking = true;
                self.move_float = true;
            }
        }
        if click {
            if self.marking {
                self.enter_move_grid();
            } else {
                self.move_float = !self.move_float;
            }
        }
    }
    fn stop_marking(&mut self) {
        self.marking = false;
        let had = !self.marked.is_empty();
        self.marked.clear();
        if had {
            self.marks_changed(true);
        }
    }
    /// The Move dialog for the image shown.
    fn open_one_move(&mut self, path: PathBuf) {
        let s = &self.lib.settings;
        let mut plan = MovePlan::new(
            vec![path],
            s.move_keep_names,
            true,
            &s.destination,
            (s.move_digits, s.move_fill_gaps),
        );
        plan.one = true;
        self.dialog = Some(Dialog::Move(plan));
    }
    /// The marked images' Move grid: a grid of only the marked images with
    /// Move, Destination, Keep names and Cancel on its icon bar; Cancel or
    /// a move goes back to the image view as it was.
    fn enter_move_grid(&mut self) {
        if self.marked.is_empty() {
            self.notify("Mark images with the floating Move button first");
            return;
        }
        if self.busy || self.view_pending {
            return;
        }
        self.move_grid = Some(MoveGridSaved {
            filter: self.filter.clone(),
            current: self.current.clone(),
            grid_normal: self.grid_normal,
        });
        self.filter = Filter {
            marked: true,
            ..Default::default()
        };
        self.lib.settings.enter_move_grid_settings();
        self.settings_move_target = false;
        self.changed();
        self.grid = true;
        self.grid_normal = true;
        self.grid_viewer = None;
        self.preview = None;
        self.grid_start = 0;
        self.move_grid_scroll = 0.;
        self.bar = false;
        self.panel = Panel::None;
        self.popup = None;
        self.wheel.reset();
        self.rebuild();
    }
    /// Back from the Move grid to the image view (its filter and image).
    fn exit_move_grid(&mut self) {
        let Some(saved) = self.move_grid.take() else {
            return;
        };
        self.lib.settings.leave_move_grid_settings();
        self.changed();
        self.filter = saved.filter;
        self.grid = false;
        self.grid_normal = saved.grid_normal;
        self.grid_viewer = None;
        self.preview = None;
        self.delete_selection.clear();
        self.tag_selection.clear();
        self.tag_mode = false;
        self.move_mode = false;
        self.move_selection.clear();
        if let Some(current) = saved.current.filter(|p| self.files.contains(p)) {
            self.current = Some(current);
        }
        if matches!(self.popup, Some((Popup::Destination, _))) {
            self.popup = None;
        }
        self.wheel.reset();
        self.rebuild();
    }
    /// The Move grid's ×: an image taken off the marked images.
    fn take_off_marked(&mut self, path: &Path) {
        self.marked.retain(|p| p != path);
        self.delete_selection.remove(path);
        self.marks_changed(true);
        if self.marked.is_empty() {
            self.notify("No marked images left");
            self.exit_move_grid();
        }
    }
    /// The Move grid's Move: all marked images (Move selected mode, as in
    /// the grid, moves the chosen ones).
    fn move_marked(&mut self) {
        let sources = self.visible.to_vec();
        if sources.is_empty() {
            return;
        }
        let s = &self.lib.settings;
        let mut plan = MovePlan::new(
            sources,
            s.move_keep_names,
            true,
            &s.destination,
            (s.move_digits, s.move_fill_gaps),
        );
        plan.marked = true;
        self.dialog = Some(Dialog::Move(plan));
    }
    /// Marks the image shown for moving, or takes the mark off.
    fn toggle_mark(&mut self, path: &Path) {
        if let Some(i) = self.marked.iter().position(|p| p == path) {
            self.marked.remove(i);
        } else {
            self.marked.push(path.to_path_buf());
        }
        self.marks_changed(true);
    }
    /// The floating Move button over the single image: moves the image shown
    /// (a Move dialog), or while marking marks it.
    fn floating_move(&mut self, ctx: &egui::Context) {
        if !self.move_float || self.grid || self.image_only() {
            return;
        }
        let Some(current) = self.current.clone() else {
            return;
        };
        let s = self.lib.settings.clone();
        let size = s.delete_size;
        let start = s
            .move_pos
            .map(|v| Pos2::new(v[0], v[1]))
            .unwrap_or_else(|| {
                Pos2::new(
                    ctx.screen_rect().right() - 2. * size - 90.,
                    ctx.screen_rect().top() + 110.,
                )
            });
        let area = egui::Area::new(Id::new("floating_move"))
            .default_pos(start)
            .movable(true)
            .constrain(true)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let margin = close_margin(s.close_size);
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::splat(size + margin * 2.), Sense::hover());
                let center = rect.center();
                let radius = size / 2.;
                let usable = !self.busy && !self.view_pending;
                let opacity = s.delete_opacity * if usable { 1. } else { 0.4 };
                let color = |c: [u8; 4]| {
                    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], (c[3] as f32 * opacity) as u8)
                };
                let marked = self.marked.contains(&current);
                // Marking: a ring in the marking colour, full when marked.
                ui.painter()
                    .circle_filled(center, radius, color(s.move_color));
                let ring = if self.marking {
                    Stroke::new(
                        if marked { 4.0_f32 } else { 2. },
                        MOVE_BLUE.gamma_multiply(opacity),
                    )
                } else {
                    Stroke::new(s.delete_border_width, MOVE_BLUE.gamma_multiply(opacity))
                };
                if self.marking || s.delete_border {
                    ui.painter().circle_stroke(center, radius, ring);
                }
                let response = ui.interact(
                    Rect::from_center_size(center, Vec2::splat(size * 0.7)),
                    Id::new("move_circle_button"),
                    Sense::click(),
                );
                let response = response.on_hover_text(match (self.marking, marked) {
                    (false, _) => "Move this image…",
                    (true, false) => "Mark this image to move",
                    (true, true) => "Take the mark off",
                });
                let response = if usable {
                    self.hover.watch_in(HoverArea::Move, response, false)
                } else {
                    response
                };
                let font = (size * 0.16).clamp(10., 28.);
                let lines = if self.marking {
                    vec![
                        (
                            if marked { "MARKED" } else { "MARK" }.to_owned(),
                            -font * 0.55,
                        ),
                        (self.marked.len().to_string(), font * 0.65),
                    ]
                } else {
                    vec![("MOVE".to_owned(), 0.)]
                };
                for (text, dy) in lines {
                    ui.painter().text(
                        center + Vec2::new(0., dy),
                        egui::Align2::CENTER_CENTER,
                        text,
                        egui::FontId::proportional(font),
                        Color32::from_white_alpha((255. * opacity) as u8),
                    );
                }
                if response.clicked() && usable {
                    if self.marking {
                        self.toggle_mark(&current);
                    } else {
                        self.open_one_move(current.clone());
                    }
                }
                let close = close_center(center, radius, s.close_size, s.close_angle);
                let rect = Rect::from_center_size(close, Vec2::splat(s.close_size));
                let r = self.hover.watch_in(
                    HoverArea::Move,
                    ui.interact(rect, Id::new("move_close"), Sense::click())
                        .on_hover_text("Hide the floating Move button"),
                    false,
                );
                ui.painter().circle_filled(
                    close,
                    s.close_size / 2.,
                    Color32::from_rgba_unmultiplied(32, 37, 46, (255. * s.delete_opacity) as u8),
                );
                ui.painter().circle_stroke(
                    close,
                    s.close_size / 2.,
                    Stroke::new(1.0_f32, color(s.delete_border_color)),
                );
                ui.painter().text(
                    close,
                    egui::Align2::CENTER_CENTER,
                    "×",
                    egui::FontId::proportional(s.close_size * 0.7),
                    Color32::from_white_alpha((255. * s.delete_opacity) as u8),
                );
                if r.clicked() {
                    self.move_float = false;
                }
            });
        let p = area.response.rect.min;
        if self.lib.settings.move_pos != Some([p.x, p.y]) {
            self.lib.settings.move_pos = Some([p.x, p.y]);
            self.changed();
        }
    }
    /// The Move grid open: a frame round the whole window (option).
    fn move_grid_frame(&self, ctx: &egui::Context) {
        let s = &self.lib.settings;
        if self.move_grid.is_none() || !s.move_grid_frame {
            return;
        }
        let width = s.move_grid_frame_width.clamp(1., 24.);
        let c = s.move_grid_frame_color;
        let alpha = (c[3] as f32 * s.move_grid_frame_opacity.clamp(0.05, 1.)) as u8;
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            Id::new("move_grid_frame"),
        ))
        .rect_stroke(
            ctx.screen_rect(),
            0.,
            Stroke::new(
                width,
                Color32::from_rgba_unmultiplied(c[0], c[1], c[2], alpha),
            ),
            egui::StrokeKind::Inside,
        );
    }
    /// What the floating trash button moves to the Recycle Bin: the grid's
    /// selection (like the grid's own Trash button), the image shown big from
    /// the grid, or the current image. Empty means the button does nothing.
    fn floating_delete_targets(&self) -> Vec<PathBuf> {
        if self.grid && self.grid_viewer.is_none() {
            if self.grid_normal {
                return vec![];
            }
            let mut paths: Vec<PathBuf> = self.delete_selection.iter().cloned().collect();
            paths.sort_by_key(|p| (self.visible.position(p).unwrap_or(usize::MAX), p.clone()));
            return paths;
        }
        if self.grid {
            return self.grid_viewer.clone().into_iter().collect();
        }
        self.current.clone().into_iter().collect()
    }
    fn floating_delete(&mut self, ctx: &egui::Context) {
        if !self.delete_float {
            return;
        }
        let s = self.lib.settings.clone();
        let size = s.delete_size;
        let start = s
            .delete_pos
            .map(|v| Pos2::new(v[0], v[1]))
            .unwrap_or_else(|| {
                Pos2::new(
                    ctx.screen_rect().right() - size - 45.,
                    ctx.screen_rect().top() + 110.,
                )
            });
        let area = egui::Area::new(Id::new(("floating_trash", self.delete_area_revision)))
            .default_pos(start)
            .movable(true)
            .constrain(true)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let margin = close_margin(s.close_size);
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::splat(size + margin * 2.), Sense::hover());
                let center = rect.center();
                let radius = size / 2.;
                let targets = self.floating_delete_targets();
                let in_grid = self.grid && self.grid_viewer.is_none();
                let usable = !targets.is_empty() && !self.busy && !self.view_pending;
                // Greyed out while there is nothing to delete.
                let opacity = s.delete_opacity * if usable { 1. } else { 0.4 };
                let color = |c: [u8; 4]| {
                    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], (c[3] as f32 * opacity) as u8)
                };
                ui.painter()
                    .circle_filled(center, radius, color(s.delete_color));
                if s.delete_border {
                    ui.painter().circle_stroke(
                        center,
                        radius,
                        Stroke::new(s.delete_border_width, color(s.delete_border_color)),
                    );
                }
                let response = ui.interact(
                    Rect::from_center_size(center, Vec2::splat(size * 0.7)),
                    Id::new("trash_circle_button"),
                    Sense::click(),
                );
                let response = if usable {
                    response
                } else {
                    response.on_hover_text(if in_grid {
                        "Select images to delete first"
                    } else {
                        "No image to delete"
                    })
                };
                let response = if usable {
                    self.hover.watch_in(HoverArea::Trash, response, true)
                } else {
                    response
                };
                let font = (size * 0.16).clamp(10., 28.);
                // In the grid the number of selected images goes below.
                let lines = if in_grid {
                    vec![
                        ("TRASH".to_owned(), -font * 0.55),
                        (targets.len().to_string(), font * 0.65),
                    ]
                } else {
                    vec![("TRASH".to_owned(), 0.)]
                };
                for (text, dy) in lines {
                    ui.painter().text(
                        center + Vec2::new(0., dy),
                        egui::Align2::CENTER_CENTER,
                        text,
                        egui::FontId::proportional(font),
                        Color32::from_white_alpha((255. * opacity) as u8),
                    );
                }
                if response.clicked() && usable {
                    self.request_delete(targets);
                }
                let close = close_center(center, radius, s.close_size, s.close_angle);
                let rect = Rect::from_center_size(close, Vec2::splat(s.close_size));
                let r = self.hover.watch_in(
                    HoverArea::Trash,
                    ui.interact(rect, Id::new("trash_close"), Sense::click()),
                    false,
                );
                ui.painter().circle_filled(
                    close,
                    s.close_size / 2.,
                    Color32::from_rgba_unmultiplied(32, 37, 46, (255. * s.delete_opacity) as u8),
                );
                ui.painter().circle_stroke(
                    close,
                    s.close_size / 2.,
                    Stroke::new(1.0_f32, color(s.delete_border_color)),
                );
                ui.painter().text(
                    close,
                    egui::Align2::CENTER_CENTER,
                    "×",
                    egui::FontId::proportional(s.close_size * 0.7),
                    Color32::from_white_alpha((255. * s.delete_opacity) as u8),
                );
                if r.clicked() {
                    self.delete_float = false;
                }
                // Restore deleted beside the ×, one button further round.
                let ring = radius + s.close_size / 2. + CLOSE_GAP;
                let step = 2. * ((s.close_size + 4.) / (2. * ring)).clamp(0., 1.).asin();
                let undo = close_center(
                    center,
                    radius,
                    s.close_size,
                    s.close_angle - step.to_degrees(),
                );
                let restorable = self.restorable();
                let can = restorable > 0 && !self.busy && !self.view_pending;
                let r = ui
                    .interact(
                        Rect::from_center_size(undo, Vec2::splat(s.close_size)),
                        Id::new("trash_undo"),
                        Sense::click(),
                    )
                    .on_hover_text(if can {
                        format!("Restore deleted ({restorable})")
                    } else {
                        "Nothing to restore".to_owned()
                    });
                let r = self.hover.watch_in(HoverArea::Trash, r, false);
                let fade = if can { 1. } else { 0.4 };
                ui.painter().circle_filled(
                    undo,
                    s.close_size / 2.,
                    Color32::from_rgba_unmultiplied(32, 37, 46, (255. * s.delete_opacity) as u8),
                );
                ui.painter().circle_stroke(
                    undo,
                    s.close_size / 2.,
                    Stroke::new(1.0_f32, color(s.delete_border_color)),
                );
                // An undo arrow (the monospace font has ↶).
                ui.painter().text(
                    undo,
                    egui::Align2::CENTER_CENTER,
                    "↶",
                    egui::FontId::monospace(s.close_size * 0.8),
                    Color32::from_white_alpha((255. * s.delete_opacity * fade) as u8),
                );
                if r.clicked() && can {
                    self.request_restore();
                }
            });
        let p = area.response.rect.min;
        if self.lib.settings.delete_pos != Some([p.x, p.y]) {
            self.lib.settings.delete_pos = Some([p.x, p.y]);
            self.changed();
        }
    }
    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings {
            self.settings_snapshot = None;
            self.edit_grid_settings_of(false);
            return;
        }
        // What Cancel goes back to.
        if self.settings_snapshot.is_none() {
            self.settings_snapshot = Some(Box::new(self.lib.settings.clone()));
        }
        let before = serde_json::to_vec(&self.lib.settings).unwrap_or_default();
        let sort_before = (
            self.lib.settings.sort_key,
            self.lib.settings.sort_descending,
        );
        let mut open = true;
        let mut invalidate = false;
        let mut actions = vec![];
        egui::Window::new("Settings")
            .title_bar(false)
            .open(&mut open)
            .default_width(750.)
            .default_height(610.)
            .resizable(true)
            .show(ctx, |ui| {
                self.settings_contents(ui, &mut invalidate, &mut actions)
            });
        self.settings = open;
        self.sync_history_limit();
        if before != serde_json::to_vec(&self.lib.settings).unwrap_or_default() {
            self.changed();
        }
        if invalidate {
            self.images.invalidate();
        }
        if sort_before
            != (
                self.lib.settings.sort_key,
                self.lib.settings.sort_descending,
            )
        {
            self.sort_files();
            if !self.lib.settings.sort_key.needs_metadata() {
                self.rebuild();
            }
        }
        for action in actions {
            match action {
                "done" => self.settings = false,
                "cancel" => self.cancel_settings(),
                "rescan" => self.dialog = Some(Dialog::Rescan),
                "reindex" => self.dialog = Some(Dialog::Reindex),
                "usn_install" | "usn_remove" | "usn_test" => self.usn_action(action),
                "usn_refresh" => self.usn_refresh(),
                "watch_check" => {
                    if let Some(handle) = &self.watch.handle {
                        handle.check_now();
                    }
                }
                "clear_thumbnails" => self.dialog = Some(Dialog::ClearThumbnails),
                "recover" => {
                    self.dialog = Some(Dialog::Recover(self.lib.settings.destination.clone()))
                }
                "backup_now" => self.backup(None),
                other => self.pick(other),
            }
        }
    }
    fn settings_contents(
        &mut self,
        ui: &mut egui::Ui,
        invalidate: &mut bool,
        actions: &mut Vec<&'static str>,
    ) {
        // Pages in a column at the left, the page beside it, Done / Cancel
        // and the profile in use below.
        // Room for the separator and the buttons: content taller than the
        // window would make the window grow every frame.
        let footer = 64.;
        let height = (ui.available_height() - footer).max(160.);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(170., height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(170.);
                    ui.heading("Settings");
                    ui.add_space(4.);
                    ui.horizontal(|ui| {
                        let search = ui.add(
                            egui::TextEdit::singleline(&mut self.settings_query)
                                .hint_text("Search settings")
                                .desired_width(134.),
                        );
                        let search_id = search.id;
                        self.hover.watch(search, false);
                        // × empties the search and leaves the cursor in the field.
                        let clear = ui
                            .add_enabled(
                                !self.settings_query.is_empty(),
                                egui::Button::new("×").frame(false),
                            )
                            .on_hover_text("Clear the search");
                        if self.hover.watch(clear, false).clicked() {
                            self.settings_query.clear();
                            ui.memory_mut(|m| m.request_focus(search_id));
                        }
                    });
                    let mut loose = self.lib.settings.settings_search_loose;
                    let r = ui.checkbox(&mut loose, RichText::new("Looser matching").size(12.));
                    let r = r.on_hover_text("Also finds words with a letter wrong, missing or extra, and the start of a word");
                    self.hover.watch(r, false);
                    if loose != self.lib.settings.settings_search_loose {
                        self.lib.settings.settings_search_loose = loose;
                    }
                    ui.add_space(6.);
                    // Searching: how many controls of each page match.
                    let counts = self.settings_search_counts(ui);
                    let none = !counts.is_empty() && counts.iter().all(|n| *n == 0);
                    // A new search: to the first page with matches when the
                    // page shown has none.
                    let query = self.settings_query.trim().to_lowercase();
                    if self.settings_last_query != query {
                        self.settings_last_query = query;
                        if counts.get(self.settings_tab) == Some(&0) {
                            if let Some(first) = counts.iter().position(|n| *n > 0) {
                                self.settings_tab = first;
                            }
                        }
                    }
                    for (i, name) in SETTINGS_PAGES.iter().enumerate() {
                        // Nothing found: no page is chosen.
                        let chosen = self.settings_tab == i && !none;
                        let label = match counts.get(i) {
                            Some(&n) if n > 0 => format!("{name}  ({n})"),
                            _ => (*name).to_owned(),
                        };
                        let text = RichText::new(label).size(14.);
                        let text = if chosen {
                            text.color(GREEN).strong()
                        } else if counts.get(i) == Some(&0) {
                            text.color(MUTED.gamma_multiply(0.6))
                        } else {
                            text
                        };
                        let r = ui.add_sized(
                            [160., 26.],
                            egui::Button::new(text).selected(chosen).frame(chosen),
                        );
                        if self.hover.watch(r, false).clicked() {
                            self.settings_tab = i;
                        }
                    }
                },
            );
            // A line between the pages and the page, as tall as they are (a
            // separator here would take all the height there is and grow
            // the window every frame).
            let (line, _) = ui.allocate_exact_size(Vec2::new(9., height), Sense::hover());
            ui.painter().vline(
                line.center().x,
                line.y_range(),
                ui.visuals().widgets.noninteractive.bg_stroke,
            );
            ui.vertical(|ui| {
                // Fills the window's height (the window can be made taller);
                // the room for the buttons below is kept, or the window
                // would grow every frame.
                egui::ScrollArea::vertical()
                    .id_salt(("settings_page", self.settings_tab))
                    .auto_shrink([false, false])
                    .max_height(height)
                    .show(ui, |ui| {
                        self.hover.search = self.settings_query.trim().to_lowercase();
                        self.hover.loose = self.lib.settings.settings_search_loose;
                        if self.settings_nothing_found() {
                            // Nothing on any page: say so instead of a page.
                            ui.add_space(40.);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "No settings match \"{}\"",
                                        self.settings_query.trim()
                                    ))
                                    .size(18.)
                                    .color(MUTED),
                                );
                                ui.add_space(6.);
                                ui.label(
                                    RichText::new(if self.lib.settings.settings_search_loose {
                                        "Try fewer or other words."
                                    } else {
                                        "Try fewer or other words, or Looser matching."
                                    })
                                    .color(MUTED),
                                );
                            });
                            self.hover.search.clear();
                            return;
                        }
                        self.hover.first_found = None;
                        self.settings_controls(ui, self.settings_tab, invalidate, actions);
                        // A new search or page: the first match in view.
                        let key = (self.hover.search.clone(), self.settings_tab);
                        if let Some(rect) = self.hover.first_found.take() {
                            if self.settings_scrolled.as_ref() != Some(&key) {
                                ui.scroll_to_rect(rect, Some(egui::Align::Center));
                                self.settings_scrolled = Some(key);
                            }
                        }
                        self.hover.search.clear();
                        self.hover.found.clear();
                    });
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            if self.hover.button(ui, "Done", false, true, false) {
                actions.push("done");
            }
            let cancel = ui
                .add(egui::Button::new("Cancel"))
                .on_hover_text("Undo the changes made since Settings was opened");
            if self.hover.watch(cancel, false).clicked() {
                actions.push("cancel");
            }
            ui.label(RichText::new("Saved automatically.").color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // The profile in use; a click shows Profiles & backup.
                let active = self.lib.settings.active_profile.clone();
                let differences = self.lib.settings.profile_differences();
                let text = match (active.is_empty(), differences) {
                    (true, _) => "Profile: none".to_owned(),
                    (false, 0) => format!("Profile: {active}"),
                    (false, n) => format!("Profile: {active} — {n} changed"),
                };
                let r = plain_text_link(ui, text, false, true).on_hover_text("Profiles");
                if self.hover.watch(r, false).clicked() {
                    self.settings_tab = SETTINGS_PROFILES;
                }
            });
        });
    }
    /// The Grid page: whose grid settings it shows, the grid's or the Move
    /// grid's (marked images).
    fn grid_target_switch(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Settings of");
            let open = self.move_grid.is_some();
            let moving = open || self.settings_move_target;
            if self.hover.button(ui, "Grid", !moving, !open, false) {
                self.edit_grid_settings_of(false);
            }
            if self
                .hover
                .button(ui, "Move grid (marked images)", moving, !open, false)
            {
                self.edit_grid_settings_of(true);
            }
        });
        if self.move_grid.is_some() || self.settings_move_target {
            ui.label(
                RichText::new("These are the Move grid's own grid settings; the grid keeps its own. The Move grid's other settings are under Move & delete.")
                    .color(MOVE_BLUE),
            );
        }
    }
    /// Profiles & backup: the profile in use and its buttons.
    fn profile_page(&mut self, ui: &mut egui::Ui) {
        self.profile_row(ui);
        ui.label(RichText::new("A profile keeps the display and interaction settings under a name. Folders, watching, caches, backups, the graphics backend and diagnostics stay as they are. The bars' Profile button and the right-click menu switch profiles too.").color(MUTED));
    }
    /// Settings' search: each page drawn unseen (nothing can be clicked)
    /// to count the controls whose label has the words. Empty: not
    /// searching.
    fn settings_search_counts(&mut self, ui: &mut egui::Ui) -> Vec<usize> {
        let query = self.settings_query.trim().to_lowercase();
        if query.is_empty() {
            self.settings_counts.clear();
            return vec![];
        }
        let scroll = self.settings_scroll;
        let mut counts = vec![];
        for page in 0..SETTINGS_PAGES.len() {
            self.hover.search = query.clone();
            self.hover.loose = self.lib.settings.settings_search_loose;
            self.hover.found.clear();
            let area = Rect::from_min_size(Pos2::new(-20000., -20000.), Vec2::new(600., 20000.));
            let mut unseen = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("settings_search", page))
                    .max_rect(area)
                    .invisible(),
            );
            // Only counting: anything the unseen pages would ask for is
            // dropped.
            self.settings_controls(&mut unseen, page, &mut false, &mut vec![]);
            counts.push(self.hover.found.len());
            self.hover.first_found = None;
        }
        self.hover.search.clear();
        self.hover.found.clear();
        self.settings_scroll = scroll;
        self.settings_counts = counts.clone();
        counts
    }
    /// A search that nothing on any page matches.
    fn settings_nothing_found(&self) -> bool {
        !self.settings_query.trim().is_empty()
            && !self.settings_counts.is_empty()
            && self.settings_counts.iter().all(|n| *n == 0)
    }
    fn settings_controls(
        &mut self,
        ui: &mut egui::Ui,
        page: usize,
        invalidate: &mut bool,
        actions: &mut Vec<&'static str>,
    ) {
        let page = page.min(SETTINGS_PAGES.len() - 1);
        ui.horizontal(|ui| {
            ui.heading(SETTINGS_PAGES[page]);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.hover.button(
                    ui,
                    "Reset this page…",
                    false,
                    !SETTINGS_RESET[page].is_empty(),
                    false,
                ) {
                    self.dialog = Some(Dialog::ResetSettingsPage(page));
                }
            });
        });
        // Calls needing all of `self` come before the settings are borrowed.
        match page {
            2 => self.grid_target_switch(ui),
            SETTINGS_PROFILES => self.profile_page(ui),
            _ => {}
        }
        let s = &mut self.lib.settings;
        let h = &mut self.hover;
        match page {
            0 => {
                h.section(ui, "Image");
                h.note(ui, "Image sizing");
                ui.horizontal_wrapped(|ui| {
                    for mode in [DisplayMode::Fit, DisplayMode::Fill, DisplayMode::ShrinkOnly] {
                        if h.button(ui, mode.label(), s.view_mode() == mode, true, false) {
                            s.display_mode = Some(mode);
                        }
                    }
                });
                h.note(ui, "Fit shows the whole image. Fill covers the window and crops the edges, keeping the aspect ratio. Shrink only does not enlarge small images.");
                h.check(
                    ui,
                    &mut s.animate_gif,
                    "Play animated GIFs (single image view and the big image from the grid)",
                );
                ui.add_enabled_ui(s.animate_gif, |ui| {
                    h.slider(
                        ui,
                        &mut s.animation_mb,
                        64..=8192,
                        "Animated GIF memory limit (MB)",
                    );
                });
                h.note(ui, RichText::new("Frames are kept decoded in video memory while the GIF is shown. A larger animation shows its first frame; grid thumbnails always show the first frame.").color(MUTED));
                h.section(ui, "Magnifier");
                h.check(
                    ui,
                    &mut s.magnifier,
                    "Hold the left mouse button for a magnifying glass",
                );
                h.slider(ui, &mut s.lens_px, 80.0..=700.0, "Lens diameter (px)");
                h.slider(ui, &mut s.lens_zoom, 1.1..=12.0, "Lens magnification");
                h.check(ui, &mut s.lens_border, "Magnifier border");
                h.check(ui, &mut s.lens_soft_edges, "Soft magnifier edges");
                h.slider(ui, &mut s.lens_feather_px, 1.0..=60.0, "Edge softness (px)");
                h.check(ui, &mut s.lens_full_window, "Full-window zoom");
                h.note(ui, "Hold and move the left mouse button to magnify. Full-window zoom follows the pointer across the image viewing area; release to restore the view.");
                h.section(ui, "While an image loads");
                h.check(
                    ui,
                    &mut s.jpeg_preview,
                    "Show a quick preview while the image loads",
                );
                h.note(ui, "Uses cached thumbnails, embedded JPEG previews or reduced JPEG decoding. Magnification waits for full detail.");
                h.check(
                    ui,
                    &mut s.keep_previous_image,
                    "Keep the previous image on screen until the next one is ready",
                );
                h.check(
                    ui,
                    &mut s.fullscreen_image_only,
                    "Fullscreen shows only the image (no top bar, image number or notices)",
                );
                h.section(ui, "Title");
                h.check(
                    ui,
                    &mut s.image_title_folder,
                    "Image view title: the folder before the image name",
                );
                h.check(
                    ui,
                    &mut s.path_forward_slashes,
                    "Show folder paths with forward slashes (D:/Photos)",
                );
                h.slider(
                    ui,
                    &mut s.title_raise,
                    -6.0..=6.0,
                    "Raise the title beside NOIRVIEW (px, 0 = same baseline)",
                );
            }
            1 => {
                h.section(ui, "Order");
                order_controls(ui, h, &mut s.order, "browse_order");
                ui.horizontal_wrapped(|ui| {
                    h.note(ui, "Sort images by");
                    for k in crate::sorting::SortKey::ALL {
                        if h.button(ui, k.label(), s.sort_key == k, true, false) {
                            s.sort_key = k;
                        }
                    }
                    if h.button(ui, "Descending", s.sort_descending, true, false) {
                        s.sort_descending = !s.sort_descending;
                    }
                });
                h.section(ui, "Mouse wheel");
                h.check(ui, &mut s.wheel_up_next, "Scroll up for next image");
                h.check(
                    ui,
                    &mut s.random_wheel_both,
                    "Random / Shuffle order: scrolling either way shows a new image (no history)",
                );
                h.note(
                    ui,
                    "On: scroll up for Next, down for Previous. Off reverses the wheel direction.",
                );
                h.section(ui, "History");
                h.slider(
                    ui,
                    &mut s.history_limit,
                    2..=100_000,
                    "History length (visits)",
                );
                h.note(ui, format!("Recent trail: {} / {} visits. Includes the current image and repeated visits. Distant visits are forgotten at the limit.",self.nav.history_len(),self.nav.history_limit()));
                h.note(ui, "Changing the browsing order starts a new trail at the current image. Within one order, Previous follows the recorded trail.");
                h.check(
                    ui,
                    &mut s.keep_selected_tags,
                    "Keep selected tags when the image changes (with tag lock)",
                );
                h.section(ui, "Round notice");
                h.check(
                    ui,
                    &mut s.round_notice,
                    "Tell me when all images have been shown (In order, Shuffle)",
                );
                ui.add_enabled_ui(s.round_notice, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        h.note(ui, "As");
                        if h.button(ui, "a notice", !s.round_notice_dialog, true, false) {
                            s.round_notice_dialog = false;
                        }
                        if h.button(ui, "a dialog", s.round_notice_dialog, true, false) {
                            s.round_notice_dialog = true;
                        }
                    });
                });
                ui.add_enabled_ui(s.round_notice && !s.round_notice_dialog, |ui| {
                    h.slider(
                        ui,
                        &mut s.round_notice_size,
                        10.0..=120.0,
                        "Notice text size",
                    );
                    h.slider(
                        ui,
                        &mut s.round_notice_secs,
                        0.5..=60.0,
                        "Notice shown for (s)",
                    );
                    h.slider(
                        ui,
                        &mut s.round_notice_opacity,
                        0.05..=1.0,
                        "Notice opacity",
                    );
                    h.slider(
                        ui,
                        &mut s.round_notice_images,
                        0..=50,
                        "…or gone after this many new images (0 = only the time)",
                    );
                    ui.horizontal_wrapped(|ui| {
                        h.check(ui, &mut s.round_notice_fade, "Fade in and out");
                        ui.add_enabled_ui(s.round_notice_fade, |ui| {
                            h.slider(ui, &mut s.round_notice_fade_ms, 50..=2000, "Fade (ms)");
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        h.note(ui, "Notice place");
                        for (value, label) in
                            [("top", "Top"), ("center", "Centre"), ("bottom", "Bottom")]
                        {
                            if h.button(ui, label, s.round_notice_place == value, true, false) {
                                s.round_notice_place = value.to_owned();
                            }
                        }
                        h.note(ui, "Colour");
                        let c = s.round_notice_color;
                        let mut color = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
                        if ui.color_edit_button_srgba(&mut color).changed() {
                            s.round_notice_color = color.to_srgba_unmultiplied();
                        }
                    });
                });
                h.note(ui, RichText::new("In the image view: once every image of the list has been shown, a new round begins. The toolbar's round button (in the group by default) turns this on and off.").color(MUTED));
                h.section(ui, "Window edges");
                h.check(
                    ui,
                    &mut s.edges,
                    "Browse by hovering at the left / right edge",
                );
                h.slider(
                    ui,
                    &mut s.side_px,
                    2.0..=800.0,
                    "Side activation strip (px)",
                );
                h.slider(ui, &mut s.side_ms, 0..=4000, "Side activation delay (ms)");
                h.check(
                    ui,
                    &mut s.top_delete,
                    "Move image to Recycle Bin at the top edge",
                );
                h.slider(ui, &mut s.top_px, 2.0..=600.0, "Top activation strip (px)");
                h.slider(ui, &mut s.top_ms, 0..=6000, "Top activation delay (ms)");
                h.note(ui, RichText::new("Strips cover at most 45 % of the window. With a 0 ms top delay and no confirmation, touching the top edge moves the image to the Recycle Bin at once.").color(MUTED));
                h.section(ui, "Slideshow");
                h.slider(ui, &mut s.slideshow_secs, 0.5..=120.0, "Seconds per image");
                order_controls(ui, h, &mut s.slideshow_order, "slide_order");
                h.slider(
                    ui,
                    &mut s.slideshow_side_px,
                    2.0..=800.0,
                    "Slideshow side strip (px)",
                );
                h.slider(
                    ui,
                    &mut s.slideshow_side_ms,
                    0..=4000,
                    "Slideshow side delay (ms)",
                );
                h.slider(
                    ui,
                    &mut s.slideshow_top_px,
                    2.0..=600.0,
                    "Slideshow top strip (px)",
                );
                h.slider(
                    ui,
                    &mut s.slideshow_top_ms,
                    0..=4000,
                    "Slideshow pause / resume delay (ms)",
                );
                h.note(
                    ui,
                    "During a slideshow the top edge pauses / resumes; it never deletes.",
                );
            }
            2 => {
                h.section(ui, "Pages");
                h.slider(
                    ui,
                    &mut s.thumbnail_count,
                    1..=64,
                    "Images per thumbnail page",
                );
                h.check(
                    ui,
                    &mut s.grid_round_full,
                    "Round page size up to fill the grid",
                );
                h.note(ui, "When a page size leaves empty cells, it is rounded up to the next full grid, if the folder has enough images. − / + then step between full sizes.");
                h.check(
                    ui,
                    &mut s.grid_page_arrows,
                    "Previous / Next page arrows in the grid controls",
                );
                h.check(
                    ui,
                    &mut s.grid_open_at_current,
                    "Open the grid at the page of the image being viewed",
                );
                h.check(
                    ui,
                    &mut s.grid_random_both,
                    "Random / Shuffle page order: both directions show a new page (no history)",
                );
                h.check(
                    ui,
                    &mut s.filter_per_view,
                    "Separate filters for the grid and the image view (off: one filter for both)",
                );
                h.check(
                    ui,
                    &mut s.grid_wheel_pages,
                    "Mouse wheel changes the grid page",
                );
                h.check(
                    ui,
                    &mut s.grid_page_wrap,
                    "Grid pages wrap around: back from the first page to the last, on from the last to the first (In order pages)",
                );
                h.check(
                    ui,
                    &mut s.confirm_preset_remove,
                    "Confirm before removing a page-size button",
                );
                h.section(ui, "Opening");
                ui.horizontal_wrapped(|ui| {
                    use crate::model::GridStartMode as M;
                    h.note(ui, "Grid opens in");
                    for (mode, label) in [
                        (M::Selection, "Selection mode"),
                        (M::Normal, "Normal mode"),
                        (M::LastUsed, "Last used mode"),
                    ] {
                        if h.button(ui, label, s.grid_start_mode == mode, true, false) {
                            s.grid_start_mode = mode;
                        }
                    }
                });
                h.section(ui, "Controls");
                ui.horizontal_wrapped(|ui| {
                    h.note(ui, "Grid controls");
                    if h.button(ui, "Text buttons", !s.grid_bar_icons, true, false) {
                        s.grid_bar_icons = false;
                    }
                    if h.button(ui, "Icon bar", s.grid_bar_icons, true, false) {
                        s.grid_bar_icons = true;
                    }
                });
                ui.add_enabled_ui(!s.grid_bar_icons, |ui| {
                    h.check(ui, &mut s.grid_controls_top, "Text buttons at the top");
                    h.check(
                        ui,
                        &mut s.grid_hide_toolbar_text,
                        "Hide the floating toolbar on the grid page with the text buttons (they then also have Open folder and the floating trash)",
                    );
                });
                h.check(
                    ui,
                    &mut s.grid_bar_fixed_width,
                    "Grid controls keep their width (page number and counts take the room of their widest value)",
                );
                h.note(ui, RichText::new("Controls that do not fit in the window go behind » at the right end, which shows them in a bar above.").color(MUTED));
                ui.add_enabled_ui(s.grid_bar_icons, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        h.note(ui, "Icon bar");
                        if h.button(ui, "Always shown", s.grid_bar_fixed, true, false) {
                            s.grid_bar_fixed = true;
                        }
                        if h.button(
                            ui,
                            "Opens at the bottom edge (like the toolbar)",
                            !s.grid_bar_fixed,
                            true,
                            false,
                        ) {
                            s.grid_bar_fixed = false;
                        }
                    });
                    h.slider(
                        ui,
                        &mut s.grid_bar_bottom,
                        0.0..=1500.0,
                        "Icon bar distance from bottom (px)",
                    );
                    h.check(
                        ui,
                        &mut s.grid_hide_toolbar,
                        "Hide the floating toolbar on the grid page (otherwise it is stacked above the icon bar)",
                    );
                    h.note(ui, RichText::new("Always shown: the grid leaves room for the bar. Opening at the bottom edge: the lock icon on the bar (or the grid lock in Display settings) keeps it open. Without the toolbar the bar also has Filter, Open folder and the floating trash button.").color(MUTED));
                });
                // Icon colours, and box and text colours of the text buttons.
                for (title, which) in [
                    ("Icon bar colours", 0),
                    ("Text button box colours", 1),
                    ("Text button text colours", 2),
                ] {
                    egui::CollapsingHeader::new(title).show(ui, |ui| {
                        let (colors, default) = match which {
                            0 => (&mut s.grid_bar_colors, ICON_COLOR),
                            1 => (&mut s.grid_text_colors, TEXT_BUTTON_FILL),
                            _ => (&mut s.grid_text_font_colors, ICON_COLOR),
                        };
                        for (key, name, _) in GRID_BUTTONS {
                            if which > 0 && *key == "lock" {
                                continue;
                            }
                            let mut color = colors.get(*key).copied().unwrap_or(default);
                            color_controls(ui, h, &mut color, name);
                            if color != default || colors.contains_key(*key) {
                                colors.insert((*key).to_owned(), color);
                            }
                        }
                        if h.button(ui, "Reset colours", false, !colors.is_empty(), false) {
                            colors.clear();
                        }
                    });
                }
                h.section(ui, "Under the images and the title");
                h.check(
                    ui,
                    &mut s.grid_keep_tag_mode,
                    "Keep grid tag mode and palette open after saving keywords",
                );
                h.check(
                    ui,
                    &mut s.grid_show_marks,
                    "Show keywords and Favorite / Elite under grid images",
                );
                ui.add_enabled_ui(s.grid_show_marks, |ui| {
                    h.check(
                        ui,
                        &mut s.grid_marks_icons,
                        "Favorite / Elite as icons only (♥ / ★)",
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    h.note(ui, "Grid page title");
                    if h.button(ui, "first – last image", s.grid_caption_range, true, false) {
                        s.grid_caption_range = true;
                    }
                    if h.button(ui, "first image – ...", !s.grid_caption_range, true, false) {
                        s.grid_caption_range = false;
                    }
                });
                h.check(
                    ui,
                    &mut s.grid_caption_folder,
                    "Grid page title: the folder before the image name",
                );
                h.check(
                    ui,
                    &mut s.grid_caption_count,
                    "Grid page title: the number of images at the end",
                );
                h.section(ui, "Hover preview");
                h.check(
                    ui,
                    &mut s.grid_hover_preview,
                    "Hover preview: rest on a grid image to show it larger",
                );
                h.slider(
                    ui,
                    &mut s.hover_preview_ms,
                    150..=3000,
                    "Hover preview delay (ms)",
                );
                h.check(
                    ui,
                    &mut s.hover_circle,
                    "Hover preview starts only from a circle in the middle of the image",
                );
                h.slider(
                    ui,
                    &mut s.hover_circle_pct,
                    10.0..=100.0,
                    "Circle diameter (% of the image's shorter side)",
                );
                h.slider(
                    ui,
                    &mut s.hover_circle_reveal_px,
                    0.0..=400.0,
                    "Show the circle when the pointer is this close (px)",
                );
                h.check(
                    ui,
                    &mut s.hover_circle_fade,
                    "Fade the circle in as the pointer approaches",
                );
                h.check(
                    ui,
                    &mut s.hover_circle_always,
                    "Always show the circle on every image (faint)",
                );
                h.slider(
                    ui,
                    &mut s.hover_circle_opacity,
                    0.004..=1.0,
                    "Circle opacity",
                );
                h.slider(
                    ui,
                    &mut s.hover_circle_width,
                    0.5..=4.0,
                    "Circle line width (px)",
                );
                h.slider(
                    ui,
                    &mut s.preview_size,
                    40.0..=100.0,
                    "Preview size (% of the grid area)",
                );
                h.check(
                    ui,
                    &mut s.preview_close_top,
                    "Close the preview from a top strip",
                );
                h.check(
                    ui,
                    &mut s.preview_close_left,
                    "Close the preview from a left strip",
                );
                h.check(
                    ui,
                    &mut s.preview_close_right,
                    "Close the preview from a right strip",
                );
                h.check(
                    ui,
                    &mut s.preview_close_click,
                    "Close the preview with a left click",
                );
                h.slider(
                    ui,
                    &mut s.preview_close_px,
                    2.0..=800.0,
                    "Preview closing strip size (px)",
                );
                h.slider(
                    ui,
                    &mut s.preview_close_ms,
                    0..=3000,
                    "Preview closing delay (ms)",
                );
                h.slider(
                    ui,
                    &mut s.preview_close_move_px,
                    0.0..=1000.0,
                    "Close the preview when the pointer moves this far (px, 0 = off)",
                );
                ui.add_enabled_ui(s.preview_close_move_px > 0., |ui| {
                    ui.horizontal_wrapped(|ui| {
                        h.note(ui, "In this direction");
                        for (value, label) in [
                            ("any", "Any"),
                            ("up", "Up"),
                            ("down", "Down"),
                            ("left", "Left"),
                            ("right", "Right"),
                        ] {
                            if h.button(ui, label, s.preview_close_move_dir == value, true, false) {
                                s.preview_close_move_dir = value.to_owned();
                            }
                        }
                    });
                });
                h.note(
                    ui,
                    "The preview also closes with the gesture and with Escape.",
                );
                h.section(ui, "Selection");
                h.check(
                    ui,
                    &mut s.normal_keeps_selection,
                    "Keep the trash / tag selection when switching to Normal mode",
                );
                h.note(
                    ui,
                    "When off, Normal mode clears the selection; Restore selection brings it back.",
                );
                h.check(
                    ui,
                    &mut s.selection_flash,
                    "Flash the button on Select all / Clear / Restore selection",
                );
                h.check(
                    ui,
                    &mut s.selection_toast,
                    "Show a large message on Select all / Clear / Restore selection",
                );
                h.slider(
                    ui,
                    &mut s.selection_toast_size,
                    14.0..=48.0,
                    "Message text size",
                );
                h.slider(
                    ui,
                    &mut s.selection_toast_width,
                    200.0..=1600.0,
                    "Message box width (px)",
                );
                h.slider(
                    ui,
                    &mut s.selection_toast_height,
                    60.0..=800.0,
                    "Message box height (px)",
                );
                h.section(ui, "Big image from the grid");
                h.check(
                    ui,
                    &mut s.grid_viewer_browse,
                    "Browse images in the big image opened from the grid (the grid page follows)",
                );
                h.slider(
                    ui,
                    &mut s.grid_hold_ms,
                    100..=1500,
                    "Big image: hold longer than this for the magnifier (ms)",
                );
                h.note(
                    ui,
                    "In the big image opened from the grid, a shorter click returns to the grid.",
                );
                h.check(
                    ui,
                    &mut s.magnify_on_move,
                    "Big image: moving while pressed starts the magnifier at once",
                );
                h.check(
                    ui,
                    &mut s.magnify_at_once,
                    "Big image: magnify at once on press (a quick click still returns to the grid)",
                );
            }
            3 => {
                h.section(ui, "Toolbar");
                h.slider(ui, &mut s.bar_width, 340.0..=1600.0, "Toolbar width (px)");
                s.initialize_tag_width();
                h.slider(
                    ui,
                    s.tags_max_width.as_mut().unwrap(),
                    340.0..=1600.0,
                    "Tags maximum width (px)",
                );
                h.slider(
                    ui,
                    &mut s.bar_height,
                    40.0..=160.0,
                    "Toolbar minimum height (px)",
                );
                h.slider(
                    ui,
                    &mut s.bar_bottom,
                    0.0..=1500.0,
                    "Distance from bottom, image view (px)",
                );
                let mut grid = s.bar_bottom_grid();
                h.slider(
                    ui,
                    &mut grid,
                    0.0..=1500.0,
                    "Distance from bottom, grid (px)",
                );
                s.bar_bottom_grid = Some(grid);
                h.check(
                    ui,
                    &mut s.bar_locked,
                    if s.bar_lock_universal {
                        "Lock the toolbar open (everywhere)"
                    } else {
                        "Lock the toolbar open in the image view"
                    },
                );
                ui.add_enabled_ui(!s.bar_lock_universal, |ui| {
                    h.check(
                        ui,
                        &mut s.bar_locked_grid,
                        "Lock the toolbar open in the grid",
                    );
                });
                h.check(
                    ui,
                    &mut s.bar_lock_universal,
                    "Universal lock: one lock for the image view and the grid",
                );
                h.note(ui, RichText::new("A locked toolbar stays open: the bottom edge and Escape do not close it. The lock icon at its right end switches the lock of the current view. The image view and the grid keep their own distance from the bottom; the toolbar always stays on screen.").color(MUTED));
                h.slider(
                    ui,
                    &mut s.bar_offset,
                    -600.0..=600.0,
                    "Horizontal offset (px)",
                );
                color_controls(ui, h, &mut s.bar_color, "Toolbar color");
                h.check(
                    ui,
                    &mut s.show_favorites_button,
                    "Show Favorites button in toolbar",
                );
                h.slider(
                    ui,
                    &mut s.bar_edge_px,
                    2.0..=100.0,
                    "Bottom activation strip (px)",
                );
                h.slider(
                    ui,
                    &mut s.bar_delay_ms,
                    0..=3000,
                    "Toolbar activation delay (ms)",
                );
                bar_buttons_editor(ui, h, s, &mut self.bar_buttons_tab);
                h.section(ui, "Hover activation");
                h.check(
                    ui,
                    &mut s.hover,
                    "Activate controls by hovering (toolbar, grid, menus, dialogs)",
                );
                let mut trash = s.hover_trash();
                h.check(
                    ui,
                    &mut trash,
                    "Activate the floating trash button by hovering",
                );
                s.hover_trash = Some(trash);
                let mut moving = s.hover_move();
                h.check(
                    ui,
                    &mut moving,
                    "Activate the floating Move button by hovering",
                );
                s.hover_move = Some(moving);
                let mut settings = s.hover_settings();
                h.check(
                    ui,
                    &mut settings,
                    "Activate Settings window controls by hovering",
                );
                s.hover_settings = Some(settings);
                h.slider(ui, &mut s.hover_ms, 0..=4000, "Control dwell delay (ms)");
                h.slider(
                    ui,
                    &mut s.destructive_ms,
                    0..=6000,
                    "Destructive action dwell delay (ms)",
                );
                h.check(
                    ui,
                    &mut s.hover_ring,
                    "Show the progress ring while hovering a control",
                );
                h.note(ui, RichText::new("The progress ring shows when a control will activate. Leave a control or edge to re-arm it.").color(MUTED));
                h.section(ui, "Floating trash button");
                h.slider(ui, &mut s.delete_size, 48.0..=260.0, "Diameter (px)");
                color_controls(ui, h, &mut s.delete_color, "Button color");
                h.slider(
                    ui,
                    &mut s.delete_opacity,
                    0.02..=1.0,
                    "Overall opacity (including border & close)",
                );
                h.check(ui, &mut s.delete_border, "Show border");
                h.slider(
                    ui,
                    &mut s.delete_border_width,
                    0.5..=12.0,
                    "Border width (px)",
                );
                color_controls(ui, h, &mut s.delete_border_color, "Border color");
                h.slider(
                    ui,
                    &mut s.close_angle,
                    0.0..=360.0,
                    "Close button angle (degrees)",
                );
                h.slider(
                    ui,
                    &mut s.close_size,
                    16.0..=64.0,
                    "Close button diameter (px)",
                );
                if h.button(ui, "Reset trash button position", false, true, false) {
                    s.delete_pos = None;
                    self.delete_area_revision += 1;
                }
                h.section(ui, "Floating Move button");
                ui.horizontal(|ui| {
                    h.note(ui, "Button colour");
                    let c = s.move_color;
                    let mut color = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
                    if ui.color_edit_button_srgba(&mut color).changed() {
                        s.move_color = color.to_srgba_unmultiplied();
                    }
                });
            }
            4 => {
                h.section(ui, "Recycle Bin");
                h.check(
                    ui,
                    &mut s.confirm_delete,
                    "Confirm before moving images to Recycle Bin",
                );
                h.check(
                    ui,
                    &mut s.confirm_restore,
                    "Confirm before restoring deleted images",
                );
                h.check(
                    ui,
                    &mut s.forget_deletion_on_close,
                    "Forget the last deletion when NoirView closes",
                );
                h.note(ui, RichText::new("Restore deleted (toolbar, grid bar, right-click menu, Ctrl+Z) moves the last deleted images back from the Recycle Bin and shows the first one. Off: it also works after a restart.").color(MUTED));
                h.section(ui, "Moving");
                h.note(ui, "Move destination");
                h.note(ui, s.destination.display().to_string());
                if h.button(
                    ui,
                    "Choose destination",
                    false,
                    !self.busy && !self.view_pending,
                    false,
                ) {
                    actions.push("destination");
                }
                h.check(
                    ui,
                    &mut s.verified_move,
                    "Verify moved files with SHA-256 and keep a recovery journal",
                );
                h.check(
                    ui,
                    &mut s.move_keep_names,
                    "Keep the original file names when moving (default in the Move dialog)",
                );
                h.slider(ui, &mut s.move_digits, 1..=8, "Digits in sequence names");
                h.check(
                    ui,
                    &mut s.move_fill_gaps,
                    "Sequence names fill free numbers in the destination first",
                );
                let example = format!("{:0w$}", 1, w = s.move_digits.clamp(1, 8));
                h.note(ui, RichText::new(format!("Without keeping the names they become {example}.ext and on (the format and extension stay intact), after the highest such number already in the destination; past the last number of that width it goes on with one digit more. Clashing original names get \" (2)\" and so on.")).color(MUTED));
                h.note(ui, "The move includes all images in the opened folder scope, even with an active filter.");
                if h.button(
                    ui,
                    "Recover interrupted moves…",
                    false,
                    !self.busy && !self.view_pending && !s.destination.as_os_str().is_empty(),
                    false,
                ) {
                    actions.push("recover");
                }
                h.section(ui, "Marking and the Move grid");
                h.note(ui, RichText::new("Single image view. Its toolbar button: a click shows the floating Move button (moves the image shown); holding starts marking images to move, and while marking a click opens the Move grid of the marked images (holding stops marking). The Move grid works like the grid (hover preview, tags, selections, Trash, page size) with Move, Destination (hold: favorites), Keep names and Cancel on its icon bar; a click on a thumbnail shows it big.").color(MUTED));
                ui.horizontal_wrapped(|ui| {
                    h.note(ui, "Move grid pages");
                    if h.button(ui, "Pages", !s.move_grid_all, true, false) {
                        s.move_grid_all = false;
                    }
                    if h.button(
                        ui,
                        "All on one page (scrolls on)",
                        s.move_grid_all,
                        true,
                        false,
                    ) {
                        s.move_grid_all = true;
                    }
                });
                h.slider(
                    ui,
                    &mut s.move_grid_count,
                    1..=crate::grid_layout::MAX_COUNT,
                    "Images per page",
                );
                h.check(
                    ui,
                    &mut s.move_grid_round_full,
                    "Round page size up to fill the grid",
                );
                ui.add_enabled_ui(s.move_grid_all, |ui| {
                    h.check(
                        ui,
                        &mut s.move_grid_row,
                        "One row at a time (instead of a page)",
                    );
                });
                h.note(ui, RichText::new("All on one page: the page goes on downwards; the wheel and the page arrows bring the next page (or row) up instead of turning the page.").color(MUTED));
                h.check(
                    ui,
                    &mut s.move_grid_browse,
                    "The big image from the Move grid browses the marked images",
                );
                h.check(
                    ui,
                    &mut s.move_grid_marks,
                    "Show the \"✔ MOVE\" mark on the Move grid's thumbnails",
                );
                ui.horizontal_wrapped(|ui| {
                    h.note(ui, "× on the thumbnails: colour");
                    let c = s.move_x_color;
                    let mut color = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
                    if ui.color_edit_button_srgba(&mut color).changed() {
                        s.move_x_color = color.to_srgba_unmultiplied();
                    }
                });
                corner_controls(ui, h, "× place", &mut s.move_x_corner, &mut s.move_x_offset);
                corner_controls(
                    ui,
                    h,
                    "\"✔ MOVE\" mark place",
                    &mut s.move_mark_corner,
                    &mut s.move_mark_offset,
                );
                h.check(
                    ui,
                    &mut s.move_grid_frame,
                    "A frame round the window while the Move grid is open",
                );
                ui.add_enabled_ui(s.move_grid_frame, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        h.note(ui, "Frame colour");
                        let c = s.move_grid_frame_color;
                        let mut color = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
                        if ui.color_edit_button_srgba(&mut color).changed() {
                            s.move_grid_frame_color = color.to_srgba_unmultiplied();
                        }
                    });
                    h.slider(
                        ui,
                        &mut s.move_grid_frame_width,
                        1.0..=24.0,
                        "Frame width (px)",
                    );
                    h.slider(
                        ui,
                        &mut s.move_grid_frame_opacity,
                        0.05..=1.0,
                        "Frame opacity",
                    );
                });
                h.slider(ui, &mut s.move_x_size, 12.0..=48.0, "× size (px)");
                h.slider(ui, &mut s.move_x_opacity, 0.1..=1.0, "× opacity");
                h.check(
                    ui,
                    &mut s.grid_move_marks,
                    "Show the marks on the grid's thumbnails too",
                );
            }
            5 => {
                h.section(ui, "Folders");
                h.check(
                    ui,
                    &mut s.recursive,
                    "Include subfolders when opening a folder",
                );
                if self.settings_scroll.take() == Some("favorites") {
                    ui.scroll_to_cursor(Some(egui::Align::TOP));
                }
                h.section(ui, "Favorite folders");
                h.note(ui, RichText::new("Hold Open folder (toolbar or grid) to open one of these; the list can also add or remove the open folder.").color(MUTED));
                h.check(
                    ui,
                    &mut s.remember_favorite_places,
                    "Favorite folders open where browsing left them",
                );
                h.note(ui, RichText::new("Kept for each profile (and for no profile) and across restarts, with the browsing order: Shuffle goes on with the images not yet shown, new ones included. Go to image 1 starts over; deleted images are skipped.").color(MUTED));
                let mut remove = None;
                let mut swap = None;
                let count = s.favorite_folders.len();
                egui::Grid::new("favorite_folders")
                    .num_columns(4)
                    .spacing(Vec2::new(8., 4.))
                    .show(ui, |ui| {
                        for (i, f) in s.favorite_folders.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                if h.button(ui, "⏶", false, i > 0, false) {
                                    swap = Some((i, i - 1));
                                }
                                if h.button(ui, "⏷", false, i + 1 < count, false) {
                                    swap = Some((i, i + 1));
                                }
                            });
                            let own = f
                                .path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            ui.add(
                                egui::TextEdit::singleline(&mut f.name)
                                    .desired_width(160.)
                                    .hint_text(own),
                            );
                            ui.add(
                                egui::Label::new(
                                    RichText::new(f.path.display().to_string()).color(MUTED),
                                )
                                .truncate(),
                            );
                            ui.horizontal(|ui| {
                                if h.button(ui, "Change…", false, true, false) {
                                    self.favorite_pick = Some(i);
                                    actions.push("favorite");
                                }
                                if h.button(ui, "Remove", false, true, true) {
                                    remove = Some(i);
                                }
                            });
                            ui.end_row();
                        }
                    });
                if let Some((a, b)) = swap {
                    s.favorite_folders.swap(a, b);
                }
                if let Some(i) = remove {
                    s.favorite_folders.remove(i);
                }
                ui.horizontal(|ui| {
                    if h.button(ui, "Add a folder…", false, true, false) {
                        self.favorite_pick = None;
                        actions.push("favorite");
                    }
                    let open = self
                        .folder
                        .clone()
                        .filter(|f| !s.favorite_folders.iter().any(|x| x.path == *f));
                    if h.button(ui, "Add the open folder", false, open.is_some(), false) {
                        if let Some(path) = open {
                            s.favorite_folders.push(crate::model::FavoriteFolder {
                                name: String::new(),
                                path,
                            });
                        }
                    }
                });
                h.note(
                    ui,
                    RichText::new("An empty name shows the folder's own name.").color(MUTED),
                );
                ui.horizontal_wrapped(|ui| {
                    h.note(ui, "In the list as");
                    for (value, label) in [
                        ("buttons", "buttons"),
                        ("text", "/name"),
                        ("icon", "🗀 name"),
                    ] {
                        if h.button(ui, label, s.favorite_style == value, true, false) {
                            s.favorite_style = value.to_owned();
                        }
                    }
                });
                h.section(ui, "Keywords and tags");
                h.note(ui, "Keywords are cached in a local SQLite index. Only new or changed images are read again; the open folder is watched for changes.");
                if let Some(store) = &self.store {
                    if let Some(parent) = store.path.parent() {
                        h.note(
                            ui,
                            format!(
                                "Keyword index: {}",
                                parent.join("keyword-index.sqlite3").display()
                            ),
                        );
                    }
                }
                if let Some(progress) = self.last_index {
                    h.note(
                        ui,
                        format!(
                            "Last index: {} checked · {} cached · {} read · {:.2}s",
                            progress.total,
                            progress.cached,
                            progress.read,
                            progress.elapsed.as_secs_f32()
                        ),
                    );
                }
                if h.button(ui, "Rebuild keyword index…", false, !self.busy, false) {
                    actions.push("reindex");
                }

                h.note(ui, "Tags shows My tags by default. Additional includes image keywords with yellow borders. Use + to create your own tags.");
                h.note(ui, "Remove tags hides words from the palette; existing image keywords remain unchanged.");
                h.note(
                    ui,
                    "JPEG: XMP + Windows Tags. PNG/WebP: embedded XMP. Other formats: image.ext.xmp sidecars.",
                );
                if h.button(ui, "Rescan current folder…", false, !self.busy, false) {
                    actions.push("rescan");
                }
                h.section(ui, "Watched folders");
                h.check(
                    ui,
                    &mut s.watch_folders,
                    "Keep thumbnails and keywords of these folders up to date",
                );
                let mut remove = None;
                for (i, folder) in s.watched_folders.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(folder.path.display().to_string()).truncate());
                        h.check(ui, &mut folder.subfolders, "Subfolders");
                        if h.button(ui, "Remove", false, true, false) {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    s.watched_folders.remove(i);
                }
                ui.horizontal(|ui| {
                    if h.button(ui, "Add folder…", false, true, false) {
                        actions.push("watch");
                    }
                    if h.button(
                        ui,
                        "Check all now",
                        false,
                        self.watch.handle.is_some(),
                        false,
                    ) {
                        actions.push("watch_check");
                    }
                });
                let on = s.watch_folders;
                ui.add_enabled_ui(on, |ui| {
                h.check(
                    ui,
                    &mut s.watch_while_active,
                    "Keep working while I use NoirView",
                );
                h.check(
                    ui,
                    &mut s.watch_when_unfocused,
                    "Keep working when NoirView is not the active window",
                );
                h.note(ui, "While NoirView is open, it watches the folders itself; otherwise it pauses while you use it.");
                h.check(
                    ui,
                    &mut s.watch_background,
                    if cfg!(windows) {
                        "Also while NoirView is closed (starts at Windows sign-in)"
                    } else {
                        "Also while NoirView is closed"
                    },
                );
                ui.horizontal_wrapped(|ui| {
                    h.note(ui, "Pace");
                    for pace in crate::model::WatchPace::ALL {
                        if h.button(ui, pace.label(), s.watch_pace == pace, true, false) {
                            s.watch_pace = pace;
                        }
                    }
                });
                h.note(ui, "Gentle rests between images (easiest on a hard disk), Balanced rests less, Fast does not rest.");
                });
                if let Some(handle) = &self.watch.handle {
                    let status = handle.status();
                    h.note(ui, format!("Watcher (in NoirView): {}", status.summary()));
                    if status.cache_full {
                        ui.colored_label(
                            RED,
                            "The thumbnail cache is full: new thumbnails are not made. Raise the thumbnail cache size.",
                        );
                    }
                    for warning in &status.warnings {
                        ui.colored_label(RED, warning);
                    }
                } else if self.watch.refused {
                    ui.colored_label(
                        RED,
                        "Another open NoirView window is watching folders; close it to watch here.",
                    );
                } else if s.watch_folders && !s.watched_folders.is_empty() {
                    h.note(
                        ui,
                        if self.watch.lock.is_none() {
                            "Watcher: waiting for the background watcher to hand over…"
                        } else {
                            "Watcher: starting…"
                        },
                    );
                }
                if let Some(last) = &self.watch.background {
                    h.note(
                        ui,
                        format!(
                            "Last background run (version {}): {}",
                            last.version,
                            last.summary()
                        ),
                    );
                }
                if let Some(error) = &self.watch.error {
                    ui.colored_label(RED, error);
                }
                h.note(
                    ui,
                    "Keywords are shared with the viewer when you open the same folder with the same subfolder setting; thumbnails are shared always.",
                );
                ui.separator();
                h.note(
                    ui,
                    RichText::new("Fast change detection (NTFS change journal)").strong(),
                );
                h.note(ui, "An optional Windows service that reads the NTFS change journal for the watcher. It runs as the system account because only administrators may read the journal, opens no images, and answers only you. Installing and removing it each ask for administrator rights once.");
                if self
                    .usn
                    .checked
                    .is_none_or(|t| t.elapsed() > Duration::from_secs(10))
                {
                    actions.push("usn_refresh");
                }
                let installed = matches!(
                    self.usn.state,
                    Some(crate::usn::State::Running) | Some(crate::usn::State::Stopped)
                );
                let outdated = self
                    .usn
                    .version
                    .as_deref()
                    .is_some_and(|v| v != env!("CARGO_PKG_VERSION"));
                h.note(
                    ui,
                    match (&self.usn.state, &self.usn.version) {
                        (None, _) => "Service: checking…".to_string(),
                        (Some(crate::usn::State::Unsupported), _) => {
                            "Service: available on Windows only".to_string()
                        }
                        (Some(crate::usn::State::NotInstalled), _) => {
                            "Service: not installed (the watcher lists folders instead)".to_string()
                        }
                        (Some(crate::usn::State::Running), Some(v)) => {
                            format!("Service: running (version {v})")
                        }
                        (Some(crate::usn::State::Running), None) => {
                            "Service: running but not answering".to_string()
                        }
                        (Some(crate::usn::State::Stopped), _) => {
                            "Service: installed but not running".to_string()
                        }
                    },
                );
                let idle = !self.usn.busy && self.usn.state != Some(crate::usn::State::Unsupported);
                ui.horizontal(|ui| {
                    let label = if outdated {
                        "Update service…"
                    } else if installed {
                        "Reinstall service…"
                    } else {
                        "Install service…"
                    };
                    if h.button(ui, label, false, idle, false) {
                        actions.push("usn_install");
                    }
                    if h.button(ui, "Remove service…", false, idle && installed, true) {
                        actions.push("usn_remove");
                    }
                    if h.button(ui, "Test", false, idle && self.usn.version.is_some(), false) {
                        actions.push("usn_test");
                    }
                    if self.usn.busy {
                        ui.spinner();
                    }
                });
                if outdated {
                    ui.colored_label(
                        RED,
                        "The service is from another NoirView version: update it.",
                    );
                }
                if let Some(message) = &self.usn.message {
                    h.note(ui, message);
                }
                h.check(
                    ui,
                    &mut s.watch_use_journal,
                    "Use the change journal when the service is installed",
                );
                h.note(ui, "With the service, the watcher reads what changed since the last check (at start and after large copies) instead of listing folders. Live watching is the same either way.");
            }
            6 => {
                h.section(ui, "Image loading");
                h.note(ui, "Image loading");
                ui.horizontal_wrapped(|ui| {
                    for mode in [
                        crate::model::LoadingMode::Standard,
                        crate::model::LoadingMode::Aggressive,
                    ] {
                        if h.button(ui, mode.label(), s.loading_mode == mode, true, false) {
                            s.loading_mode = mode;
                        }
                    }
                });
                h.note(ui, "Standard loads what is on screen and the neighbouring pages. Aggressive also caches thumbnails for the whole open folder in the background (one low-priority worker) and keeps a screen-sized first view of large images you have opened.");
                let aggressive = s.loading_mode == crate::model::LoadingMode::Aggressive;
                ui.add_enabled_ui(aggressive, |ui| {
                    h.check(
                        ui,
                        &mut s.cache_while_active,
                        "Keep caching thumbnails while I use NoirView",
                    );
                    h.check(
                        ui,
                        &mut s.cache_when_unfocused,
                        "Keep caching thumbnails when the window is not active",
                    );
                    h.slider(
                        ui,
                        &mut s.proxy_cache_mb,
                        0..=4096,
                        "First-view disk cache for large images (MiB)",
                    );
                    h.slider(
                        ui,
                        &mut s.proxy_min_side,
                        crate::thumbnail_cache::PROXY_SIDE..=8192,
                        "Keep a first view for images at least this long (px, longer side)",
                    );
                });
                h.note(ui, "Without these options the background pass pauses 2 s after any input and while another program is in front. It always pauses during keyword indexing. The first view is a slightly compressed 2560 px copy shown until the exact image is ready; zero disables it. When one was shown, a note at the bottom left of the image reports its timing for a few seconds.");
                let proxies = self.images.proxy_cache.stats();
                if aggressive {
                    h.note(ui, format!(
                        "First views: {} hits / {} saved this session ({} not saved) · {} stored ({} MiB)",
                        proxies.hits,
                        proxies.writes,
                        proxies.skipped + proxies.errors,
                        proxies.entries,
                        proxies.bytes / (1024 * 1024)
                    ));
                    if let Some(summary) = self.view_timer.as_ref().and_then(|t| t.summary()) {
                        h.note(ui, format!("Last opened image: {summary}"));
                    }
                    if !s.jpeg_preview {
                        h.note(ui, RichText::new("First views are not shown: Settings → Display → \"Show a quick preview while the image loads\" is off.").color(RED));
                    }
                }
                h.slider(ui, &mut s.prefetch_ahead, 0..=32, "Prefetch ahead");
                h.slider(ui, &mut s.prefetch_behind, 0..=32, "Prefetch behind");
                h.note(ui, "Maximum images in and against the browsing direction. Memory limits may reduce these counts. Zero disables that side.");
                h.section(ui, "Thumbnail cache");
                h.slider(
                    ui,
                    &mut s.thumbnail_cache_mb,
                    0..=4096,
                    "Thumbnail disk cache (MiB)",
                );
                h.note(ui, "Speeds up reopening thumbnails. Zero disables disk caching; full image detail is preserved.");
                if let Some(p) = self.images.pregen_progress() {
                    h.note(
                        ui,
                        if p.full {
                            format!(
                                "Pre-generation stopped: cache nearly full ({} / {} images)",
                                p.done, p.total
                            )
                        } else {
                            format!(
                                "Pre-generated: {} / {} images ({} new this pass)",
                                p.done, p.total, p.generated
                            )
                        },
                    );
                }
                let cache = self.images.disk_cache.stats();
                h.note(
                    ui,
                    format!(
                        "Thumbnail cache: {} hits / {} saved this session · {} stored ({} MiB)",
                        cache.hits,
                        cache.writes,
                        cache.entries,
                        cache.bytes / (1024 * 1024)
                    ),
                );
                h.note(
                    ui,
                    if cache.pending == 0 {
                        "Thumbnail cache: ready".to_owned()
                    } else {
                        format!("Thumbnail cache: saving {} image(s)…", cache.pending)
                    },
                );
                if cache.skipped > 0 || cache.errors > 0 {
                    h.note(ui, format!(
                        "Cache: {} older writes skipped under memory pressure / {} write errors this session",
                        cache.skipped, cache.errors
                    ));
                }
                h.note(
                    ui,
                    if s.thumbnail_cache_dir.as_os_str().is_empty() {
                        "Thumbnail cache folder: default (inside the NoirView data folder)"
                            .to_owned()
                    } else {
                        format!(
                            "Thumbnail cache folder: {}",
                            s.thumbnail_cache_dir.join(CACHE_FOLDER).display()
                        )
                    },
                );
                ui.horizontal(|ui| {
                    if h.button(ui, "Choose cache folder", false, true, false) {
                        actions.push("cache_dir");
                    }
                    if h.button(ui, "Use default folder", false, true, false) {
                        s.thumbnail_cache_dir = PathBuf::new();
                    }
                });
                if s.thumbnail_cache_dir != self.cache_dir_in_use {
                    h.note(
                        ui,
                        "The new cache folder is used after restarting NoirView.",
                    );
                }
                h.note(ui, "A folder outside the program folder keeps thumbnails when NoirView is replaced by a newer version.");
                if h.button(
                    ui,
                    "Clear thumbnail cache (disk and memory)",
                    false,
                    true,
                    false,
                ) {
                    actions.push("clear_thumbnails");
                }
                h.section(ui, "Memory");
                ui.horizontal_wrapped(|ui| {
                    h.note(ui, "Memory use");
                    for choice in crate::model::MemoryUse::ALL {
                        if h.button(ui, choice.label(), s.memory_use() == choice, true, false) {
                            s.memory_use = Some(choice);
                            s.cache_mb =
                                crate::memory::image_budget_mb(choice, s.cache_mb, &self.machine);
                        }
                    }
                });
                h.note(
                    ui,
                    "Low, Balanced and High size the memory for images (full images, first views and thumbnails) from this computer's RAM and graphics card; Custom uses the value set below. When memory runs short, fewer images are kept ready, and at worst only the image on screen.",
                );
                if s.memory_use() == crate::model::MemoryUse::Custom {
                    h.slider(ui, &mut s.cache_mb, 64..=8192, "Image memory budget (MiB)");
                } else {
                    h.note(ui, format!("Image memory budget: {} MiB", s.cache_mb));
                }
                h.note(
                    ui,
                    format!(
                        "Images in memory now: {} MiB · memory now: {}",
                        self.images.memory_mb(),
                        self.memory_guard.level.label()
                    ),
                );
                let m = &self.machine;
                let mut computer = format!(
                    "This computer: RAM {} ({} free)",
                    m.ram_total.map_or("unknown".into(), crate::memory::size),
                    m.ram_available
                        .map_or("unknown".into(), crate::memory::size)
                );
                if let Some(gpu) = &m.gpu {
                    computer.push_str(&format!(
                        " · {}: {}{}",
                        gpu.name,
                        crate::memory::size(gpu.dedicated),
                        match (gpu.usage, gpu.budget) {
                            (Some(u), Some(b)) => format!(
                                " (NoirView uses {} of the {} it may use)",
                                crate::memory::size(u),
                                crate::memory::size(b)
                            ),
                            _ => String::new(),
                        }
                    ));
                }
                h.note(ui, computer);
                h.section(ui, "Display");
                h.check(
                    ui,
                    &mut s.low_latency_display,
                    "Low-latency display: the graphics driver queues one frame instead of two (after a restart)",
                );
                ui.horizontal_wrapped(|ui| {
                    h.note(ui, "Graphics backend (after a restart)");
                    for (value, label) in [
                        ("", "Automatic"),
                        ("dx12", "DirectX 12"),
                        ("vulkan", "Vulkan"),
                        ("opengl", "OpenGL"),
                    ] {
                        if h.button(ui, label, s.graphics_backend == value, true, false) {
                            s.graphics_backend = value.to_owned();
                        }
                    }
                });
                h.note(ui, RichText::new(format!("In use: {}. Automatic is OpenGL on Windows (Vulkan / DirectX 12 if OpenGL does not start). If a backend does not start, the next start uses the automatic choice.", self.gpu)).color(MUTED));
                h.note(ui, RichText::new("Vulkan with NVIDIA: if a strip stays at the window's bottom and right after resizing and clicks land above, set NVIDIA Control Panel → Manage 3D settings → Vulkan/OpenGL present method → Prefer native, or use OpenGL.").color(MUTED));
                h.check(
                    ui,
                    &mut s.remember_window,
                    "Remember the window's size and position (after a restart)",
                );
                h.note(ui, RichText::new("Off: NoirView opens at its default size each time. Try this if part of the window stays transparent or clicks land above where you click.").color(MUTED));
            }
            7 => {
                h.note(ui, "MP3 · WAV · FLAC · Ogg Vorbis");
                ui.horizontal(|ui| {
                    if h.button(ui, "Add tracks", false, true, false) {
                        actions.push("tracks");
                    }
                    if h.button(ui, "Clear playlist", false, !s.music.is_empty(), false) {
                        s.music.clear();
                    }
                });
                h.check(ui, &mut s.music_shuffle, "Play tracks in random order");
                h.slider(ui, &mut s.volume, 0.0..=1.0, "Volume");
                let mut remove = None;
                for (i, p) in s.music.iter().enumerate() {
                    ui.push_id(i, |ui| {
                        ui.horizontal(|ui| {
                            if h.button(ui, "×", false, true, false) {
                                remove = Some(i);
                            }
                            h.note(ui, p.file_name().unwrap_or_default().to_string_lossy());
                            h.note(
                                ui,
                                RichText::new(p.display().to_string()).small().color(MUTED),
                            );
                        });
                    });
                }
                if let Some(i) = remove {
                    s.music.remove(i);
                }
                if let Some(track) = self.music.track() {
                    h.note(ui, format!("Current: {track}"));
                }
            }
            8 => {
                h.note(ui, &self.gpu);
                h.note(ui, RichText::new("Images are rendered by wgpu. Decoding, prefetching and image processing run off the UI thread.").color(MUTED));
                for mode in [
                    Upscale::Linear,
                    Upscale::Nearest,
                    Upscale::Lanczos,
                    Upscale::Cas,
                    Upscale::Onnx,
                ] {
                    if h.button(ui, mode.label(), s.upscale == mode, true, false) {
                        s.upscale = mode;
                        *invalidate = true;
                    }
                }
                h.slider(
                    ui,
                    &mut s.upscale_factor,
                    1..=4,
                    "Lanczos / sharpening scale",
                );
                h.slider(
                    ui,
                    &mut s.sharpen,
                    0.0..=1.0,
                    "Adaptive sharpening strength",
                );
                h.note(ui, "ONNX / ESRGAN model");
                h.note(ui, s.model_path.display().to_string());
                if h.button(ui, "Choose ONNX model", false, true, false) {
                    actions.push("model");
                }
                h.note(
                    ui,
                    "ONNX Runtime library (1.22; DirectML package on Windows)",
                );
                h.note(ui, s.ort_library.display().to_string());
                if h.button(ui, "Choose runtime library", false, true, false) {
                    actions.push("runtime");
                }
                h.slider(
                    ui,
                    &mut s.tile_size,
                    32..=512,
                    "AI tile size (px, plus 8 px overlap)",
                );
                h.note(ui, RichText::new("Compatible models: one float32 RGB NCHW input and output, range 0–1, dynamic spatial dimensions, integer 1×–4× scale. Windows uses DirectML adapter 0. Supply your own model and matching runtime DLLs; restart after changing the runtime.").color(MUTED));
                if h.button(ui, "Apply rendering settings / retry", false, true, false) {
                    *invalidate = true;
                }
            }
            9 => {
                h.section(ui, "Profiles");
                h.section(ui, "Backups & restore");
                if let Some(store) = &self.store {
                    h.note(
                        ui,
                        format!("State: {}", crate::state::database_path(store).display()),
                    );
                    h.note(
                        ui,
                        format!(
                            "Primary backups: {}",
                            store.exe_dir.join("backups").display()
                        ),
                    );
                }
                h.note(ui, "Primary backups are stored beside the executable. That folder must be writable.");
                ui.horizontal(|ui| {
                    if h.button(ui, "Back up now", false, !self.backup_busy, false) {
                        actions.push("backup_now");
                    }
                    if h.button(
                        ui,
                        "Restore backup…",
                        false,
                        !self.busy && !self.view_pending && !self.backup_busy,
                        false,
                    ) {
                        actions.push("restore");
                    }
                });
                ui.separator();
                h.note(ui, "Additional backup location");
                h.note(ui, s.backup_extra.display().to_string());
                ui.horizontal(|ui| {
                    if h.button(ui, "Choose backup folder", false, true, false) {
                        actions.push("backup");
                    }
                    if h.button(ui, "Clear extra location", false, true, false) {
                        s.backup_extra = PathBuf::new();
                    }
                });
                h.check(ui, &mut s.backup_exit, "Back up when closing NoirView");
                h.check(
                    ui,
                    &mut s.backup_schedule,
                    "Enable scheduled backups (local time)",
                );
                ui.horizontal_wrapped(|ui| {
                    for (i, day) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
                        .iter()
                        .enumerate()
                    {
                        h.check(ui, &mut s.backup_days[i], day);
                    }
                });
                ui.horizontal(|ui| {
                    h.note(ui, "Time");
                    h.watch(
                        ui.add(egui::DragValue::new(&mut s.backup_hour).range(0..=23)),
                        false,
                    );
                    h.note(ui, ":");
                    h.watch(
                        ui.add(egui::DragValue::new(&mut s.backup_minute).range(0..=59)),
                        false,
                    );
                });
                h.slider(
                    ui,
                    &mut s.backup_keep,
                    1..=100,
                    "Backups to keep per location",
                );
                h.note(ui, "Missed schedules run at the next launch. Restore makes a safety copy of the current state first.");
                h.note(ui, "Backups contain settings, tags, favorites and view state, not the image or music files.");
            }
            10 => {
                h.section(ui, "Keyboard shortcuts");
                h.note(ui, "Click a field and type an egui key name, for example ArrowRight, Space, F11, or T.");
                for (action, key) in &mut s.shortcuts {
                    ui.horizontal(|ui| {
                        ui.set_min_width(280.);
                        h.note(ui, format!("{action:14}"));
                        let response = ui.add(egui::TextEdit::singleline(key).desired_width(150.));
                        h.watch(response, false);
                        if egui::Key::from_name(key).is_none() {
                            ui.colored_label(RED, "Unknown key");
                        }
                    });
                }
                if h.button(ui, "Reset shortcuts", false, true, false) {
                    s.shortcuts = Settings::default().shortcuts;
                }
                ui.separator();
                h.note(
                    ui,
                    "Escape closes the top dialog, settings or panel, then leaves fullscreen.",
                );
                h.section(ui, "Mouse gestures");
                h.check(
                    ui,
                    &mut s.gestures,
                    "Gestures with the right mouse button (drag; a short click opens the menu)",
                );
                h.note(
                    ui,
                    "The gesture closes the big image and the hover preview.",
                );
                h.check(
                    ui,
                    &mut s.gesture_opens_tile,
                    "The same gesture over a grid image opens it big",
                );
                h.check(ui, &mut s.gesture_trail, "Show the gesture trail");
                color_controls(ui, h, &mut s.gesture_trail_color, "Trail color");
                h.slider(
                    ui,
                    &mut s.gesture_trail_width,
                    1.0..=12.0,
                    "Trail width (px)",
                );
                ui.horizontal_wrapped(|ui| {
                    h.note(ui, "Gesture");
                    for g in crate::gesture::Gesture::ALL {
                        if h.button(ui, g.label(), s.gesture == g, s.gestures, false) {
                            s.gesture = g;
                        }
                    }
                });
                h.slider(
                    ui,
                    &mut s.gesture_min_px,
                    40.0..=600.0,
                    "Gesture minimum distance (px)",
                );
                h.slider(
                    ui,
                    &mut s.gesture_max_ms,
                    200..=3000,
                    "Gesture maximum duration (ms)",
                );
            }
            _ => {
                h.section(ui, "Settings window");
                h.slider(
                    ui,
                    &mut s.settings_search_opacity,
                    0.1..=1.0,
                    "Search outline opacity",
                );
                h.note(
                    ui,
                    "The outline round settings that match the search in the page column.",
                );
                h.section(ui, "Diagnostics");
                h.check(
                    ui,
                    &mut s.diagnostic_log,
                    "Write a diagnostic log (data/noirview.log)",
                );
                h.check(
                    ui,
                    &mut s.window_diagnostics,
                    "Window size diagnostics: draw the window's edges and the pointer as NoirView sees them",
                );
                h.note(
                    ui,
                    "Also writes window and surface sizes to data/noirview-window.log while on.",
                );
            }
        }
    }
    /// The profile choices as shown: None, Default, then the saved ones.
    fn profile_names(&self) -> Vec<String> {
        [NO_PROFILE.to_owned(), DEFAULT_PROFILE.to_owned()]
            .into_iter()
            .chain(self.lib.settings.profiles.iter().map(|p| p.name.clone()))
            .collect()
    }
    /// The bars' Profile button: the profiles as text above it.
    fn profile_button(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context, tip: bool) {
        let open = self.popup.is_some_and(|(p, _)| p == Popup::Profiles);
        let active = self.lib.settings.active_profile.clone();
        let label = if tip {
            if active.is_empty() {
                "Profile: none".to_owned()
            } else {
                format!("Profile: {active}")
            }
        } else {
            "Profile".to_owned()
        };
        let r = self
            .hover
            .icon_response(ui, Icon::Profile, &label, open, true, None, false);
        if r.clicked() {
            self.toggle_profiles_popup(r.rect);
        }
    }
    fn toggle_profiles_popup(&mut self, button: Rect) {
        if self.popup.is_some_and(|(p, _)| p == Popup::Profiles) {
            self.popup = None;
        } else {
            self.open_popup(Popup::Profiles, button);
        }
    }
    /// Switches to a profile: its display and interaction settings, the
    /// rest as they are. "" is no profile (nothing changes), "Default" the
    /// default settings.
    fn apply_profile(&mut self, name: &str) {
        if name.is_empty() {
            self.lib.settings.active_profile.clear();
            self.changed();
            self.notify("No profile");
            return;
        }
        let Some(values) = self.lib.settings.profile_target(name) else {
            return;
        };
        let profile = Profile {
            name: name.to_owned(),
            values,
        };
        let sort_before = (
            self.lib.settings.sort_key,
            self.lib.settings.sort_descending,
        );
        self.lib.settings.apply_profile_values(&profile.values);
        self.lib.settings.active_profile = profile.name.clone();
        self.sync_history_limit();
        self.images.invalidate();
        if sort_before
            != (
                self.lib.settings.sort_key,
                self.lib.settings.sort_descending,
            )
        {
            self.sort_files();
            if !self.lib.settings.sort_key.needs_metadata() {
                self.rebuild();
            }
        }
        self.changed();
        self.notify(format!("Profile: {}", profile.name));
    }
    /// The settings in use saved as a profile: a new one, or over `name`.
    fn save_profile(&mut self, name: &str) {
        let values = self.lib.settings.profile_values();
        let profiles = &mut self.lib.settings.profiles;
        match profiles.iter_mut().find(|p| p.name == name) {
            Some(p) => p.values = values,
            None => profiles.push(Profile {
                name: name.to_owned(),
                values,
            }),
        }
        self.lib.settings.active_profile = name.to_owned();
        self.changed();
    }
    fn rename_profile(&mut self, from: &str, to: &str) {
        let s = &mut self.lib.settings;
        if s.profiles.iter().any(|p| p.name == to) {
            return;
        }
        if let Some(p) = s.profiles.iter_mut().find(|p| p.name == from) {
            p.name = to.to_owned();
            if s.active_profile == from {
                s.active_profile = to.to_owned();
            }
            self.changed();
        }
    }
    fn delete_profile(&mut self, name: &str) {
        let s = &mut self.lib.settings;
        s.profiles.retain(|p| p.name != name);
        if s.active_profile == name {
            s.active_profile.clear();
        }
        self.changed();
    }
    /// Settings' profile row: the profile in use (and how many settings now
    /// differ from it), Save, Save as new…, Rename…, Delete….
    fn profile_row(&mut self, ui: &mut egui::Ui) {
        let active = self.lib.settings.active_profile.clone();
        let names: Vec<String> = self
            .lib
            .settings
            .profiles
            .iter()
            .map(|p| p.name.clone())
            .collect();
        // None and Default cannot be saved over, renamed or deleted.
        let own = names.contains(&active);
        let choices = self.profile_names();
        let differences = self.lib.settings.profile_differences();
        let shown = if active.is_empty() {
            NO_PROFILE.to_owned()
        } else if differences > 0 {
            format!("{active} — {differences} changed")
        } else {
            active.clone()
        };
        // Labels centred on the buttons' line.
        let row = Vec2::new(ui.available_width(), 26.);
        let centred = egui::Layout::left_to_right(egui::Align::Center);
        ui.allocate_ui_with_layout(row, centred, |ui| {
            ui.label("Profile");
            egui::ComboBox::from_id_salt("profile")
                .selected_text(shown)
                .width(220.)
                .show_ui(ui, |ui| {
                    for name in &choices {
                        let key = if name == NO_PROFILE {
                            String::new()
                        } else {
                            name.clone()
                        };
                        if ui.selectable_label(key == active, name).clicked() {
                            self.pending_profile = Some(key);
                        }
                    }
                });
            if self
                .hover
                .button(ui, "Save", false, own && differences > 0, false)
            {
                self.save_profile(&active);
            }
            if self.hover.button(ui, "Save as new…", false, true, false) {
                let mut n = names.len() + 1;
                while names.contains(&format!("Profile {n}")) {
                    n += 1;
                }
                self.profile_edit = Some((None, format!("Profile {n}")));
            }
            if self.hover.button(ui, "Rename…", false, own, false) {
                self.profile_edit = Some((Some(active.clone()), active.clone()));
            }
            if self.hover.button(ui, "Delete…", false, own, true) {
                self.dialog = Some(Dialog::DeleteProfile(active.clone()));
            }
        });
        let Some((renamed, mut text)) = self.profile_edit.clone() else {
            return;
        };
        let name = text.trim().to_owned();
        let reserved = [NO_PROFILE, DEFAULT_PROFILE]
            .iter()
            .any(|r| r.eq_ignore_ascii_case(&name));
        let taken = reserved
            || names
                .iter()
                .any(|n| *n == name && Some(n) != renamed.as_ref());
        let mut done = None;
        ui.allocate_ui_with_layout(row, centred, |ui| {
            ui.label(if renamed.is_some() {
                "New name"
            } else {
                "New profile"
            });
            let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(200.));
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ok = !name.is_empty() && !taken;
            if (self.hover.button(ui, "OK", false, ok, false) || enter) && ok {
                done = Some(true);
            }
            if self.hover.button(ui, "Cancel", false, true, false) {
                done = Some(false);
            }
            if taken {
                ui.label(RichText::new("A profile has that name").color(MUTED));
            }
        });
        match done {
            Some(true) => {
                match renamed {
                    Some(from) => self.rename_profile(&from, &name),
                    None => self.save_profile(&name),
                }
                self.profile_edit = None;
            }
            Some(false) => self.profile_edit = None,
            None => self.profile_edit = Some((renamed, text)),
        }
    }
    /// Settings edit the grid's or the Move grid's grid settings. With the
    /// Move grid closed its values are swapped in while Settings show them.
    fn edit_grid_settings_of(&mut self, move_grid: bool) {
        if self.move_grid.is_some() || move_grid == self.settings_move_target {
            return;
        }
        self.settings_move_target = move_grid;
        if move_grid {
            self.lib.settings.enter_move_grid_settings();
        } else {
            self.lib.settings.leave_move_grid_settings();
        }
        self.images.invalidate();
        self.changed();
    }
    /// Settings' Cancel: back to the settings as they were when the window
    /// opened, and close it.
    fn cancel_settings(&mut self) {
        // The snapshot is from before any swap.
        self.settings_move_target = false;
        if let Some(before) = self.settings_snapshot.take() {
            let sort = (before.sort_key, before.sort_descending)
                != (
                    self.lib.settings.sort_key,
                    self.lib.settings.sort_descending,
                );
            // Profiles saved, renamed or deleted meanwhile stay.
            let mut before = *before;
            before.profiles = std::mem::take(&mut self.lib.settings.profiles);
            before.favorite_places = std::mem::take(&mut self.lib.settings.favorite_places);
            if !before
                .profiles
                .iter()
                .any(|p| p.name == before.active_profile)
            {
                before.active_profile.clear();
            }
            self.lib.settings = before;
            self.sync_history_limit();
            self.images.invalidate();
            if sort {
                self.sort_files();
                self.rebuild();
            }
            self.changed();
        }
        self.settings = false;
    }
    fn clear_thumbnail_cache(&mut self, ctx: &egui::Context) {
        // The watcher's folders are done; a full check refills them.
        if let Some(handle) = &self.watch.handle {
            handle.check_now();
        }
        let cache = self.images.disk_cache.clone();
        let proxies = self.images.proxy_cache.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = cache.clear().and(proxies.clear());
            let _ = tx.send(Event::ThumbnailCacheCleared(result));
            ctx.request_repaint();
        });
    }
    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(dialog) = self.dialog.clone() {
            let mut close = false;
            egui::Modal::new(Id::new("confirmation")).show(ctx,|ui|{ui.set_max_width(540.);match dialog.clone(){
            Dialog::RemovePreset(n)=>{ui.heading(format!("Remove the {n}-image page button?"));
ui.horizontal(|ui|{if self.hover.button(ui,"Remove",false,true,true){self.lib.settings.grid_presets.retain(|p|*p!=n);self.changed();close=true;}
if self.hover.button(ui,"Cancel",false,true,false){close=true;}});},
            Dialog::RemoveTags(tags)=>{ui.heading(format!("Remove {} tag(s) from the palette?",tags.len()));
ui.label(tags.iter().take(12).map(|t|tag_label(t).into_owned()).collect::<Vec<_>>().join(", ")+if tags.len()>12{", …"}else{""});
ui.label("Keywords already saved in images are not changed.");
ui.horizontal(|ui|{if self.hover.button(ui,"Remove",false,true,true){self.lib.hide_palette_tags(&tags);self.selected_tags.retain(|t|!tags.contains(t));self.changed();close=true;}
if self.hover.button(ui,"Cancel",false,true,false){close=true;}});},
            Dialog::Delete(paths)=>{ui.heading("Move to Recycle Bin?");
ui.label(format!("{} image(s) and their metadata sidecars will be moved to Recycle Bin.",paths.len()));for line in delete_dialog_lines(&paths){ui.label(line);}ui.horizontal(|ui|{if self.hover.button(ui,"Move to Recycle Bin",false,!self.busy && !self.view_pending,true){self.delete(paths);close=true;}
if self.hover.button(ui,"Cancel",false,true,false){close=true;}});},
            Dialog::RestoreDeleted(paths)=>{ui.heading("Restore from Recycle Bin?");
ui.label(format!("{} image(s) deleted last will be moved back to their folders.",paths.len()));for line in delete_dialog_lines(&paths){ui.label(line);}ui.horizontal(|ui|{if self.hover.button(ui,"Restore",false,!self.busy && !self.view_pending,false){self.restore_deleted();close=true;}
if self.hover.button(ui,"Cancel",false,true,false){close=true;}});},
            Dialog::ResetSettingsPage(page)=>{ui.heading(format!("Reset {}?", SETTINGS_PAGES[page]));
ui.label("The settings on this page go back to their defaults. Folder lists, chosen folders and files, and profiles stay.");
ui.horizontal(|ui|{if self.hover.button(ui,"Reset",false,true,true){self.lib.settings.reset_values(SETTINGS_RESET[page]);self.sync_history_limit();self.images.invalidate();self.changed();close=true;}
if self.hover.button(ui,"Cancel",false,true,false){close=true;}});},
            Dialog::DeleteProfile(name)=>{ui.heading(format!("Delete the profile \"{name}\"?"));
ui.label("The settings in use stay as they are.");
ui.horizontal(|ui|{if self.hover.button(ui,"Delete",false,true,true){self.delete_profile(&name);close=true;}
if self.hover.button(ui,"Cancel",false,true,false){close=true;}});},
            Dialog::RoundDone(n)=>{ui.heading("All images shown");
ui.label(format!("All {n} images of the list have been shown. Browsing goes on with a new round."));
if self.hover.button(ui,"OK",false,true,false){close=true;}},
            Dialog::Move(mut plan)=>{
                // One width whatever the names: long ones are cut, not widening it.
                // The marked images as a grid: wider with more images.
                ui.set_width(440.);
                ui.heading(if plan.one {
                    "Move this image?".to_owned()
                } else if plan.marked {
                    format!("Move {} marked images?", plan.sources.len())
                } else if plan.selected {
                    format!("Move {} selected images?", plan.sources.len())
                } else {
                    "Move all folder images?".to_owned()
                });
                ui.horizontal(|ui| {
                    let dest = self.lib.settings.destination.clone();
                    let text = format!(
                        "Destination: {}",
                        if dest.as_os_str().is_empty() {
                            "(none)".to_owned()
                        } else {
                            dest.display().to_string()
                        }
                    );
                    // Long paths end in "…" instead of widening the dialog.
                    let width = (ui.available_width() - 190.).max(120.);
                    ui.allocate_ui(Vec2::new(width, 20.), |ui| {
                        ui.add(egui::Label::new(text).truncate())
                            .on_hover_text(dest.display().to_string());
                    });
                    if self.hover.button(ui, "Browse…", false, !self.busy, false) {
                        self.pick("destination");
                    }
                    // A favorite folder as the destination, listed above (or
                    // below) the button.
                    let open = self.popup.is_some_and(|(p, _)| p == Popup::Destination);
                    let r = ui.add_enabled(!self.busy, egui::Button::new("Favourites…").selected(open));
                    let rect = r.rect;
                    if self.hover.watch(r, false).clicked() {
                        if open {
                            self.popup = None;
                        } else {
                            self.open_popup(Popup::Destination, rect);
                        }
                    }
                });
                let mut keep = plan.keep_names;
                self.hover.check(ui, &mut keep, "Keep the original file names");
                if keep != plan.keep_names {
                    plan.keep_names = keep;
                    plan.replan(&self.lib.settings.destination);
                    self.dialog = Some(Dialog::Move(plan.clone()));
                }
                match &plan.items {
                    Ok(items) => {
                        ui.label(format!("{} image(s)", items.len()));
                        if let (Some(first), Some(last)) = (items.first(), items.last()) {
                            let name = |p: &Path| p.file_name().unwrap_or_default().to_string_lossy().into_owned();
                            // Long original names end in "…" instead of widening the dialog.
                            // One image: its name once.
                            let line = if items.len() == 1 {
                                name(&first.target)
                            } else {
                                format!("{} — {}", name(&first.target), name(&last.target))
                            };
                            ui.add(egui::Label::new(line.clone()).truncate()).on_hover_text(line);
                        }
                    }
                    Err(e) => {
                        ui.colored_label(RED, e);
                    }
                }
                ui.label(if self.lib.settings.verified_move {
                    "SHA-256 verification and recovery journal are enabled."
                } else {
                    "Basic copy validation is enabled. SHA-256 verification and recovery journal are off."
                });
                ui.horizontal(|ui| {
                    let ready = plan.items.as_ref().is_ok_and(|i| !i.is_empty());
                    let label = if plan.one { "Move image" } else { "Move images" };
                    if self.hover.button(ui, label, false, ready && !self.busy && !self.view_pending, true) {
                        if let Ok(items) = plan.items.clone() {
                            // The move starts before the view changes: a view
                            // being built holds moves back.
                            if operations::conflicts(&items) {
                                self.dialog = Some(Dialog::Conflict(items));
                            } else {
                                self.start_move(items, Conflict::Cancel);
                                close = true;
                            }
                            if plan.marked {
                                // The moved images leave the marks; the rest
                                // stay marked if only some were moved.
                                let moved: HashSet<&PathBuf> = plan.sources.iter().collect();
                                self.marked.retain(|p| !moved.contains(p));
                                if self.marked.is_empty() {
                                    self.stop_marking();
                                }
                                self.exit_move_grid();
                            } else if plan.selected && !plan.one {
                                self.move_mode = false;
                                self.move_selection.clear();
                                self.marks_changed(false);
                            }
                        }
                    }
                    if self.hover.button(ui, "Cancel", false, true, false) {
                        close = true;
                    }
                });
            }
            Dialog::Conflict(items)=>{ui.heading("Destination name conflict");
ui.label("A target image or sidecar already exists. Existing files will be preserved.");
ui.horizontal(|ui|{if self.hover.button(ui,"Skip",false,!self.busy && !self.view_pending,false){self.start_move(items.clone(),Conflict::Skip);close=true;}
if self.hover.button(ui,"Rename",false,!self.busy && !self.view_pending,false){self.start_move(items,Conflict::Rename);close=true;}
if self.hover.button(ui,"Cancel",false,true,false){close=true;}});},
            Dialog::Restore(path)=>{ui.heading("Restore this backup?");
ui.label(path.display().to_string());
ui.label("Current settings and library will be replaced after a safety copy is saved. Image files are unchanged.");
ui.horizontal(|ui|{if self.hover.button(ui,"Restore",false,!self.busy && !self.backup_busy,true){self.restore_backup(path);close=true;}
if self.hover.button(ui,"Cancel",false,true,false){close=true;}});},
            Dialog::Recover(folder)=>{ui.heading("Recover interrupted moves?");
ui.label("Recovery verifies copied files before removing any remaining source copies. The destination folder is locked for the duration of recovery.");
ui.label(folder.display().to_string());
ui.horizontal(|ui|{if self.hover.button(ui,"Recover moves",false,!self.busy && !self.view_pending,true){self.busy=true;
let tx=self.tx.clone();std::thread::spawn(move||{let r=operations::recover(&folder).map_err(|e|format!("{e:#}"));
let _=tx.send(Event::Recovered(r));});close=true;}
if self.hover.button(ui,"Cancel",false,true,false){close=true;}});},
            Dialog::ClearThumbnails=>{ui.heading("Clear the thumbnail cache?");
ui.label("Thumbnails and screen-sized first views are removed from disk and memory and made again when needed. Watched folders are checked again.");
ui.horizontal(|ui|{if self.hover.button(ui,"Clear thumbnail cache",false,true,true){self.clear_thumbnail_cache(ctx);close=true;}
if self.hover.button(ui,"Cancel",false,true,false){close=true;}});},
            Dialog::Rescan=>{ui.heading("Rescan the current folder?");
ui.label("The folder is listed again and changed images are read again. The view returns to the folder's start.");
ui.horizontal(|ui|{if self.hover.button(ui,"Rescan",false,!self.busy,false){if let Some(p)=self.folder.clone(){self.open(p);}close=true;}
if self.hover.button(ui,"Cancel",false,true,false){close=true;}});},
            Dialog::Reindex=>{ui.heading("Rebuild the keyword index?");
ui.label("Keywords of every image in the open folder are read again from the files. This can take a while for a large folder.");
ui.horizontal(|ui|{if self.hover.button(ui,"Rebuild",false,!self.busy,true){if let Some(path)=self.current.clone().or_else(||self.folder.clone()){self.open_indexed(path,true);}close=true;}
if self.hover.button(ui,"Cancel",false,true,false){close=true;}});},
            Dialog::Error(message)=>{ui.heading("Action needs attention");egui::ScrollArea::vertical().max_height(300.).show(ui,|ui|{ui.colored_label(RED,message);});if self.hover.button(ui,"Close",false,true,false){close=true;}},
        }});
            if close {
                self.dialog = None;
            }
        }
        if self.new_tag_open {
            let mut close = false;
            egui::Modal::new(Id::new("new_tag")).show(ctx, |ui| {
                ui.heading("Create My tag");
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.new_tag)
                        .id(Id::new("new_tag_name"))
                        .hint_text("Keyword or phrase")
                        .desired_width(320.),
                );
                if std::mem::take(&mut self.new_tag_focus) {
                    field.request_focus();
                }
                self.hover.watch(field, false);
                ui.horizontal(|ui| {
                    if self.hover.button(
                        ui,
                        "Create tag",
                        false,
                        !self.new_tag.trim().is_empty(),
                        false,
                    ) {
                        let tag = self.new_tag.trim().to_string();
                        if tag.len() > 1024 {
                            self.notify("Tag is too long (maximum 1024 bytes)");
                        } else {
                            self.lib.create_manual_tag(tag.clone());
                            self.selected_tags.insert(tag);
                            self.new_tag.clear();
                            self.changed();
                            close = true;
                        }
                    }
                    if self.hover.button(ui, "Cancel", false, true, false) {
                        close = true;
                    }
                });
            });
            if close {
                self.new_tag_open = false;
                self.new_tag_focus = false;
                self.new_tag.clear();
            }
        }
        if let Some(path) = self.keyword_loading.clone() {
            egui::Modal::new(Id::new("keyword_loading")).show(ctx, |ui| {
                ui.heading("Reading image keywords…");
                ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                ui.spinner();
                if self.hover.button(ui, "Cancel", false, true, false) {
                    self.keyword_loading = None;
                }
            });
        }
        if let Some((path, mut text)) = self.keyword_editor.clone() {
            let mut close = false;
            egui::Modal::new(Id::new("keyword_editor")).show(ctx, |ui| {
                ui.heading("Edit image keywords");
                ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                ui.label("One keyword or phrase per line. This only changes this image.");
                if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg")) {
                    ui.label("JPEG keywords also update Windows Tags and IPTC. Maximum 64 UTF-8 bytes per phrase; no semicolons.");
                }
                self.hover.watch(
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .desired_width(420.)
                            .desired_rows(10),
                    ),
                    false,
                );
                ui.horizontal(|ui| {
                    if self.hover.button(
                        ui,
                        "Save image keywords",
                        false,
                        !self.busy && !self.view_pending,
                        false,
                    ) {
                        let tags = text
                            .lines()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .collect();
                        self.save_tags(Some((path.clone(), tags)));
                        close = true;
                    }
                    if self.hover.button(ui, "Cancel", false, true, false) {
                        close = true;
                    }
                });
            });
            self.keyword_editor = if close { None } else { Some((path, text)) };
        }
    }
    fn start_move(&mut self, items: Vec<MoveItem>, policy: Conflict) {
        if self.view_pending {
            return;
        }
        self.busy = true;
        let verify = self.lib.settings.verified_move;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let report = operations::move_batch(items, verify, policy);
            let _ = tx.send(Event::Moved(report));
        });
    }
    fn info_window(&mut self, ctx: &egui::Context) {
        if !self.info {
            return;
        }
        let mut open = true;
        let mut close = false;
        egui::Window::new("Image information")
            .title_bar(false)
            .open(&mut open)
            .default_width(400.)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Image information");
                    if self.hover.button(ui, "Close", false, true, false) {
                        close = true;
                    }
                });
                if let Some(p) = self.current.clone() {
                    ui.label(p.display().to_string());
                    if let Ok(m) = std::fs::metadata(&p) {
                        ui.label(format!("Size: {:.2} MiB", m.len() as f64 / 1048576.));
                        if let Ok(modified) = m.modified() {
                            let date: chrono::DateTime<chrono::Local> = modified.into();
                            ui.label(format!("Modified: {}", date.format("%Y-%m-%d %H:%M:%S")));
                        }
                    }
                    let key = self.images.key(p.clone(), false);
                    if let Some((_, dim)) = self.images.texture(&key) {
                        ui.label(format!("Dimensions (oriented): {} × {}", dim[0], dim[1]));
                    }
                    ui.label(format!(
                        "Format: {}",
                        p.extension()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_uppercase()
                    ));
                    let mut m = self.lib.photos.get(&p).cloned().unwrap_or_default();
                    ui.separator();
                    ui.label("Keywords");
                    ui.label(if m.tags.is_empty() {
                        "No keywords".into()
                    } else {
                        m.tags.iter().cloned().collect::<Vec<_>>().join(" · ")
                    });
                    let old = (m.favorite, m.elite);
                    ui.add_enabled_ui(!self.busy && !self.view_pending, |ui| {
                        self.hover.check(ui, &mut m.favorite, "Favorite");
                        self.hover.check(ui, &mut m.elite, "Elite");
                    });
                    if m.elite {
                        m.favorite = true;
                    }
                    if old != (m.favorite, m.elite) {
                        self.lib.photos.insert(p.clone(), m);
                        if self.filter.active() {
                            self.refresh_index_filter();
                        }
                        self.changed();
                    }
                    if self.hover.button(
                        ui,
                        "Edit image keywords",
                        false,
                        !self.busy && !self.view_pending,
                        false,
                    ) {
                        self.edit_keywords(p);
                    }
                }
            });
        self.info = open && !close;
    }
    fn help_window(&mut self, ctx: &egui::Context) {
        if !self.help {
            return;
        }
        let mut open = true;
        let mut close = false;
        egui::Window::new("Welcome to NoirView").title_bar(false).open(&mut open).default_width(460.).show(ctx,|ui|{ui.horizontal(|ui|{ui.heading("Your images, uninterrupted.");if self.hover.button(ui,"Close",false,true,false){close=true;}});
ui.label("Open a folder or drop an image into the window. Scroll to browse. Double-click the image for fullscreen.");
ui.separator();
ui.label("Move to the bottom activation edge to reveal the floating toolbar. Leave the edge and return to hide it.");
ui.label("Tags: + creates My tags. Additional includes image keywords with yellow borders. Tags wrap upward at the configured maximum width. Select words, then Save keywords. Lock keeps the panel visible across images.");
ui.label("Grid: click to mark images for deletion, or enable Tags to select images for batch tagging. Red-bordered images cannot receive tags.");
ui.label("Slideshow: the top edge pauses / resumes. Side edges and the mouse wheel still browse.");
ui.separator();for(action,key)in &self.lib.settings.shortcuts{ui.label(format!("{key:12}  {action}"));}ui.separator();
ui.label("Animated GIFs play in the single image view and in the big image from the grid; grid thumbnails show the first frame. AI upscaling requires a compatible model and ONNX Runtime supplied separately.");});
        self.help = open && !close;
    }
    fn keyboard(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.hover.pending = None;
            if self.recent_open {
                self.recent_open = false;
            } else if self.preview.is_some() {
                self.preview = None;
            } else if self.grid_viewer.is_some() {
                self.close_grid_viewer();
            } else if self.new_tag_open {
                self.new_tag_open = false;
                self.new_tag_focus = false;
                self.new_tag.clear();
            } else if self.keyword_loading.is_some() {
                self.keyword_loading = None;
            } else if self.keyword_editor.is_some() {
                self.keyword_editor = None;
            } else if self.dialog.is_some() {
                self.dialog = None;
            } else if self.settings {
                self.settings = false;
            } else if self.help {
                self.help = false;
            } else if self.info {
                self.info = false;
            } else if self.panel != Panel::None {
                self.panel = Panel::None;
            } else if self.bar && !self.bar_locked() {
                self.bar = false;
            } else if self.fullscreen {
                self.toggle_fullscreen(ctx);
            }
            return;
        }
        if ctx.wants_keyboard_input()
            || self.dialog.is_some()
            || self.new_tag_open
            || self.keyword_editor.is_some()
            || self.keyword_loading.is_some()
            || self.settings
        {
            return;
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) {
            self.request_restore();
            return;
        }
        let actions: Vec<_> = self
            .lib
            .settings
            .shortcuts
            .iter()
            .filter(|(_, key)| {
                egui::Key::from_name(key)
                    .map(|k| {
                        ctx.input(|i| i.key_pressed(k) && !i.modifiers.command && !i.modifiers.alt)
                    })
                    .unwrap_or(false)
            })
            .map(|(a, _)| a.clone())
            .collect();
        for action in actions {
            match action.as_str() {
                "next" => self.navigate(true),
                "previous" => self.navigate(false),
                "fullscreen" => self.toggle_fullscreen(ctx),
                "open" => self.pick("open"),
                "settings" => self.settings = true,
                "tags" => self.switch_panel(Panel::Tags),
                "thumbnails" => self.toggle_grid(),
                "delete" => {
                    let paths = if self.grid {
                        self.delete_selection.iter().cloned().collect()
                    } else {
                        self.current.iter().cloned().collect()
                    };
                    self.request_delete(paths);
                }
                "slideshow" => {
                    self.toggle_slideshow();
                }
                "info" => self.info = !self.info,
                "bar" => self.bar = !self.bar,
                _ => {}
            }
        }
    }
    /// Folder watcher upkeep, once per frame: take the watcher over from a
    /// background process, run the engine for the watched folders, keep the
    /// background configuration and the sign-in entry current.
    fn sync_watcher(&mut self, ctx: &egui::Context) {
        use crate::watch::{control, Handle, Setup};
        let Some(data) = self
            .store
            .as_ref()
            .filter(|_| self.persist_allowed)
            .and_then(|s| s.path.parent().map(Path::to_path_buf))
        else {
            return;
        };
        let Some(dir) = control::dir() else {
            return;
        };
        if !self.watch.requested {
            // Always: an older version's background process must let go of
            // its folder, and two writers must never share the caches.
            self.watch.requested = true;
            let (tx, ctx, dir) = (self.tx.clone(), ctx.clone(), dir.clone());
            std::thread::spawn(move || {
                let lock = control::take_over(&dir, Duration::from_secs(15));
                let _ = tx.send(Event::WatchOwner(lock));
                ctx.request_repaint();
            });
        }
        let s = &self.lib.settings;
        let enabled = s.watch_folders && !s.watched_folders.is_empty();
        let background = enabled && s.watch_background;
        let exe = std::env::current_exe().ok();
        if self.watch.autostart != Some(background) {
            self.watch.autostart = Some(background);
            if let Some(exe) = &exe {
                if let Err(e) = control::sync_autostart(exe, background) {
                    self.watch.error = Some(format!("{e:#}"));
                }
            }
        }
        let cache_root = thumbnail_cache_root(&self.cache_dir_in_use, Some(&data));
        let config = control::Config {
            version: env!("CARGO_PKG_VERSION").into(),
            exe: exe.clone().unwrap_or_default(),
            folders: s.watched_folders.clone(),
            settings: s.clone(),
            cache_root,
            keyword_db: Some(data.join("keyword-index.sqlite3")),
            state_db: data.join("watch-state.sqlite3"),
            background,
        };
        let bytes = serde_json::to_vec(&(
            &config.folders,
            config.background,
            &config.cache_root,
            s.watch_folders,
            s.thumbnail_cache_mb,
            s.watch_pace,
        ))
        .unwrap_or_default();
        if bytes != self.watch.config && self.watch.lock.is_some() {
            self.watch.config = bytes;
            if let Err(e) = control::write_config(&dir, &config) {
                self.watch.error = Some(format!("Watcher settings could not be saved: {e:#}"));
            }
        }
        let cache = self.images.disk_cache.clone();
        // Whether the journal would be used now (service installed later or
        // removed): checked every 30 s, the engine restarts on a change.
        if self
            .watch
            .journal_checked
            .is_none_or(|t| t.elapsed() > Duration::from_secs(30))
        {
            self.watch.journal_checked = Some(Instant::now());
            self.watch.journal = crate::watch::journal_wanted(&config.settings);
        }
        let journal = self.watch.journal && config.settings.watch_use_journal;
        let current = self.watch.handle.as_ref().is_some_and(|h| {
            h.folders == config.folders
                && h.pace == config.settings.watch_pace
                && h.journal == journal
                && !h.finished()
                && self
                    .watch
                    .cache
                    .as_ref()
                    .is_some_and(|c| std::sync::Arc::ptr_eq(c, &cache))
        });
        let wanted = enabled && self.watch.lock.is_some() && !self.closing;
        if (!wanted || !current) && self.watch.handle.is_some() {
            let old = self.watch.handle.take().unwrap();
            std::thread::spawn(move || old.stop());
        }
        if wanted && self.watch.handle.is_none() {
            let setup = Setup {
                folders: config.folders.clone(),
                settings: config.settings.clone(),
                cache: cache.clone(),
                keyword_db: config.keyword_db.clone(),
                state_db: config.state_db.clone(),
                owner: "NoirView",
                gate: self.images.background_gate(),
                stop_requested: std::sync::Arc::new(|| false),
            };
            let (source, warnings) = crate::watch::source_for(&config.folders, &config.settings);
            match Handle::start(setup, source) {
                Ok(handle) => {
                    handle.status.lock().unwrap().warnings.extend(warnings);
                    self.watch.handle = Some(handle);
                    self.watch.cache = Some(cache);
                    self.watch.error = None;
                }
                Err(e) => {
                    // Not retried every frame; a settings change tries again.
                    self.lib.settings.watch_folders = false;
                    self.watch.error = Some(format!("Folder watcher could not start: {e:#}"));
                }
            }
        }
    }
    /// Asks Windows and the service for their state (in the background).
    fn usn_refresh(&mut self) {
        if self.usn.busy {
            return;
        }
        self.usn.busy = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let state = crate::usn::state();
            let version = match crate::usn::call(&crate::usn::protocol::Request::Hello) {
                // Another message protocol: shown as needing an update.
                Ok(crate::usn::protocol::Response::Hello { version, protocol }) => {
                    Some(if protocol == crate::usn::protocol::PROTOCOL {
                        version
                    } else {
                        format!("{version}, incompatible")
                    })
                }
                _ => None,
            };
            let _ = tx.send(Event::UsnStatus(state, version));
        });
    }
    /// Install / remove / test, reported in Settings when done.
    fn usn_action(&mut self, action: &'static str) {
        let Some(exe_dir) = self.store.as_ref().map(|s| s.exe_dir.clone()) else {
            return;
        };
        let volumes: Vec<String> = {
            let mut v: Vec<String> = self
                .lib
                .settings
                .watched_folders
                .iter()
                .filter_map(|f| crate::usn::protocol::volume_of(&f.path.to_string_lossy()))
                .collect();
            v.sort();
            v.dedup();
            if v.is_empty() {
                v.push("C:".into());
            }
            v
        };
        self.usn.busy = true;
        self.usn.message = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            use crate::usn::protocol::{Request, Response};
            let message = match action {
                "usn_install" => match crate::usn::install(&exe_dir) {
                    Ok(()) => "Service installed and started.".to_string(),
                    Err(e) => format!("{e:#}"),
                },
                "usn_remove" => match crate::usn::uninstall(&exe_dir) {
                    Ok(()) => "Service removed.".to_string(),
                    Err(e) => format!("{e:#}"),
                },
                _ => volumes
                    .iter()
                    .map(|volume| {
                        match crate::usn::call(&Request::Test {
                            volume: volume.clone(),
                        }) {
                            Ok(Response::Test {
                                journal,
                                records,
                                ms,
                            }) => format!(
                                "{volume} journal {} MiB: {records} records read in {ms} ms",
                                journal.max_size >> 20
                            ),
                            Ok(Response::Error { message }) => format!("{volume}: {message}"),
                            Ok(other) => format!("{volume}: unexpected answer {other:?}"),
                            Err(e) => format!("{volume}: {e:#}"),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            let _ = tx.send(Event::UsnDone(message));
        });
    }
    /// The window drew with the chosen graphics backend: it works. If the
    /// last start with a chosen backend never got here, `main` used the
    /// automatic choice: say so and go back to it.
    fn check_backend(&mut self) {
        if self.backend_checked {
            return;
        }
        let Some(data) = self
            .store
            .as_ref()
            .and_then(|s| s.path.parent().map(Path::to_path_buf))
        else {
            return;
        };
        self.backend_checked = true;
        let _ = std::fs::remove_file(data.join("backend-trying"));
        // OpenGL started: the automatic choice may use it again.
        if self.gpu.contains("OpenGL") {
            let _ = std::fs::remove_file(data.join("opengl-unavailable"));
        }
        if let Ok(failed) = std::fs::read_to_string(data.join("backend-failed")) {
            let _ = std::fs::remove_file(data.join("backend-failed"));
            self.lib.settings.graphics_backend.clear();
            self.changed();
            self.notify(backend_failed_notice(&failed));
        }
    }
    /// Startup options that must be known before the window exists.
    fn sync_display_file(&mut self) {
        let low = self.lib.settings.low_latency_display;
        let backend = self.lib.settings.graphics_backend.clone();
        let remember = self.lib.settings.remember_window;
        let wanted = (low, backend.clone(), remember);
        if self.display_written.as_ref() == Some(&wanted) || !self.persist_allowed {
            return;
        }
        if let Some(data) = self.store.as_ref().and_then(|s| s.path.parent()) {
            self.display_written = Some(wanted);
            let display = crate::storage::Display {
                low_latency: low,
                backend,
                remember_window: remember,
            };
            let _ = crate::storage::write_display(data, &display);
        }
    }
    /// Closing: the background process takes over the watched folders.
    fn hand_watcher_to_background(&mut self) -> impl FnOnce() + Send + 'static {
        let handle = self.watch.handle.take();
        let lock = self.watch.lock.take();
        let s = &self.lib.settings;
        let start = s.watch_folders && s.watch_background && !s.watched_folders.is_empty();
        let exe = std::env::current_exe().ok();
        move || {
            if let Some(handle) = handle {
                handle.stop();
            }
            if let (Some(lock), Some(exe)) = (lock, exe) {
                drop(lock);
                if start {
                    if let Some(dir) = crate::watch::control::dir() {
                        crate::watch::control::log(
                            &dir,
                            "NoirView closed: starting the background watcher",
                        );
                    }
                    let _ = crate::watch::control::spawn_background(&exe);
                }
            }
        }
    }
    fn memory_tight(&self) -> bool {
        self.memory_guard.level >= crate::memory::Pressure::Tight
    }
    fn memory_critical(&self) -> bool {
        self.memory_guard.level == crate::memory::Pressure::Critical
    }
    /// The image budget for the chosen Memory use on this machine (Custom:
    /// the value set by hand).
    fn apply_memory_use(&mut self) {
        let s = &mut self.lib.settings;
        let choice = s.memory_use();
        s.cache_mb = crate::memory::image_budget_mb(choice, s.cache_mb, &self.machine);
    }
    /// Once a second: reads the machine's memory, sizes the image budget and
    /// sets how much of it may be used now (memory pressure).
    fn memory_check(&mut self) {
        let now = Instant::now();
        if self
            .memory_checked
            .is_some_and(|t| now.duration_since(t) < Duration::from_secs(1))
        {
            return;
        }
        self.memory_checked = Some(now);
        self.machine = self.memory_probe.read();
        self.apply_memory_use();
        let s = &self.lib.settings;
        let reading = crate::memory::Reading {
            process: crate::sysinfo::process_memory(),
            limit: crate::memory::process_limit(s.memory_use(), s.cache_mb, &self.machine),
            ram_total: self.machine.ram_total,
            ram_available: self.machine.ram_available,
            gpu_usage: self.machine.gpu.as_ref().and_then(|g| g.usage),
            gpu_budget: self.machine.gpu.as_ref().and_then(|g| g.budget),
        };
        if let Some(level) = self.memory_guard.update(reading, now) {
            use crate::memory::Pressure;
            match level {
                Pressure::Tight => self.notify(
                    "Memory is getting low: fewer images kept ready, background work paused",
                ),
                Pressure::Critical => self.notify(
                    "High memory use: only the image on screen is kept, background work paused",
                ),
                Pressure::Normal => {}
            }
            let summary = format!(
                "Memory {}: process {} of {}, RAM free {}, graphics card {}",
                level.label(),
                reading.process.map_or("-".into(), crate::memory::size),
                crate::memory::size(reading.limit),
                reading
                    .ram_available
                    .map_or("-".into(), crate::memory::size),
                match (reading.gpu_usage, reading.gpu_budget) {
                    (Some(u), Some(b)) =>
                        format!("{} of {}", crate::memory::size(u), crate::memory::size(b)),
                    _ => "-".into(),
                }
            );
            self.log_line(&summary);
        }
        self.images
            .set_pressure(self.memory_guard.level.budget_share());
    }
    /// Once a second: memory guard. Every five seconds, when enabled: one line in
    /// data/noirview.log (rotated at 4 MiB) for diagnosing slowdowns.
    fn health(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.health_checked) < Duration::from_secs(1) {
            return;
        }
        self.health_checked = now;
        let memory = self.memory_guard.last.process;
        if !self.lib.settings.diagnostic_log
            || now.duration_since(self.health_logged) < Duration::from_secs(5)
        {
            return;
        }
        self.health_logged = now;
        let Some(path) = self
            .store
            .as_ref()
            .and_then(|s| s.path.parent())
            .map(|p| p.join("noirview.log"))
        else {
            return;
        };
        let page = match &self.grid_timer {
            Some((_, _, Some((took, prepared)))) => format!(
                "{} ms prepared {}",
                took.as_millis(),
                prepared.map_or("-".into(), |p| format!("{} ms", p.as_millis()))
            ),
            Some((_, started, None)) => format!("loading {} ms", started.elapsed().as_millis()),
            None => "-".into(),
        };
        let index = self.index_progress.as_ref().map_or("-".into(), |p| {
            format!("{}/{} read {}", p.done, p.total, p.read)
        });
        let thumbs = crate::decode_stats::take().unwrap_or_else(|| "-".into());
        let paint = paint_summary(&eframe::noirview_diag::take_paint_stats());
        let line = format!(
            "{} mem={}MiB textures={}MiB budget={}MiB grid={} page={} index={} memory_guard={} ram_free={} gpu={} thumbs=[{}] paint=[{}]\n",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            memory.map_or(0, |m| m / (1024 * 1024)),
            self.images.memory_mb(),
            self.lib.settings.cache_mb,
            self.grid,
            page,
            index,
            self.memory_guard.level.label(),
            self.machine
                .ram_available
                .map_or("-".into(), crate::memory::size),
            self.machine.gpu.as_ref().map_or("-".into(), |g| format!(
                "{}/{}",
                g.usage.map_or("-".into(), crate::memory::size),
                g.budget.map_or("-".into(), crate::memory::size)
            )),
            thumbs,
            paint,
        );
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 4 * 1024 * 1024) {
            let _ = std::fs::rename(&path, path.with_extension("log.old"));
        }
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }
    /// Restart the background thumbnail pass when the image list or its
    /// settings change. Order: from the current page / image to the end, then
    /// the beginning.
    fn sync_pregen(&mut self) {
        let s = &self.lib.settings;
        let key = (
            self.visible.len(),
            self.visible.first().cloned(),
            self.visible.last().cloned(),
            s.loading_mode,
            s.thumbnail_cache_mb,
        );
        if self.pregen_key.as_ref() == Some(&key) {
            return;
        }
        self.pregen_key = Some(key);
        let start = if self.grid {
            self.grid_start
        } else {
            self.current
                .as_ref()
                .and_then(|p| self.visible.position(p))
                .unwrap_or(0)
        }
        .min(self.visible.len());
        let paths: Vec<PathBuf> = self.visible[start..]
            .iter()
            .chain(&self.visible[..start])
            .cloned()
            .collect();
        self.images.pregenerate(paths, &self.lib.settings);
    }
    fn image_demand(&mut self, ctx: &egui::Context) -> HashSet<crate::imaging::Key> {
        self.sync_history_limit();
        self.memory_check();
        self.images
            .configure(&self.lib.settings, ctx.input(|i| i.max_texture_side) as u32);
        self.sync_pregen();
        if self
            .grid_viewer
            .as_ref()
            .is_some_and(|p| !self.view_pending && self.visible.position(p).is_none())
        {
            self.close_grid_viewer();
        }
        let mut high = Vec::new();
        let mut low = Vec::new();
        if self.view_pending {
            if let Some(path) = &self.current {
                high.push(self.images.key(path.clone(), false));
            }
        } else if self.grid && self.grid_viewer.is_none() {
            let count = self.grid_count();
            if let Some(p) = &self.preview {
                high.push(self.images.key(p.clone(), false));
            }
            let (page, next) = self.page_paths();
            high.extend(page.into_iter().map(|p| self.images.key(p, true)));
            // Prepare the next page first, then the previous one, in the background.
            let next = next.iter();
            let previous = self
                .visible
                .iter()
                .skip(self.grid_start.saturating_sub(count))
                .take(self.grid_start.min(count));
            low.extend(
                next.chain(previous)
                    .map(|p| self.images.key(p.clone(), true)),
            );
        } else if let Some(path) = &self.current {
            let full = self.images.key(path.clone(), false);
            if self.lib.settings.jpeg_preview && !self.images.textures.contains_key(&full) {
                let thumb = self.images.key(path.clone(), true);
                let aggressive =
                    self.lib.settings.loading_mode == crate::model::LoadingMode::Aggressive;
                // Aggressive: the preview may be a stored screen-sized proxy,
                // sharper than a grid tile already in memory.
                if self.images.textures.contains_key(&thumb) && !aggressive {
                    high.push(thumb);
                } else {
                    if self.images.textures.contains_key(&thumb) {
                        high.push(thumb);
                    }
                    high.push(self.images.preview_key(path.clone()));
                }
            }
            // The full image after its preview: a cached thumbnail is ready in
            // about a millisecond, a full decode can take a second on a hard disk.
            high.push(full);
            let order = if self.slideshow {
                self.lib.settings.slideshow_order
            } else {
                self.lib.settings.order
            };
            low.extend(
                self.nav
                    .prefetch(
                        &self.visible,
                        Some(path),
                        order,
                        // Tight memory: only the nearest images ahead and behind.
                        if self.memory_tight() {
                            self.lib.settings.prefetch_ahead.min(2)
                        } else {
                            self.lib.settings.prefetch_ahead
                        },
                        if self.memory_tight() {
                            self.lib.settings.prefetch_behind.min(1)
                        } else {
                            self.lib.settings.prefetch_behind
                        },
                    )
                    .into_iter()
                    .map(|p| self.images.key(p, false)),
            );
        }
        if self.recent_open {
            let recent: Vec<PathBuf> = self.recent.iter().cloned().collect();
            high.extend(recent.into_iter().map(|p| self.images.key(p, true)));
        }
        self.health();
        if self.memory_critical() || self.grid_viewer.is_some() {
            // Safety guard: keep showing what is on screen, skip speculative work.
            low.clear();
        }
        self.images.plan(
            &high,
            &low,
            &self.lib.settings,
            ctx.input(|i| i.max_texture_side) as u32,
        );
        high.into_iter().collect()
    }
    fn edges(&mut self, ctx: &egui::Context) {
        // A locked toolbar is open in its view, also right after switching
        // views and at start.
        if self.bar_locked() {
            self.bar = true;
        }
        let now = Instant::now();
        let pos = ctx.pointer_hover_pos();
        let screen = ctx.screen_rect();
        let s = self.lib.settings.clone();
        let popup = ctx.memory(|m| m.any_popup_open());
        let magnifying = s.magnifier && ctx.input(|i| i.pointer.primary_down());
        let blocked = self.settings
            || self.dialog.is_some()
            || self.keyword_editor.is_some()
            || self.keyword_loading.is_some()
            || self.new_tag_open
            || self.busy
            || self.view_pending
            || self.help
            || self.info
            || popup;
        // In a window the pointer easily overshoots the bottom edge (no
        // screen edge stops it): leaving through the bottom strip also opens
        // the bar (but never closes it).
        let left_bottom = pos.is_none()
            && !self.fullscreen
            && !self.bar
            && self
                .last_pointer
                .is_some_and(|p| p.y >= screen.bottom() - s.bar_edge_px.max(BAR_EXIT_PX));
        if pos.is_some() {
            self.last_pointer = pos;
        }
        if pos.is_some_and(|p| p.y < screen.bottom() - s.bar_edge_px) {
            self.bar_edge_spent = false;
        }
        if self.bar_dwell.update(
            !blocked
                && !magnifying
                && !self.bar_edge_spent
                && (left_bottom
                    || pos
                        .map(|p| p.y >= screen.bottom() - s.bar_edge_px)
                        .unwrap_or(false)),
            Duration::from_millis(s.bar_delay_ms),
            now,
        ) && !(self.bar && self.bar_locked())
        {
            self.bar = !self.bar;
            self.bar_edge_spent = true;
        }
        // The edges work with the bar open too, but not while the pointer is
        // over the bar or another floating control (like the wheel).
        let over_ui = pos
            .and_then(|p| ctx.layer_id_at(p))
            .is_some_and(|layer| layer.order != egui::Order::Background);
        let side = !blocked && !magnifying && !over_ui && (s.edges || self.slideshow);
        // Large strips stay apart: each at most 45 % of the window.
        let width = if self.slideshow {
            s.slideshow_side_px
        } else {
            s.side_px
        }
        .min(screen.width() * EDGE_MAX_SHARE);
        let delay = Duration::from_millis(if self.slideshow {
            s.slideshow_side_ms
        } else {
            s.side_ms
        });
        if self.left_dwell.update(
            side && pos.map(|p| p.x <= screen.left() + width).unwrap_or(false),
            delay,
            now,
        ) {
            self.navigate(false);
        }
        if self.right_dwell.update(
            side && pos.map(|p| p.x >= screen.right() - width).unwrap_or(false),
            delay,
            now,
        ) {
            self.navigate(true);
        }
        let top_width = if self.slideshow {
            s.slideshow_top_px
        } else {
            s.top_px
        }
        .min(screen.height() * EDGE_MAX_SHARE);
        // The corners belong to the side strips only while those are on.
        let corner = if s.edges || self.slideshow { width } else { 0. };
        let top_delay = Duration::from_millis(if self.slideshow {
            s.slideshow_top_ms
        } else {
            s.top_ms
        });
        if self.top_dwell.update(
            !blocked
                && !magnifying
                && !over_ui
                && (self.slideshow || s.top_delete)
                && pos
                    .map(|p| {
                        p.y <= screen.top() + top_width
                            && p.x > screen.left() + corner
                            && p.x < screen.right() - corner
                    })
                    .unwrap_or(false),
            top_delay,
            now,
        ) {
            if self.slideshow {
                self.paused = !self.paused;
                self.last_slide = now;
            } else if let Some(p) = self
                .current
                .clone()
                .filter(|_| !self.grid && self.grid_viewer.is_none())
            {
                self.request_delete(vec![p]);
            }
        }
        let big = self.grid_viewer.is_some();
        let wheel_active = !blocked
            && (!big || self.big_browse())
            && (!self.grid || big || s.grid_wheel_pages || self.move_grid_all())
            && !self.visible.is_empty()
            && pos
                .and_then(|p| ctx.layer_id_at(p))
                .is_some_and(|layer| layer.order == egui::Order::Background);
        self.wheel.set_up_next(s.wheel_up_next);
        let points_per_line = ctx.options(|options| options.line_scroll_speed);
        let steps = ctx.input(|input| {
            self.wheel
                .steps(&input.events, wheel_active, points_per_line)
        });
        let order = if self.slideshow {
            s.slideshow_order
        } else {
            s.order
        };
        // Random and Shuffle order can give a new image in both directions.
        let always_next = s.random_wheel_both
            && matches!(
                order,
                crate::model::Order::Random | crate::model::Order::Shuffle
            );
        for next in steps {
            if self.grid && !big {
                self.grid_page(next);
            } else {
                self.navigate(next || always_next);
            }
        }
        if self.wheel.has_pending() {
            ctx.request_repaint();
        }
        for (dwell, interval) in [
            (&self.bar_dwell, Duration::from_millis(s.bar_delay_ms)),
            (&self.left_dwell, delay),
            (&self.right_dwell, delay),
            (&self.top_dwell, top_delay),
        ] {
            if let Some(after) = dwell.timer(interval, now) {
                ctx.request_repaint_after(after);
            }
        }
        let suspended = self.paused || magnifying || blocked || self.grid;
        if self.slide_suspended && !suspended {
            self.last_slide = now;
        }
        self.slide_suspended = suspended;
        if self.slideshow && !suspended {
            let interval = crate::repaint::slide_interval(s.slideshow_secs);
            if now.saturating_duration_since(self.last_slide) >= interval {
                self.navigate(true);
            }
            ctx.request_repaint_after(crate::repaint::remaining(
                self.last_slide,
                interval,
                Instant::now(),
            ));
        }
        if blocked {
            self.last_slide = now;
        }
    }
}
fn color_controls(ui: &mut egui::Ui, h: &mut Hover, value: &mut [u8; 4], label: &str) {
    ui.push_id(label, |ui| {
        let id = ui.make_persistent_id("expanded");
        let mut open = ui.data_mut(|d| d.get_temp::<bool>(id).unwrap_or(false));
        ui.horizontal(|ui| {
            // A swatch with a colour picker, or the sliders.
            let mut color = Color32::from_rgba_unmultiplied(value[0], value[1], value[2], value[3]);
            let picker = egui::widgets::color_picker::color_edit_button_srgba(
                ui,
                &mut color,
                egui::widgets::color_picker::Alpha::OnlyBlend,
            );
            if picker.changed() {
                *value = color.to_srgba_unmultiplied();
            }
            if h.button(ui, label, open, true, false) {
                open = !open;
                ui.data_mut(|d| d.insert_temp(id, open));
            }
        });
        if open {
            for (i, name) in ["Red", "Green", "Blue", "Alpha"].iter().enumerate() {
                h.slider(ui, &mut value[i], 0..=255, name);
            }
        }
    });
}

fn order_controls(ui: &mut egui::Ui, h: &mut Hover, order: &mut Order, id: &str) {
    ui.push_id(id, |ui| {
        ui.horizontal_wrapped(|ui| {
            for mode in [Order::Sequential, Order::Random, Order::Shuffle] {
                if h.button(ui, mode.label(), *order == mode, true, false) {
                    *order = mode;
                }
            }
        });
    });
}
impl eframe::App for Viewer {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Unit tests drive these process-wide gates themselves.
        if !cfg!(test) {
            let s = &self.lib.settings;
            if !raw.events.is_empty() && !s.cache_while_active {
                crate::pregen::note_activity();
            }
            crate::pregen::set_blocked(
                (!raw.focused && !s.cache_when_unfocused)
                    || self.index_progress.is_some()
                    || self.memory_tight(),
            );
            crate::pregen::note_frame(
                !raw.events.is_empty(),
                raw.focused,
                self.index_progress.is_some() || self.memory_tight(),
                s.watch_while_active,
                s.watch_when_unfocused,
            );
        }
        if let Some(pos) = self.hover.pending.take() {
            // The click lands where the dwell finished, also when the pointer
            // kept moving (a quick pass over a control); a real press or
            // leaving the window cancels it.
            let cancelled = raw.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::PointerButton { .. } | egui::Event::PointerGone
                )
            });
            if !cancelled && self.hover.enabled {
                let now = raw.events.iter().rev().find_map(|e| match e {
                    egui::Event::PointerMoved(p) => Some(*p),
                    _ => None,
                });
                raw.events.push(egui::Event::PointerMoved(pos));
                for pressed in [true, false] {
                    raw.events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: raw.modifiers,
                    });
                }
                if let Some(now) = now {
                    raw.events.push(egui::Event::PointerMoved(now));
                }
            }
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.parent_window.is_none() {
            use wgpu::rwh::{HasDisplayHandle as _, HasWindowHandle as _};
            if let (Ok(w), Ok(d)) = (frame.window_handle(), frame.display_handle()) {
                self.parent_window = Some(ParentWindow {
                    window: w.as_raw(),
                    display: d.as_raw(),
                });
            }
        }
        if std::env::var_os("NOIRVIEW_FRAME_TRACE").is_some() {
            eprintln!("noirview frame");
        }
        if self.startup_frame(ctx) {
            return;
        }
        // After the window's size changes draw once more: the graphics
        // driver can show a frame of the old size (seen on Windows as a
        // transparent strip and clicks below the buttons).
        self.check_backend();
        let screen = ctx.screen_rect().size();
        if screen != self.last_screen {
            self.last_screen = screen;
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        self.retire.pump();
        self.poll(ctx);
        self.begin_open();
        if self.closing || self.close_allowed || self.restoring {
            // Freeze edits after the final snapshot and while a restore replaces state.
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                });
            });
            if ctx.input(|i| i.viewport().close_requested()) && !self.close_allowed {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
            return;
        }
        self.poll_folder(ctx);
        self.sync_watcher(ctx);
        self.sync_display_file();
        self.queue_identity(ctx);
        self.poll_view(ctx);
        self.hover.begin(&self.lib.settings);
        self.keyboard(ctx);
        let dropped = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()));
        if let Some(p) = dropped {
            self.open(p);
        }
        self.music.configure(
            &self.lib.settings.music,
            self.lib.settings.music_shuffle,
            self.lib.settings.volume,
        );
        self.music.tick();
        if let Some(e) = self.music.error.take() {
            self.notify(e);
        }
        self.edges(ctx);
        let keep = self.image_demand(ctx);
        self.images.poll(ctx, &self.lib.settings, &keep);
        if let Some(w) = self.images.warning.take() {
            self.notify(w);
        }
        self.central(ctx);
        self.gif_player.end_frame();
        self.gesture_frame(ctx);
        self.toolbar(ctx);
        self.order_chooser(ctx);
        self.button_popups(ctx, false);
        self.prompt_window(ctx);
        self.grid_icon_bar(ctx);
        self.panels(ctx);
        self.tag_favorite_chooser(ctx);
        self.floating_delete(ctx);
        self.floating_move(ctx);
        self.move_grid_frame(ctx);
        self.record_place();
        self.info_window(ctx);
        self.recent_window(ctx);
        self.help_window(ctx);
        self.hover.area = HoverArea::Settings;
        self.settings_window(ctx);
        if let Some(name) = self.pending_profile.take() {
            self.apply_profile(&name);
        }
        self.hover.area = HoverArea::General;
        self.dialogs(ctx);
        self.button_popups(ctx, true);
        self.window_diagnostics(ctx);
        // Toolbar/page/settings actions can change demand during UI construction.
        // Publish the whole batch atomically, with foreground before prefetch.
        self.image_demand(ctx);
        if self.busy
            || self.backup_busy
            || self.loading
            || self.view_pending
            || self.index_progress.is_some()
        {
            egui::Area::new(Id::new("busy"))
                .anchor(egui::Align2::LEFT_BOTTOM, Vec2::new(20., -20.))
                .order(egui::Order::Tooltip)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(if self.backup_busy {
                            "Saving backup…"
                        } else if self.loading {
                            "Reading folder…"
                        } else if self.view_pending {
                            "Updating view…"
                        } else if self.busy {
                            "Working…"
                        } else {
                            "Indexing keywords…"
                        });
                        if let Some(progress) = self.index_progress {
                            ui.label(format!(
                                "{} / {} · {} cached",
                                progress.done, progress.total, progress.cached
                            ));
                        }
                        if (self.loading || self.index_progress.is_some())
                            && self
                                .hover
                                .button(ui, "Stop indexing", false, !self.busy, false)
                        {
                            self.folders.cancel();
                            if self.loading {
                                self.view_pending = false;
                            }
                            self.loading = false;
                            self.index_progress = None;
                        }
                    });
                });
        }
        if self
            .toast
            .as_ref()
            .map(|(_, t)| t.elapsed() >= crate::repaint::TOAST_LIFETIME)
            .unwrap_or(false)
        {
            self.toast = None;
        }
        self.draw_big_toast(ctx);
        self.draw_round_message(ctx);
        if let Some((message, _)) = self.toast.as_ref().filter(|_| !self.image_only()) {
            egui::Area::new(Id::new("toast"))
                .anchor(egui::Align2::CENTER_TOP, Vec2::new(0., 65.))
                .order(egui::Order::Tooltip)
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .corner_radius(12.)
                        .inner_margin(12.)
                        .show(ui, |ui| {
                            ui.set_max_width(700.);
                            ui.label(message);
                        });
                });
        }
        self.hover.finish(ctx);
        if self.image_seen != self.current {
            self.image_seen = self.current.clone();
            self.image_changed = Instant::now();
        }
        if self.dirty
            && !self.restoring
            && !self.closing
            && crate::repaint::save_due(self.last_save, self.image_changed, Instant::now())
                .is_zero()
            && self.persist_allowed
        {
            if let Some(writer) = &mut self.writer {
                match writer.save(&mut self.lib) {
                    Ok(()) => self.dirty = false,
                    // Retried with the next save; nothing for the user to do.
                    Err(e) if e.is::<crate::state::WriterBusy>() => {
                        self.log_line(&format!("Save postponed: {e}"))
                    }
                    Err(e) => self.notify(format!("Cannot save settings: {e:#}")),
                }
            }
            self.last_save = Instant::now();
        }
        if !self.backup_busy
            && !self.busy
            && !self.view_pending
            && Instant::now() >= self.next_backup_attempt
        {
            if let Some(slot) = storage::due_slot(&self.lib, chrono::Local::now()) {
                if self.dialog.is_none() {
                    self.backup(Some(slot));
                }
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.close_allowed {
            if self.busy || self.backup_busy || self.closing {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.notify("Wait for the current operation to finish before closing");
            } else if self.lib.settings.backup_exit && self.persist_allowed && self.writer.is_some()
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.closing = true;
                self.backup(None);
                if !self.backup_busy {
                    self.closing = false;
                }
            } else {
                self.flush_before_close(ctx);
                if !self.close_allowed {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                }
            }
        }
        let now = Instant::now();
        let suspended = self.paused
            || self.grid
            || self.settings
            || self.dialog.is_some()
            || self.keyword_editor.is_some()
            || self.keyword_loading.is_some()
            || self.new_tag_open
            || self.busy
            || self.view_pending
            || self.help
            || self.info
            || ctx.memory(|m| m.any_popup_open())
            || (self.lib.settings.magnifier && ctx.input(|i| i.pointer.primary_down()));
        if self.slide_suspended && !suspended {
            self.last_slide = now;
        }
        self.slide_suspended = suspended;
        if self.slideshow && !suspended {
            ctx.request_repaint_after(crate::repaint::remaining(
                self.last_slide,
                crate::repaint::slide_interval(self.lib.settings.slideshow_secs),
                now,
            ));
        }
        if self.dirty && self.persist_allowed && self.writer.is_some() {
            ctx.request_repaint_after(crate::repaint::save_due(
                self.last_save,
                self.image_changed,
                now,
            ));
        }
        if let Some((_, start)) = &self.toast {
            ctx.request_repaint_after(crate::repaint::remaining(
                *start,
                crate::repaint::TOAST_LIFETIME,
                now,
            ));
        }
        if self.music.playing {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.lib.settings.backup_schedule && self.lib.settings.backup_days.iter().any(|v| *v) {
            let mut delay = crate::repaint::next_minute(chrono::Local::now());
            if self.next_backup_attempt > now {
                delay = delay.min(self.next_backup_attempt.duration_since(now));
            }
            ctx.request_repaint_after(delay);
        }
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(writer) = self.writer.take() {
            writer.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::bounded;
    fn viewer(ctx: &egui::Context) -> Viewer {
        let startup = Startup::with_reader(
            || Loaded {
                store: None,
                library: Ok(Library::default()),
            },
            || {},
        );
        let mut viewer = Viewer::bootstrap(ctx.clone(), "test GPU".into(), None, startup);
        viewer.startup = None;
        viewer
    }
    #[test]
    fn view_timer_reports_a_first_view_only_when_one_was_shown() {
        let mut timer = ViewTimer {
            path: "a.png".into(),
            opened: Instant::now(),
            first: None,
            better: None,
            sharp: None,
        };
        assert_eq!(timer.summary(), None);
        timer.sharp = Some(Duration::from_millis(180));
        assert_eq!(
            timer.summary().unwrap(),
            "full detail after 180 ms, no first view needed"
        );
        timer.first = Some((Duration::from_millis(35), 2560));
        assert_eq!(
            timer.summary().unwrap(),
            "First view 2560 px after 35 ms · full detail after 180 ms"
        );
        assert!(timer.had_first_view());
        // The grid thumbnail first, then the stored first view, then detail.
        timer.first = None;
        timer.reduced(512);
        assert!(!timer.had_first_view());
        timer.reduced(2560);
        timer.reduced(512);
        assert!(timer.had_first_view());
        assert_eq!(timer.better.unwrap().1, 2560);
        let summary = timer.summary().unwrap();
        assert!(summary.starts_with("Preview 512 px after "));
        assert!(summary.contains(" · First view 2560 px after "));
        assert!(summary.ends_with(" · full detail after 180 ms"));
    }
    #[test]
    fn real_watcher_state_worker_and_view_preserve_marks_after_external_rename() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("images");
        std::fs::create_dir(&root).unwrap();
        let old = root.join("old.jpg");
        let new = root.join("renamed.jpg");
        let copy = root.join("copy.jpg");
        std::fs::write(&old, include_bytes!("../tests/fixtures/iptc-keywords.jpg")).unwrap();
        let store = Store {
            path: dir.path().join("data/library.json"),
            exe_dir: dir.path().into(),
        };
        let mut lib = Library::default();
        lib.photos.entry(old.clone()).or_default().favorite = true;
        lib.photos.entry(old.clone()).or_default().elite = true;
        store.save(&lib).unwrap();
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.lib = store.load().unwrap();
        v.store = Some(store.clone());
        v.persist_allowed = true;
        let tx = v.tx.clone();
        v.writer = Some(StateWriter::new(store, move |n| {
            let _ = tx.send(Event::State(n));
        }));
        v.open(root);
        let pump = |v: &mut Viewer, check: &dyn Fn(&Viewer) -> bool| {
            let end = Instant::now() + Duration::from_secs(15);
            loop {
                v.poll(&ctx);
                v.retire.pump();
                v.begin_open();
                v.poll_folder(&ctx);
                v.queue_identity(&ctx);
                v.poll_view(&ctx);
                if check(v) {
                    break;
                }
                assert!(Instant::now() < end, "identity/view pipeline stalled: pending={} inflight={} serial={:?}/{} photos={:?}",v.identity_pending,v.identity_inflight,v.identity_snapshot.as_ref().map(|s|s.serial),v.folders.identity_serial(),v.lib.photos);
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        pump(&mut v, &|v| {
            v.last_index.is_some() && !v.identity_inflight && !v.view_pending
        });
        v.filter.favorite = true;
        v.rebuild();
        pump(&mut v, &|v| !v.view_pending);
        assert_eq!(v.visible.len(), 1);
        std::fs::rename(&old, &new).unwrap();
        std::fs::copy(&new, &copy).unwrap();
        pump(&mut v, &|v| {
            v.lib.photos.get(&new).is_some_and(|m| m.elite)
                && !v.view_pending
                && v.visible.iter().any(|p| p == &new)
        });
        assert!(!v.lib.photos.contains_key(&old));
        assert!(!v.lib.photos.get(&copy).is_some_and(|m| m.favorite));
        assert_eq!(v.visible.len(), 1);
        v.writer.take().unwrap().shutdown();
    }
    #[test]
    fn chosen_cache_folder_gets_its_own_subfolder_and_falls_back_to_the_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        assert_eq!(
            thumbnail_cache_root(Path::new(""), Some(&data)),
            Some(data.join("thumbnails"))
        );
        let chosen = dir.path().join("elsewhere");
        let root = thumbnail_cache_root(&chosen, Some(&data)).unwrap();
        assert_eq!(root, chosen.join(CACHE_FOLDER));
        assert!(root.is_dir());
        // A location that cannot be created (a file is in the way) keeps the default.
        let blocked = dir.path().join("file");
        std::fs::write(&blocked, b"x").unwrap();
        assert_eq!(
            thumbnail_cache_root(&blocked, Some(&data)),
            Some(data.join("thumbnails"))
        );
        assert_eq!(thumbnail_cache_root(&blocked, None), None);
    }
    fn grid_frame(ctx: &egui::Context, v: &mut Viewer) {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| v.draw_grid(ui));
        });
    }
    fn pointer(ctx: &egui::Context, v: &mut Viewer, pos: Pos2, pressed: Option<bool>) {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 600.));
        let mut input = egui::RawInput {
            screen_rect: Some(area),
            ..Default::default()
        };
        input.events.push(egui::Event::PointerMoved(pos));
        if let Some(pressed) = pressed {
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
        }
        let _ = ctx.run(input, |ctx| v.grid_viewer_input(ctx, area));
    }
    #[test]
    fn grid_big_image_short_click_or_gesture_returns_and_hold_or_browse_does_not() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let (a, b) = (PathBuf::from("a.jpg"), PathBuf::from("b.jpg"));
        v.visible = vec![a.clone(), b.clone()].into();
        v.grid = true;
        v.grid_normal = true;
        v.open_grid_viewer(b.clone());
        assert_eq!(v.current.as_ref(), Some(&b));
        // No browsing while the big image is open.
        v.navigate(false);
        assert_eq!(v.current.as_ref(), Some(&b));
        // A long hold is the magnifier's: the view stays open.
        let p = Pos2::new(400., 300.);
        pointer(&ctx, &mut v, p, Some(true));
        std::thread::sleep(Duration::from_millis(v.lib.settings.grid_hold_ms + 50));
        pointer(&ctx, &mut v, p, Some(false));
        assert!(v.grid_viewer.is_some());
        // A short click returns to the grid.
        pointer(&ctx, &mut v, p, Some(true));
        pointer(&ctx, &mut v, p, Some(false));
        assert!(v.grid_viewer.is_none());
        // A left drag is not a click: the view stays open.
        v.open_grid_viewer(b.clone());
        pointer(&ctx, &mut v, Pos2::new(100., 300.), Some(true));
        pointer(&ctx, &mut v, Pos2::new(300., 310.), Some(false));
        assert!(v.grid_viewer.is_some());
    }
    fn right(ctx: &egui::Context, v: &mut Viewer, pos: Pos2, pressed: Option<bool>) {
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 600.))),
            ..Default::default()
        };
        input.events.push(egui::Event::PointerMoved(pos));
        if let Some(pressed) = pressed {
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            });
        }
        let _ = ctx.run(input, |ctx| v.gesture_frame(ctx));
    }
    fn right_drag(ctx: &egui::Context, v: &mut Viewer, from: Pos2, to: Pos2) {
        right(ctx, v, from, Some(true));
        right(ctx, v, from.lerp(to, 0.5), None);
        right(ctx, v, to, None);
        right(ctx, v, to, Some(false));
        right(ctx, v, to, None);
    }
    #[test]
    fn right_button_gesture_closes_views_or_opens_a_tile_and_a_click_does_not() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let (a, b) = (PathBuf::from("a.jpg"), PathBuf::from("b.jpg"));
        v.visible = vec![a.clone(), b.clone()].into();
        v.grid = true;
        v.lib.settings.gestures = true;
        v.open_grid_viewer(b.clone());
        // Short right click: the context menu's, no gesture.
        right(&ctx, &mut v, Pos2::new(100., 100.), Some(true));
        right(&ctx, &mut v, Pos2::new(101., 100.), Some(false));
        right(&ctx, &mut v, Pos2::new(101., 100.), None);
        assert!(v.grid_viewer.is_some());
        // Wrong direction does nothing; the configured one closes.
        right_drag(&ctx, &mut v, Pos2::new(400., 300.), Pos2::new(200., 300.));
        assert!(v.grid_viewer.is_some());
        right_drag(&ctx, &mut v, Pos2::new(200., 300.), Pos2::new(400., 310.));
        assert!(v.grid_viewer.is_none());
        // The hover preview closes first.
        v.preview = Some(a.clone());
        right_drag(&ctx, &mut v, Pos2::new(200., 300.), Pos2::new(400., 310.));
        assert!(v.preview.is_none());
        // Over a tile, with the option on, it opens that image big.
        v.grid_tiles = vec![(
            Rect::from_min_size(Pos2::new(150., 250.), Vec2::new(100., 100.)),
            a.clone(),
        )];
        right_drag(&ctx, &mut v, Pos2::new(200., 300.), Pos2::new(400., 310.));
        assert!(v.grid_viewer.is_none());
        v.lib.settings.gesture_opens_tile = true;
        right_drag(&ctx, &mut v, Pos2::new(200., 300.), Pos2::new(400., 310.));
        assert_eq!(v.grid_viewer.as_ref(), Some(&a));
        // Gestures off: nothing happens.
        v.close_grid_viewer();
        v.lib.settings.gestures = false;
        right_drag(&ctx, &mut v, Pos2::new(200., 300.), Pos2::new(400., 310.));
        assert!(v.grid_viewer.is_none());
    }
    #[test]
    fn normal_mode_keeps_or_clears_the_selection_and_it_can_be_remembered() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let a = PathBuf::from("a.jpg");
        v.visible = vec![a.clone()].into();
        v.grid = true;
        v.delete_selection.insert(a.clone());
        v.toggle_grid_normal();
        assert!(
            v.grid_normal && v.delete_selection.contains(&a),
            "kept by default"
        );
        v.toggle_grid_normal();
        v.lib.settings.normal_keeps_selection = false;
        v.toggle_grid_normal();
        assert!(v.delete_selection.is_empty());
        assert_eq!(
            v.last_selection.as_ref().map(|s| s.0.clone()),
            Some(HashSet::from([a.clone()]))
        );
    }
    #[test]
    fn big_message_box_uses_the_set_size_within_the_window() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let rect = |ctx: &egui::Context, v: &mut Viewer| {
            v.big_toast = Some(("Selection cleared".into(), Instant::now()));
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200., 800.))),
                ..Default::default()
            };
            for _ in 0..2 {
                let _ = ctx.run(input.clone(), |ctx| v.draw_big_toast(ctx));
            }
            ctx.memory(|m| m.area_rect(Id::new("big_toast"))).unwrap()
        };
        v.lib.settings.selection_toast_width = 700.;
        v.lib.settings.selection_toast_height = 200.;
        let r = rect(&ctx, &mut v);
        // Within the frame's stroke width.
        assert!(
            (r.width() - 700.).abs() < 4. && (r.height() - 200.).abs() < 4.,
            "{r:?}"
        );
        assert!((r.center().x - 600.).abs() < 2. && (r.center().y - 400.).abs() < 2.);
        v.lib.settings.selection_toast_width = 1600.;
        let r = rect(&ctx, &mut v);
        assert!(r.width() <= 1160. + 4., "{r:?}");
    }
    #[test]
    fn normal_mode_keeps_the_tag_selection_and_resumes_tag_mode() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let (a, b) = (PathBuf::from("a.jpg"), PathBuf::from("b.jpg"));
        v.visible = vec![a.clone(), b.clone()].into();
        v.grid = true;
        v.tag_mode = true;
        v.panel = Panel::Tags;
        v.tag_selection.insert(a.clone());
        v.delete_selection.insert(b.clone());
        v.toggle_grid_normal();
        assert!(v.grid_normal && !v.tag_mode && v.panel != Panel::Tags);
        // A frame with the palette closed must not drop the kept selection.
        grid_frame(&ctx, &mut v);
        assert!(v.tag_selection.contains(&a) && v.delete_selection.contains(&b));
        v.toggle_grid_normal();
        assert!(!v.grid_normal && v.tag_mode && v.panel == Panel::Tags);
        grid_frame(&ctx, &mut v);
        assert!(v.tag_selection.contains(&a));
        // Setting off: both cleared, and Restore selection has them.
        v.lib.settings.normal_keeps_selection = false;
        v.toggle_grid_normal();
        grid_frame(&ctx, &mut v);
        assert!(v.tag_selection.is_empty() && v.delete_selection.is_empty());
        let (trash, tags) = v.last_selection.clone().unwrap();
        assert_eq!(trash, HashSet::from([b.clone()]));
        assert_eq!(tags, HashSet::from([a.clone()]));
        v.toggle_grid_normal();
        assert!(!v.tag_mode);
    }
    #[test]
    fn preview_closes_from_enabled_strips_or_a_left_click() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let a = PathBuf::from("a.jpg");
        v.visible = vec![a.clone()].into();
        v.grid = true;
        v.lib.settings.preview_close_ms = 0;
        let frame = |ctx: &egui::Context, v: &mut Viewer, pos: Pos2, click: bool| {
            let mut input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 600.))),
                ..Default::default()
            };
            input.events.push(egui::Event::PointerMoved(pos));
            if click {
                for pressed in [true, false] {
                    input.events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    });
                }
            }
            let area = Rect::from_min_size(Pos2::new(50., 50.), Vec2::new(700., 500.));
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| v.grid_overlays(ui, area, None));
            });
        };
        // Left strip off by default: resting there keeps the preview.
        v.preview = Some(a.clone());
        frame(&ctx, &mut v, Pos2::new(5., 300.), false);
        frame(&ctx, &mut v, Pos2::new(5., 300.), false);
        assert!(v.preview.is_some());
        v.lib.settings.preview_close_left = true;
        frame(&ctx, &mut v, Pos2::new(5., 300.), false);
        frame(&ctx, &mut v, Pos2::new(5., 300.), false);
        assert!(v.preview.is_none());
        // Right strip and the top strip together; a click only when enabled.
        v.preview = Some(a.clone());
        frame(&ctx, &mut v, Pos2::new(400., 300.), true);
        assert!(v.preview.is_some());
        v.lib.settings.preview_close_click = true;
        frame(&ctx, &mut v, Pos2::new(400., 300.), true);
        assert!(v.preview.is_none());
        v.preview = Some(a.clone());
        v.lib.settings.preview_close_right = true;
        frame(&ctx, &mut v, Pos2::new(795., 300.), false);
        frame(&ctx, &mut v, Pos2::new(795., 300.), false);
        assert!(v.preview.is_none());
    }
    #[test]
    fn select_all_buttons_replace_the_selection() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let all: Vec<PathBuf> = (0..5).map(|n| PathBuf::from(format!("{n}"))).collect();
        v.active_selection().clear();
        v.select_paths(all.clone());
        assert_eq!(v.delete_selection.len(), 5);
        // "view" after "folder": only the page remains selected.
        v.active_selection().clear();
        v.select_paths(all[..2].to_vec());
        assert_eq!(v.delete_selection, all[..2].iter().cloned().collect());
    }
    #[test]
    fn deleted_tiles_stay_until_the_page_changes_and_next_page_skips_nothing() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let paths: Vec<PathBuf> = (0..6).map(|n| PathBuf::from(format!("{n}.jpg"))).collect();
        v.visible = paths.clone().into();
        v.files = paths.clone();
        v.grid = true;
        v.lib.settings.grid_round_full = false;
        v.lib.settings.thumbnail_count = 3;
        v.remove_from_view(std::slice::from_ref(&paths[1]));
        let g = v.grid_ghosts.as_ref().unwrap();
        assert_eq!(g.page, paths[..3].to_vec());
        assert!(g.deleted.contains(&paths[1]));
        // After the deletion the list is 0,2,3,4,5; page one showed 0,(1),2.
        v.visible = vec![
            paths[0].clone(),
            paths[2].clone(),
            paths[3].clone(),
            paths[4].clone(),
            paths[5].clone(),
        ]
        .into();
        v.grid_page(true);
        assert!(v.grid_ghosts.is_none());
        assert_eq!(v.visible[v.grid_start], paths[3]);
    }
    #[test]
    fn select_all_respects_the_active_mode_and_deletion_marks() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let (a, b, c) = (PathBuf::from("a"), PathBuf::from("b"), PathBuf::from("c"));
        v.delete_selection.insert(a.clone());
        v.tag_mode = true;
        v.select_paths(vec![a.clone(), b.clone(), c.clone()]);
        assert_eq!(v.tag_selection, HashSet::from([b.clone(), c.clone()]));
        v.tag_mode = false;
        v.select_paths(vec![b.clone()]);
        assert_eq!(v.delete_selection, HashSet::from([a, b]));
        v.active_selection().clear();
        assert!(v.delete_selection.is_empty());
    }
    #[test]
    fn grid_tag_mode_ends_with_its_palette_unless_kept_by_setting() {
        let ctx = egui::Context::default();
        for keep in [false, true] {
            let mut v = viewer(&ctx);
            let (a, b) = (PathBuf::from("a.jpg"), PathBuf::from("b.jpg"));
            v.visible = vec![a.clone(), b.clone()].into();
            v.grid = true;
            v.lib.settings.grid_keep_tag_mode = keep;
            v.tag_mode = true;
            v.panel = Panel::Tags;
            v.tag_selection.insert(a.clone());
            grid_frame(&ctx, &mut v);
            assert!(v.tag_mode && v.tag_selection.contains(&a));
            // Saving keywords: the palette closes unless the setting keeps it.
            let tags = BTreeSet::from(["x".to_string()]);
            v.tx.send(Event::Tagged(vec![(a.clone(), Ok(tags))]))
                .unwrap();
            v.poll(&ctx);
            grid_frame(&ctx, &mut v);
            assert_eq!(v.tag_mode, keep);
            assert_eq!(v.panel == Panel::Tags, keep);
            assert!(v.tag_selection.is_empty());
        }
        // Escape (or any other close) ends tag mode and clears the selection.
        let mut v = viewer(&ctx);
        v.visible = vec![PathBuf::from("c.jpg")].into();
        v.grid = true;
        v.tag_mode = true;
        v.panel = Panel::Tags;
        v.tag_selection.insert(PathBuf::from("c.jpg"));
        v.panel = Panel::None;
        grid_frame(&ctx, &mut v);
        assert!(!v.tag_mode && v.tag_selection.is_empty());
    }
    #[test]
    fn a_deletion_is_remembered_and_restoring_brings_marks_and_the_image_back() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let folder = PathBuf::from("/photos");
        v.folder = Some(folder.clone());
        v.files = (0..4).map(|n| folder.join(format!("{n}.jpg"))).collect();
        v.folder_paths = v.files.iter().cloned().collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        let (a, b) = (v.files[1].clone(), v.files[2].clone());
        v.lib.photos.insert(
            a.clone(),
            crate::model::PhotoMeta {
                favorite: true,
                ..Default::default()
            },
        );
        assert_eq!(v.restorable(), 0);
        v.deleting_since = 1_000;
        v.tx.send(Event::Deleted(vec![a.clone(), b.clone()], vec![]))
            .unwrap();
        v.poll(&ctx);
        let deletion = v.lib.last_deletion.clone().expect("remembered");
        assert_eq!(deletion.at, 1_000);
        assert_eq!(v.restorable(), 2);
        assert!(v.lib.photos.get(&a).is_none(), "removed with the file");
        // The view drops them (the folder watcher would also send a new list).
        v.files.retain(|p| p != &a && p != &b);
        v.folder_paths.retain(|p| p != &a && p != &b);
        v.rebuild();
        finish_view(&mut v, &ctx);
        // Only `a` comes back: its marks return, it is listed and shown;
        // `b` stays to be tried again.
        v.restored(vec![a.clone()], vec!["b failed".into()]);
        finish_view(&mut v, &ctx);
        assert!(v.lib.photos[&a].favorite);
        assert!(v.visible.position(&a).is_some());
        assert_eq!(v.current.as_ref(), Some(&a));
        assert_eq!(v.restorable(), 1);
        assert!(matches!(v.dialog, Some(Dialog::Error(_))));
        // An empty move (nothing deleted) keeps what can be restored.
        v.dialog = None;
        v.tx.send(Event::Deleted(vec![], vec!["x".into()])).unwrap();
        v.poll(&ctx);
        assert_eq!(v.restorable(), 1);
        v.restored(vec![b.clone()], vec![]);
        assert!(v.lib.last_deletion.is_none());
    }
    #[test]
    fn grid_controls_follow_the_arrangement_with_the_group_behind_more() {
        use crate::bar_layout::{placed, Place};
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let (items, bar) = v.grid_items(true);
        assert_eq!(
            &items[bar..],
            [GridItem::GoTo, GridItem::Profile],
            "Go to and Profiles in the group by default"
        );
        assert_eq!(items[bar - 1], GridItem::Status);
        let d = grid_defaults();
        let slots = placed(&d, None, "trash", Place::Hidden);
        let slots = placed(&d, Some(&slots), "sort", Place::Group);
        let slots = crate::bar_layout::moved(&d, Some(&slots), "tags", true);
        v.lib.settings.bar_buttons.insert("grid".into(), slots);
        let (items, bar) = v.grid_items(true);
        assert!(!items.contains(&GridItem::Trash), "hidden");
        assert_eq!(items[bar - 1], GridItem::Status, "the status ends the bar");
        assert_eq!(
            &items[bar..],
            [GridItem::Sort, GridItem::GoTo, GridItem::Profile],
            "the group after it"
        );
        let tags = items.iter().position(|i| *i == GridItem::Tags).unwrap();
        let undo = items.iter().position(|i| *i == GridItem::Undo).unwrap();
        assert!(tags < undo, "moved up past Restore deleted");
        // A grouped item opens behind » even when everything else fits.
        v.files = (0..30).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        v.grid = true;
        v.grid_overflow = true;
        for _ in 0..4 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(2400., 820.))),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                v.central(ctx);
                v.grid_icon_bar(ctx);
            });
        }
        assert!(v.grid_overflow, "» is there for the group");
        assert!(ctx
            .memory(|m| m.area_rect(Id::new(("grid_overflow", true))))
            .is_some());
    }
    #[test]
    fn the_toolbar_group_bar_opens_above_the_bar_and_a_press_elsewhere_closes_it() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..3).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        v.bar = true;
        // Image only, the round notice, Go to and Profiles are in the group
        // by default.
        let (_, grouped) = split_places(&v.bar_items("toolbar", &tool_defaults()));
        assert_eq!(grouped, ["image_only", "round", "goto", "profile"]);
        v.group_open = Some("toolbar");
        let frame = |v: &mut Viewer, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| v.toolbar(ctx));
        };
        frame(&mut v, vec![]);
        frame(&mut v, vec![]);
        let bar = ctx.memory(|m| m.area_rect(Id::new("toolbar"))).unwrap();
        let group = ctx.memory(|m| m.area_rect(Id::new("group_bar"))).unwrap();
        assert!(group.bottom() <= bar.top(), "{group:?} above {bar:?}");
        let at = Pos2::new(100., 100.);
        frame(&mut v, vec![egui::Event::PointerMoved(at)]);
        frame(
            &mut v,
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(v.group_open, None);
        // Nothing grouped: no » and no group bar.
        let d = tool_defaults();
        let mut slots = None;
        for key in ["image_only", "round", "goto", "profile"] {
            slots = Some(crate::bar_layout::placed(
                &d,
                slots.as_deref(),
                key,
                Place::Hidden,
            ));
        }
        v.lib
            .settings
            .bar_buttons
            .insert("toolbar".into(), slots.unwrap());
        let (_, grouped) = split_places(&v.bar_items("toolbar", &d));
        assert!(grouped.is_empty());
    }
    #[test]
    fn restoring_deleted_images_asks_first_unless_turned_off() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.request_restore();
        assert!(v.dialog.is_none() && !v.busy, "nothing to restore");
        v.lib.last_deletion = Some(crate::model::Deletion {
            at: 1,
            items: vec![("/x/a.jpg".into(), None)],
        });
        v.request_restore();
        assert!(matches!(&v.dialog, Some(Dialog::RestoreDeleted(p)) if p.len() == 1));
        assert!(!v.busy, "only after Restore");
        v.dialog = None;
        v.lib.settings.confirm_restore = false;
        v.request_restore();
        assert!(v.dialog.is_none() && v.busy, "straight to the Recycle Bin");
    }
    fn browsing_viewer(ctx: &egui::Context, n: usize) -> Viewer {
        let mut v = viewer(ctx);
        v.files = (0..n).map(|i| format!("/p/{i:02}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, ctx);
        v
    }
    #[test]
    fn a_round_notice_comes_once_every_image_has_been_shown() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 3);
        v.lib.settings.order = crate::model::Order::Sequential;
        let files = v.files.clone();
        for p in &files[..2] {
            v.count_round(p);
        }
        v.count_round(&files[1]);
        assert!(
            v.round_message.is_none(),
            "two of three, the same image twice"
        );
        v.count_round(&files[2]);
        assert!(v
            .round_message
            .as_ref()
            .is_some_and(|(t, _, _)| t.starts_with("All 3 images")));
        // A new round has begun with the image shown.
        assert_eq!(v.round_seen.len(), 1);
        // As a dialog; Random never counts; off never tells.
        v.round_message = None;
        v.lib.settings.round_notice_dialog = true;
        v.count_round(&files[0]);
        v.count_round(&files[1]);
        assert!(matches!(v.dialog, Some(Dialog::RoundDone(3))));
        v.dialog = None;
        v.lib.settings.order = crate::model::Order::Random;
        for p in &files {
            v.count_round(p);
        }
        assert!(v.dialog.is_none() && v.round_seen.is_empty());
        v.lib.settings.order = crate::model::Order::Shuffle;
        v.lib.settings.round_notice = false;
        for p in files.iter().chain(&files) {
            v.count_round(p);
        }
        assert!(v.dialog.is_none());
    }
    #[test]
    fn images_browsed_past_count_for_the_round_without_being_drawn() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 6);
        v.lib.settings.order = crate::model::Order::Sequential;
        v.lib.settings.round_notice_images = 3;
        let first = v.files[0].clone();
        // The first image is drawn; the wheel then passes the other five
        // within frames that draw only some of them.
        v.count_round(&first);
        for _ in 0..4 {
            v.navigate(true);
        }
        assert!(v.round_message.is_none(), "five of six");
        assert_eq!(v.images_shown, 5);
        v.navigate(true);
        assert!(v
            .round_message
            .as_ref()
            .is_some_and(|(t, _, _)| t.starts_with("All 6 images")));
        // Drawing the image browsed to does not count it twice.
        let last = v.current.clone().unwrap();
        v.count_round(&last);
        assert_eq!(v.images_shown, 6);
        // Browsing on counts towards the notice going away.
        for _ in 0..3 {
            v.navigate(true);
        }
        let _ = ctx.run(egui::RawInput::default(), |ctx| v.draw_round_message(ctx));
        assert!(v.round_message.is_none(), "gone after three new images");
    }
    #[test]
    fn the_round_notice_can_fade_out_before_it_goes() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 3);
        v.lib.settings.order = crate::model::Order::Sequential;
        v.lib.settings.round_notice_images = 1;
        v.lib.settings.round_notice_fade = true;
        v.lib.settings.round_notice_fade_ms = 400;
        let files = v.files.clone();
        for p in &files {
            v.count_round(p);
        }
        let frame = |v: &mut Viewer| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| v.draw_round_message(ctx));
        };
        frame(&mut v);
        assert!(v.round_message.is_some() && v.round_fade_out.is_none());
        // One new image: the notice fades out instead of vanishing.
        v.count_round(&files[0]);
        frame(&mut v);
        assert!(v.round_message.is_some() && v.round_fade_out.is_some());
        v.round_fade_out = Instant::now().checked_sub(Duration::from_millis(500));
        frame(&mut v);
        assert!(v.round_message.is_none() && v.round_fade_out.is_none());
        // Without the fade it goes at once.
        v.lib.settings.round_notice_fade = false;
        v.count_round(&files[1]);
        assert!(v.round_message.is_some(), "the third of the next round");
        v.count_round(&files[0]);
        frame(&mut v);
        assert!(v.round_message.is_none());
    }
    #[test]
    fn after_go_to_browsing_back_shows_the_previous_image_in_order() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 21);
        v.lib.settings.order = crate::model::Order::Sequential;
        v.navigate(true);
        assert_eq!(v.current.as_deref(), Some(Path::new("/p/01.jpg")));
        v.go_to(7);
        v.navigate(false);
        assert_eq!(
            v.current.as_deref(),
            Some(Path::new("/p/05.jpg")),
            "6, not back to 1"
        );
        v.navigate(false);
        assert_eq!(v.current.as_deref(), Some(Path::new("/p/04.jpg")));
    }
    #[test]
    fn the_round_notice_goes_after_new_images_and_turning_it_off_starts_over() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 5);
        v.lib.settings.order = crate::model::Order::Sequential;
        v.lib.settings.round_notice_images = 2;
        let files = v.files.clone();
        for p in &files {
            v.count_round(p);
        }
        assert!(v.round_message.is_some());
        let frame = |v: &mut Viewer| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| v.draw_round_message(ctx));
        };
        v.count_round(&files[0]);
        frame(&mut v);
        assert!(v.round_message.is_some(), "one new image");
        v.count_round(&files[1]);
        frame(&mut v);
        assert!(v.round_message.is_none(), "two new images");
        // Turned off: what was seen is forgotten.
        assert!(!v.round_seen.is_empty());
        v.lib.settings.round_notice = false;
        v.count_round(&files[2]);
        assert!(v.round_seen.is_empty());
    }
    #[test]
    fn go_to_shows_an_image_or_a_page_and_grid_pages_can_wrap() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 25);
        v.go_to(7);
        assert_eq!(v.current.as_deref(), Some(Path::new("/p/06.jpg")));
        v.go_to(999);
        assert_eq!(
            v.current.as_deref(),
            Some(Path::new("/p/24.jpg")),
            "kept within"
        );
        v.grid = true;
        v.lib.settings.grid_round_full = false;
        v.lib.settings.thumbnail_count = 10;
        let count = v.grid_count();
        v.go_to(2);
        assert_eq!(v.grid_start, count);
        v.go_to(99);
        let last = 24 / count * count;
        assert_eq!(v.grid_start, last);
        // Without wrapping the last page stays; with it, on to the first.
        v.grid_page(true);
        assert_eq!(v.grid_start, last);
        v.lib.settings.grid_page_wrap = true;
        v.grid_page(true);
        assert_eq!(v.grid_start, 0);
        v.grid_page(false);
        assert_eq!(v.grid_start, last, "back from the first to the last");
    }
    #[test]
    fn paging_back_after_a_jump_ignores_the_page_frozen_before_it() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 250);
        v.grid = true;
        v.lib.settings.grid_round_full = false;
        v.lib.settings.thumbnail_count = 12;
        let count = v.grid_count();
        // A deletion froze page 1; the view then moved to page 7 another way.
        v.freeze_grid_page(&[v.files[0].clone()]);
        assert!(v.grid_ghosts.is_some());
        v.grid_start = 6 * count;
        v.grid_page(false);
        assert_eq!(
            v.grid_start,
            5 * count,
            "page 6, not from the frozen page 1"
        );
    }
    #[test]
    fn moved_images_leave_the_grid_page_without_deleted_tiles() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 30);
        v.grid = true;
        v.lib.settings.grid_round_full = false;
        v.lib.settings.thumbnail_count = 12;
        let moved: Vec<_> = v.files[3..6]
            .iter()
            .map(|p| (p.clone(), p.with_extension("moved")))
            .collect();
        v.tx.send(Event::Moved(crate::operations::MoveReport {
            moved,
            ..Default::default()
        }))
        .unwrap();
        v.poll(&ctx);
        assert!(v.grid_ghosts.is_none());
    }
    #[test]
    fn the_image_title_can_show_the_folder_with_forward_slashes() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 3);
        assert_eq!(v.top_title(false).as_deref(), Some("00.jpg"));
        v.lib.settings.image_title_folder = true;
        v.lib.settings.path_forward_slashes = true;
        // "/p" + the platform's separator, then turned to "/".
        assert_eq!(v.top_title(false).as_deref(), Some("/p/00.jpg"));
    }
    #[test]
    fn the_big_image_from_the_grid_can_be_browsed_when_chosen() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 25);
        v.grid = true;
        v.lib.settings.grid_round_full = false;
        v.lib.settings.thumbnail_count = 10;
        v.lib.settings.order = crate::model::Order::Sequential;
        let count = v.grid_count();
        let p = v.files[count - 1].clone();
        v.current = Some(p.clone());
        v.grid_viewer = Some(p.clone());
        v.navigate(true);
        assert_eq!(v.grid_viewer.as_ref(), Some(&p), "off: stays");
        v.lib.settings.grid_viewer_browse = true;
        v.navigate(true);
        let next = v.files[count].clone();
        assert_eq!(v.grid_viewer.as_ref(), Some(&next));
        assert_eq!(v.current.as_ref(), Some(&next));
        assert_eq!(v.grid_start, count, "the grid page follows");
    }
    #[test]
    fn the_grid_page_title_names_its_images() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 25);
        assert_eq!(v.top_title(false).as_deref(), Some("00.jpg"));
        v.grid = true;
        v.lib.settings.grid_round_full = false;
        v.lib.settings.thumbnail_count = 10;
        let count = v.grid_count();
        let last = format!("{:02}.jpg", count - 1);
        assert_eq!(v.top_title(true), Some(format!("00.jpg – {last}")));
        v.lib.settings.grid_caption_range = false;
        assert_eq!(v.top_title(true).as_deref(), Some("00.jpg – ..."));
        v.lib.settings.grid_caption_folder = true;
        v.lib.settings.grid_caption_count = true;
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            v.top_title(true),
            Some(format!(
                "{}{sep}00.jpg – ... (25 images)",
                Path::new("/p").display()
            ))
        );
    }
    #[test]
    fn a_chosen_favorite_folder_is_added_once_or_replaces_the_one_changed() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        for p in ["/a", "/b", "/a"] {
            v.tx.send(Event::FavoriteFolder(p.into())).unwrap();
            v.poll(&ctx);
        }
        let paths = |v: &Viewer| {
            v.lib
                .settings
                .favorite_folders
                .iter()
                .map(|f| f.path.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(paths(&v), [PathBuf::from("/a"), PathBuf::from("/b")]);
        v.favorite_pick = Some(0);
        v.tx.send(Event::FavoriteFolder("/c".into())).unwrap();
        v.poll(&ctx);
        assert_eq!(paths(&v), [PathBuf::from("/c"), PathBuf::from("/b")]);
        assert_eq!(v.lib.settings.favorite_folders[0].label(), "c");
    }
    #[test]
    fn fullscreen_can_show_only_the_image() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..3).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        let texts = |v: &mut Viewer| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                ..Default::default()
            };
            let out = ctx.run(raw, |ctx| v.central(ctx));
            out.shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let shown = texts(&mut v);
        assert!(shown.iter().any(|t| t.starts_with("1 / 3")), "{shown:?}");
        assert!(shown.iter().any(|t| t == "Settings"), "{shown:?}");
        v.fullscreen = true;
        assert!(texts(&mut v).iter().any(|t| t.starts_with("1 / 3")));
        v.lib.settings.fullscreen_image_only = true;
        let shown = texts(&mut v);
        assert!(
            !shown
                .iter()
                .any(|t| t.starts_with("1 / 3") || t == "Settings"),
            "{shown:?}"
        );
        v.fullscreen = false;
        assert!(texts(&mut v).iter().any(|t| t == "Settings"));
    }
    #[test]
    fn successful_image_keyword_edit_does_not_promote_discovered_words_to_manual_tags() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.lib.create_manual_tag("own".into());
        let path = PathBuf::from("edited.jpg");
        let tags = BTreeSet::from(["own".into(), "from image".into()]);
        v.tx.send(Event::Tagged(vec![(path.clone(), Ok(tags.clone()))]))
            .unwrap();
        v.poll(&ctx);
        assert_eq!(v.lib.photos[&path].tags, tags);
        assert!(v.lib.is_manual_tag("own"));
        assert!(!v.lib.is_manual_tag("from image"));
        v.tag_picker.refresh(&v.lib);
        assert_eq!(v.tag_picker.len(), 1);
        v.tag_picker.show_additional = true;
        v.tag_picker.refresh(&v.lib);
        assert_eq!(v.tag_picker.len(), 2);
    }
    #[test]
    fn restore_replaces_both_palette_caches_and_clears_old_save_selections() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.lib.create_manual_tag("obsolete".into());
        v.tag_picker.show_additional = true;
        v.tag_picker.refresh(&v.lib);
        v.filter_picker.refresh(&v.lib);
        v.selected_tags.insert("obsolete".into());
        let mut replacement = Library::default();
        replacement.create_manual_tag("restored".into());
        replacement.learn_tags(&BTreeSet::from(["from image".into()]));
        v.tx.send(Event::State(Notice::Restored(Ok(Box::new(replacement)))))
            .unwrap();
        v.poll(&ctx);
        assert!(v.selected_tags.is_empty());
        assert!(!v.tag_picker.show_additional);
        v.tag_picker.refresh(&v.lib);
        assert_eq!(v.tag_picker.choices()[0].name, "restored");
        v.filter_picker.refresh(&v.lib);
        assert_eq!(v.filter_picker.len(), 2);
    }
    fn finish_view(viewer: &mut Viewer, ctx: &egui::Context) {
        let end = Instant::now() + Duration::from_secs(10);
        while viewer.view_pending {
            viewer.retire.pump();
            viewer.poll_view(ctx);
            assert!(Instant::now() < end, "view build stalled");
            std::thread::yield_now();
        }
    }
    #[test]
    fn real_viewer_waits_for_bootstrap_before_creating_writer_and_restores_settings() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store {
            path: dir.path().join("library.json"),
            exe_dir: dir.path().into(),
        };
        let mut lib = Library {
            schema: 1,
            ..Default::default()
        };
        lib.settings.prefetch_ahead = 17;
        lib.last_grid = true;
        lib.photos.entry("saved.jpg".into()).or_default().favorite = true;
        store.save(&lib).unwrap();
        let (release, wait) = bounded(1);
        let (ready, received) = bounded(1);
        let startup = Startup::with_reader(
            move || {
                wait.recv().unwrap();
                Loaded {
                    library: store.load().map_err(|e| e.to_string()),
                    store: Some(store),
                }
            },
            move || {
                ready.send(()).unwrap();
            },
        );
        let ctx = egui::Context::default();
        let mut v = Viewer::bootstrap(ctx.clone(), "test GPU".into(), None, startup);
        for _ in 0..5 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                assert!(v.startup_frame(ctx))
            });
            assert!(v.writer.is_none());
            assert!(!v.persist_allowed);
            assert!(!v.dirty);
        }
        release.send(()).unwrap();
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            assert!(!v.startup_frame(ctx))
        });
        assert!(v.writer.is_some());
        assert!(v.persist_allowed);
        assert_eq!(v.lib.settings.prefetch_ahead, 17);
        assert!(v.grid);
        assert!(v.lib.photos[Path::new("saved.jpg")].favorite);
        v.writer.take().unwrap().shutdown();
    }
    #[test]
    fn viewer_replaces_partial_filter_and_rejects_old_image_actions_until_commit() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..12_000).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.lib
            .photos
            .entry(v.files[11_999].clone())
            .or_default()
            .favorite = true;
        v.rebuild();
        v.poll_view(&ctx);
        assert!(v.view_pending);
        assert!(v.visible.is_empty());
        v.navigate(true);
        assert_eq!(v.current, Some(v.files[0].clone()));
        v.request_delete(vec![v.files[0].clone()]);
        assert!(v.dialog.is_none());
        assert!(!v.busy);
        v.filter = Filter {
            favorite: true,
            ..Default::default()
        };
        v.rebuild();
        v.poll_view(&ctx);
        assert!(v.visible.is_empty());
        // An index update changes the matching row while the new predicate is pending.
        v.lib
            .photos
            .entry(v.files[11_999].clone())
            .or_default()
            .favorite = false;
        v.lib
            .photos
            .entry(v.files[11_998].clone())
            .or_default()
            .favorite = true;
        v.refresh_index_filter();
        finish_view(&mut v, &ctx);
        assert_eq!(&*v.visible, &[v.files[11_998].clone()]);
        assert_eq!(v.current, Some(v.files[11_998].clone()));
        assert_eq!(v.lib.last_image, v.current);
        // The following folder generation replaces both pending requests and old rows.
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("old");
        let second = dir.path().join("latest");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        v.open(first);
        v.open(second.clone());
        while v.pending_open.is_some() {
            v.retire.pump();
            v.begin_open();
            std::thread::yield_now();
        }
        assert_eq!(v.folder, Some(second));
        assert!(v.visible.is_empty());
        assert!(v.current.is_none());
        v.folders.cancel();
    }
    #[test]
    fn images_chosen_for_moving_are_dropped_when_another_folder_opens() {
        let dir = tempfile::tempdir().unwrap();
        let (first, second) = (dir.path().join("first"), dir.path().join("second"));
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 4);
        v.grid = true;
        v.move_mode = true;
        v.move_selection = v.files[..2].iter().cloned().collect();
        v.delete_selection.insert(v.files[2].clone());
        // Images removed from the open folder leave the selection.
        v.folder_paths = v.files.iter().skip(1).cloned().collect();
        let gone = v.files[0].clone();
        v.remove_from_view(std::slice::from_ref(&gone));
        assert!(!v.move_selection.contains(&gone) && v.move_selection.len() == 1);
        // Another folder: nothing chosen there, Move mode ends.
        v.open(second.clone());
        while v.pending_open.is_some() {
            v.retire.pump();
            v.begin_open();
            std::thread::yield_now();
        }
        assert_eq!(v.folder, Some(second));
        assert!(v.move_selection.is_empty() && !v.move_mode);
        assert!(v.delete_selection.is_empty());
        v.folders.cancel();
    }
    #[test]
    fn every_kind_of_file_dialog_keeps_its_own_state() {
        let kinds = [
            "open",
            "file",
            "destination",
            "tracks",
            "model",
            "runtime",
            "backup",
            "cache_dir",
            "favorite",
            "watch",
            "restore",
        ];
        let guids: HashSet<u128> = kinds.iter().map(|k| dialog_guid(k)).collect();
        assert_eq!(guids.len(), kinds.len());
        // Fixed across versions, so Windows finds the saved place again.
        assert_eq!(
            dialog_guid("open"),
            0x6e6f_6972_7669_4577_9d4c_3a51_7e20_0000
        );
        assert_eq!(dialog_guid("unknown"), dialog_guid("other"));
    }
    #[test]
    fn profiles_are_saved_switched_renamed_and_deleted() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 3);
        v.lib.settings.fit = true;
        v.lib.settings.round_notice_size = 40.;
        v.save_profile("Day");
        assert_eq!(v.lib.settings.active_profile, "Day");
        v.lib.settings.fit = false;
        v.lib.settings.round_notice_size = 20.;
        v.lib.settings.destination = "/moved-to".into();
        assert_eq!(v.lib.settings.profile_differences(), 2);
        v.save_profile("Night");
        v.apply_profile("Day");
        assert!(v.lib.settings.fit && v.lib.settings.round_notice_size == 40.);
        assert_eq!(v.lib.settings.active_profile, "Day");
        assert_eq!(v.lib.settings.destination, PathBuf::from("/moved-to"));
        v.pending_profile = Some("Night".into());
        let name = v.pending_profile.take().unwrap();
        v.apply_profile(&name);
        assert!(!v.lib.settings.fit);
        // Rename keeps it in use; a taken name is refused.
        v.rename_profile("Night", "Evening");
        assert_eq!(v.lib.settings.active_profile, "Evening");
        v.rename_profile("Evening", "Day");
        assert_eq!(v.lib.settings.profiles.len(), 2);
        assert!(v.lib.settings.profiles.iter().any(|p| p.name == "Evening"));
        // Cancel in Settings keeps the profiles saved meanwhile.
        v.settings = true;
        v.settings_snapshot = Some(Box::new(v.lib.settings.clone()));
        v.save_profile("Third");
        v.lib.settings.fit = true;
        v.cancel_settings();
        assert!(!v.lib.settings.fit);
        assert_eq!(v.lib.settings.profiles.len(), 3);
        v.delete_profile("Evening");
        assert!(v.lib.settings.active_profile.is_empty());
        assert_eq!(v.lib.settings.profiles.len(), 2);
    }
    #[test]
    fn a_file_dialog_place_is_kept_per_kind() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.tx.send(Event::DialogPlace("open".into(), [120, 80]))
            .unwrap();
        v.poll(&ctx);
        assert_eq!(v.lib.settings.dialog_places.get("open"), Some(&[120, 80]));
        // Not part of profiles.
        assert!(!v
            .lib
            .settings
            .profile_values()
            .contains_key("dialog_places"));
    }
    #[test]
    fn raising_the_title_does_not_move_noirview() {
        let ctx = egui::Context::default();
        let row = |raise: f32| {
            let mut rect = Rect::NOTHING;
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    rect = ui
                        .horizontal(|ui| title_line(ui, Some("a title.jpg".into()), 600., raise))
                        .response
                        .rect;
                });
            });
            rect
        };
        assert_eq!(row(0.), row(6.));
        assert_eq!(row(0.), row(-6.));
    }
    #[test]
    fn the_floating_move_button_moves_one_image_or_marks_several() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 5);
        let files = v.files.clone();
        // Not marking: a Move dialog for the image shown.
        v.open_one_move(files[0].clone());
        match &v.dialog {
            Some(Dialog::Move(plan)) => {
                assert!(plan.one && !plan.marked);
                assert_eq!(plan.sources, vec![files[0].clone()]);
            }
            _ => panic!("no Move dialog"),
        }
        v.dialog = None;
        // Marking: nothing marked yet says so; marks toggle, kept in the
        // order marked.
        v.marking = true;
        v.enter_move_grid();
        assert!(v.move_grid.is_none());
        v.toggle_mark(&files[3]);
        v.toggle_mark(&files[1]);
        v.toggle_mark(&files[2]);
        v.toggle_mark(&files[2]);
        assert_eq!(v.marked, vec![files[3].clone(), files[1].clone()]);
        // A marked image deleted meanwhile leaves the marks.
        v.folder_paths = v.files.iter().cloned().collect();
        v.remove_from_view(std::slice::from_ref(&files[3]));
        assert_eq!(v.marked, vec![files[1].clone()]);
        // The mark is drawn at the image's corner while marking.
        v.current = Some(files[1].clone());
        v.stop_marking();
        assert!(!v.marking && v.marked.is_empty());
    }
    #[test]
    fn a_favorite_folder_opens_where_browsing_left_it_per_profile() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 12);
        let folder = PathBuf::from("/p");
        v.folder = Some(folder.clone());
        v.loading = false;
        v.lib
            .settings
            .favorite_folders
            .push(crate::model::FavoriteFolder {
                name: String::new(),
                path: folder.clone(),
            });
        v.lib.settings.remember_favorite_places = true;
        v.lib.settings.order = crate::model::Order::Shuffle;
        let mut shown = vec![v.current.clone().unwrap()];
        v.nav.jump(&v.visible, &shown[0]);
        for _ in 0..5 {
            v.navigate(true);
            shown.push(v.current.clone().unwrap());
            v.record_place();
        }
        let place = v.lib.settings.favorite_places[""][&folder].clone();
        assert_eq!(place.image, shown[5]);
        assert_eq!(place.seen.len(), 6);
        // Another profile keeps its own place.
        v.lib.settings.active_profile = "Evening".into();
        v.place_recorded = None;
        v.record_place();
        assert_eq!(v.lib.settings.favorite_places.len(), 2);
        v.lib.settings.active_profile.clear();
        // Opened again (one image deleted, one added meanwhile): the same
        // image, and the round shows only the images not shown yet.
        let mut w = browsing_viewer(&ctx, 12);
        w.files.retain(|p| *p != shown[2]);
        w.files.push("/p/new.jpg".into());
        w.rebuild();
        finish_view(&mut w, &ctx);
        w.lib.settings = v.lib.settings.clone();
        w.folder = Some(folder.clone());
        w.loading = false;
        let place = w.favorite_place(&folder).cloned().unwrap();
        w.resume_place(place, false);
        assert_eq!(w.current.as_ref(), Some(&shown[5]));
        let mut rest = HashSet::new();
        for _ in 0..7 {
            w.navigate(true);
            rest.insert(w.current.clone().unwrap());
        }
        assert_eq!(rest.len(), 7, "the 6 not shown and the new one");
        assert!(rest.contains(Path::new("/p/new.jpg")));
        assert!(shown.iter().all(|p| !rest.contains(p)));
        // Go to image 1 starts over.
        w.go_to(1);
        assert!(w.favorite_place(&folder).is_none());
        // Off: nothing is kept or used.
        w.lib.settings.remember_favorite_places = false;
        w.place_recorded = None;
        w.record_place();
        assert!(w.favorite_place(&folder).is_none());
    }
    #[test]
    fn no_profile_and_the_default_profile_can_be_chosen() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 3);
        assert_eq!(v.profile_names()[..2], ["None", "Default"]);
        v.lib.settings.lens_zoom = 7.;
        v.lib.settings.destination = "/kept".into();
        v.apply_profile(DEFAULT_PROFILE);
        assert_eq!(v.lib.settings.active_profile, "Default");
        assert_eq!(v.lib.settings.lens_zoom, Settings::default().lens_zoom);
        assert_eq!(v.lib.settings.destination, PathBuf::from("/kept"));
        assert_eq!(v.lib.settings.profile_differences(), 0);
        v.lib.settings.lens_zoom = 7.;
        assert_eq!(v.lib.settings.profile_differences(), 1);
        v.apply_profile("");
        assert!(v.lib.settings.active_profile.is_empty());
        assert_eq!(v.lib.settings.lens_zoom, 7., "no profile changes nothing");
    }
    #[test]
    fn a_popup_without_room_above_its_button_opens_below() {
        let ctx = egui::Context::default();
        let show = |anchor: Pos2| {
            let mut rect = Rect::NOTHING;
            for _ in 0..3 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    rect = near(
                        ctx,
                        Id::new(("p", anchor.y as i32)),
                        anchor,
                        anchor + Vec2::new(0., 20.),
                        |ui| {
                            ui.add_space(120.);
                            ui.label("entries");
                        },
                    );
                });
            }
            rect
        };
        assert!(show(Pos2::new(400., 500.)).bottom() <= 500.);
        assert!(show(Pos2::new(400., 40.)).top() >= 60.);
    }
    #[test]
    fn the_marked_and_selected_to_move_filters_show_only_those_images() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 6);
        let files = v.files.clone();
        v.toggle_mark(&files[1]);
        v.toggle_mark(&files[4]);
        v.filter.marked = true;
        v.rebuild();
        finish_view(&mut v, &ctx);
        assert_eq!(&*v.visible, &[files[1].clone(), files[4].clone()]);
        // A mark taken off while the filter is on leaves the view.
        v.toggle_mark(&files[4]);
        finish_view(&mut v, &ctx);
        assert_eq!(&*v.visible, &[files[1].clone()]);
        // The grid's Move selection, and both together.
        v.filter.marked = false;
        v.filter.chosen = true;
        v.move_selection = [files[1].clone(), files[2].clone()].into_iter().collect();
        v.rebuild();
        finish_view(&mut v, &ctx);
        assert_eq!(&*v.visible, &[files[1].clone(), files[2].clone()]);
        v.filter.marked = true;
        v.rebuild();
        finish_view(&mut v, &ctx);
        assert_eq!(&*v.visible, &[files[1].clone()]);
        assert!(v.filter.active());
    }
    #[test]
    fn a_press_in_a_popup_from_the_group_bar_keeps_the_group_open() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 3);
        v.bar = true;
        v.group_open = Some("toolbar");
        let frame = |v: &mut Viewer, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                v.toolbar(ctx);
                v.button_popups(ctx, false);
            });
        };
        frame(&mut v, vec![]);
        frame(&mut v, vec![]);
        let group = ctx.memory(|m| m.area_rect(Id::new("group_bar"))).unwrap();
        // The Profile button is the group's last.
        let button =
            Rect::from_center_size(group.right_center() - Vec2::new(30., 0.), Vec2::splat(30.));
        v.toggle_profiles_popup(button);
        frame(&mut v, vec![]);
        frame(&mut v, vec![]);
        let popup = v.popup_rect.unwrap();
        let at = Pos2::new(popup.center().x, popup.top() + 8.);
        frame(&mut v, vec![egui::Event::PointerMoved(at)]);
        frame(
            &mut v,
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(
            v.group_open,
            Some("toolbar"),
            "the group waits for the popup"
        );
        assert!(v.popup.is_some());
    }
    #[test]
    fn the_move_grid_shows_the_marked_images_and_goes_back_to_the_image() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 6);
        let files = v.files.clone();
        v.current = Some(files[4].clone());
        v.lib.settings.thumbnail_count = 9;
        v.lib.settings.move_grid_count = 2;
        v.lib.settings.move_grid_round_full = false;
        v.marking = true;
        for p in [&files[0], &files[2], &files[3]] {
            v.toggle_mark(p);
        }
        v.enter_move_grid();
        finish_view(&mut v, &ctx);
        assert!(v.grid && v.move_grid.is_some());
        assert_eq!(
            &*v.visible,
            &[files[0].clone(), files[2].clone(), files[3].clone()]
        );
        // Its own page size; the grid's is kept.
        assert_eq!(v.grid_count(), 2);
        v.set_grid_count(3);
        assert_eq!(v.lib.settings.move_grid_count, 3);
        assert_eq!(v.lib.settings.thumbnail_count, 9);
        // Its bar: the grid's controls that apply and its own.
        let (items, _) = v.grid_items(true);
        for item in [
            GridItem::SelectBoth,
            GridItem::Destination,
            GridItem::KeepNames,
            GridItem::MoveMarked,
            GridItem::CancelMarked,
            GridItem::HoverPreview,
            GridItem::Trash,
        ] {
            assert!(items.contains(&item), "{item:?}");
        }
        // × takes one off; Move moves the selected (none: all).
        v.take_off_marked(&files[2]);
        finish_view(&mut v, &ctx);
        assert_eq!(&*v.visible, &[files[0].clone(), files[3].clone()]);
        v.move_marked();
        match &v.dialog {
            Some(Dialog::Move(plan)) => {
                assert!(plan.marked);
                assert_eq!(plan.sources, vec![files[0].clone(), files[3].clone()]);
            }
            _ => panic!("no Move dialog"),
        }
        v.dialog = None;
        // Images chosen for deleting are not moved; Move selected mode (as
        // in the grid) moves the chosen ones.
        v.delete_selection.insert(files[0].clone());
        v.move_clicked();
        let Some(Dialog::Move(plan)) = &v.dialog else {
            panic!("no Move dialog");
        };
        assert_eq!(plan.sources.len(), 2, "all, not the ones to delete");
        v.dialog = None;
        v.delete_selection.clear();
        v.toggle_move_mode();
        v.move_selection.insert(files[3].clone());
        v.move_clicked();
        let Some(Dialog::Move(plan)) = &v.dialog else {
            panic!("no Move dialog");
        };
        assert!(plan.marked);
        assert_eq!(plan.sources, vec![files[3].clone()]);
        v.dialog = None;
        v.toggle_move_mode();
        // All on one page: the page goes on, a page or a row at a time.
        for p in &files {
            if !v.marked.contains(p) {
                v.toggle_mark(p);
            }
        }
        finish_view(&mut v, &ctx);
        v.lib.settings.move_grid_all = true;
        v.lib.settings.move_grid_count = 2;
        let (items, _) = v.grid_items(true);
        assert!(items.contains(&GridItem::Size) && items.contains(&GridItem::NextPage));
        let frame = |v: &mut Viewer| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| v.central(ctx));
        };
        frame(&mut v);
        let columns = v.grid_columns;
        assert!(columns >= 1 && v.grid_count() == 2);
        let page = 2usize.div_ceil(columns) * columns;
        v.grid_page(true);
        assert_eq!(
            v.grid_start,
            page.min(
                6usize
                    .div_ceil(columns)
                    .saturating_sub(2usize.div_ceil(columns))
                    * columns
            )
        );
        assert!(v.move_grid_scroll > 0., "slides up into place");
        v.grid_page(false);
        assert_eq!(v.grid_start, 0);
        v.lib.settings.move_grid_row = true;
        v.grid_page(true);
        assert!(v.grid_start <= columns, "one row");
        for _ in 0..40 {
            frame(&mut v);
        }
        // Cancel: the image view as it was, the marks kept.
        v.exit_move_grid();
        finish_view(&mut v, &ctx);
        assert!(!v.grid && v.move_grid.is_none());
        assert_eq!(v.visible.len(), 6);
        assert_eq!(v.current.as_ref(), Some(&files[4]));
        assert_eq!(v.marked.len(), 6, "all marked for the scrolling page");
        assert!(v.delete_selection.is_empty());
    }
    #[test]
    fn the_x_and_the_mark_go_to_their_corner_moved_in_as_set() {
        let inner = Rect::from_min_size(Pos2::new(100., 100.), Vec2::new(300., 200.));
        let size = Vec2::new(20., 20.);
        assert_eq!(
            corner_place(inner, size, "top_left", [0., 0.]),
            Pos2::new(100., 100.)
        );
        assert_eq!(
            corner_place(inner, size, "top_left", [0., 30.]),
            Pos2::new(100., 130.)
        );
        assert_eq!(
            corner_place(inner, size, "top_right", [5., 0.]),
            Pos2::new(375., 100.)
        );
        assert_eq!(
            corner_place(inner, size, "bottom_right", [0., 0.]),
            Pos2::new(380., 280.)
        );
        assert_eq!(
            corner_place(inner, size, "bottom_left", [10., 10.]),
            Pos2::new(110., 270.)
        );
        // Kept inside however far.
        assert_eq!(
            corner_place(inner, size, "top_left", [999., 999.]),
            Pos2::new(380., 280.)
        );
        let d = Settings::default();
        assert_eq!(
            (d.move_x_corner.as_str(), d.move_mark_corner.as_str()),
            ("top_left", "top_left")
        );
        assert!(
            d.move_mark_offset[1] >= d.move_x_size,
            "the mark below the × by default"
        );
    }
    #[test]
    fn the_move_grids_settings_can_be_changed_without_it() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 3);
        v.lib.settings.hover_preview_ms = 400;
        v.settings = true;
        v.edit_grid_settings_of(true);
        assert_eq!(
            v.lib.settings.hover_preview_ms, 400,
            "first time: the grid's"
        );
        v.lib.settings.hover_preview_ms = 1200;
        v.edit_grid_settings_of(false);
        assert_eq!(v.lib.settings.hover_preview_ms, 400);
        // Settings closed while showing the Move grid's: they are kept.
        v.edit_grid_settings_of(true);
        assert_eq!(v.lib.settings.hover_preview_ms, 1200);
        v.settings = false;
        let _ = ctx.run(egui::RawInput::default(), |ctx| v.settings_window(ctx));
        assert_eq!(v.lib.settings.hover_preview_ms, 400);
        assert!(!v.settings_move_target);
        // And used when the Move grid opens.
        v.marking = true;
        let first = v.files[0].clone();
        v.toggle_mark(&first);
        v.enter_move_grid();
        assert_eq!(v.lib.settings.hover_preview_ms, 1200);
        v.exit_move_grid();
        assert_eq!(v.lib.settings.hover_preview_ms, 400);
    }
    #[test]
    fn settings_pages_draw_search_and_reset() {
        let ctx = egui::Context::default();
        let mut v = browsing_viewer(&ctx, 3);
        // Every page draws.
        for page in 0..SETTINGS_PAGES.len() {
            let mut invalidate = false;
            let mut actions = vec![];
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    v.settings_controls(ui, page, &mut invalidate, &mut actions);
                });
            });
            // Folders & tags asks for the service's state when shown.
            actions.retain(|a| *a != "usn_refresh");
            assert!(
                actions.is_empty(),
                "page {page} acted by itself: {actions:?}"
            );
        }
        // Search: "circle" is on the Grid page only; "lens" on Image view.
        let count = |v: &mut Viewer, q: &str| {
            v.settings_query = q.into();
            let mut counts = vec![];
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    counts = v.settings_search_counts(ui);
                });
            });
            counts
        };
        let circle = count(&mut v, "Circle");
        assert_eq!(circle.len(), SETTINGS_PAGES.len());
        assert!(circle[2] >= 3, "{circle:?}");
        assert_eq!(circle.iter().sum::<usize>(), circle[2]);
        assert!(count(&mut v, "lens")[0] >= 1);
        // Words in any order; texts and section titles count too.
        assert!(count(&mut v, "grid wrap")[2] >= 1);
        assert!(
            count(&mut v, "mp3")[7] >= 1,
            "the Music page's formats line"
        );
        assert!(count(&mut v, "slideshow")[1] >= 1);
        // Looser matching: a letter wrong or missing, the start of a word.
        assert!(count(&mut v, "magnifer").iter().all(|n| *n == 0));
        assert!(v.settings_nothing_found());
        v.lib.settings.settings_search_loose = true;
        assert!(count(&mut v, "magnifer")[0] >= 1);
        assert!(count(&mut v, "slidesh")[1] >= 1);
        assert!(!v.settings_nothing_found());
        v.lib.settings.settings_search_loose = false;
        assert!(count(&mut v, "").is_empty(), "no search");
        assert!(!v.settings_nothing_found());
        assert!(v.hover.search.is_empty() && v.hover.found.is_empty());
        // Reset: the page's settings back, folders and lists kept.
        let all = serde_json::to_value(Settings::default()).unwrap();
        let keep = [
            "destination",
            "favorite_folders",
            "watched_folders",
            "music",
            "profiles",
            "thumbnail_cache_dir",
            "model_path",
            "ort_library",
            "backup_extra",
            "delete_pos",
        ];
        for keys in SETTINGS_RESET {
            for key in keys {
                assert!(all.get(*key).is_some(), "{key} is not a setting");
                assert!(!keep.contains(key), "{key} is not reset");
            }
        }
        v.lib.settings.lens_zoom = 9.;
        v.lib.settings.destination = "/kept".into();
        v.lib.settings.reset_values(SETTINGS_RESET[0]);
        assert_eq!(v.lib.settings.lens_zoom, Settings::default().lens_zoom);
        assert_eq!(v.lib.settings.destination, PathBuf::from("/kept"));
    }
    #[test]
    fn search_words_match_in_any_order_and_loosely_with_typos() {
        assert!(search_matches("Grid pages wrap around", "grid wrap", false));
        assert!(search_matches("Grid pages wrap around", "wrap grid", false));
        assert!(!search_matches(
            "Grid pages wrap around",
            "grid wrap circle",
            false
        ));
        assert!(!search_matches("Magnifier border", "magnifer", false));
        assert!(search_matches("Magnifier border", "magnifer", true));
        assert!(search_matches("Magnifier border", "magn", true));
        assert!(search_matches(
            "Thumbnail disk cache (MiB)",
            "thumbnial",
            true
        ));
        assert!(!search_matches("Volume", "colour", true), "too far");
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("wrap", "wrpa"), 2);
        assert_eq!(
            Settings::default().watch_pace,
            crate::model::WatchPace::Balanced
        );
    }
    #[test]
    fn unreadable_startup_cannot_backup_or_save_default_state_over_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store {
            path: dir.path().join("library.json"),
            exe_dir: dir.path().into(),
        };
        std::fs::write(&store.path, b"malformed original").unwrap();
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.install_library(
            Loaded {
                library: store.load().map_err(|e| e.to_string()),
                store: Some(store.clone()),
            },
            &ctx,
        );
        assert!(!v.persist_allowed);
        assert!(v.dialog.is_some());
        v.backup(None);
        v.flush_before_close(&ctx);
        v.writer.take().unwrap().shutdown();
        assert_eq!(std::fs::read(&store.path).unwrap(), b"malformed original");
        assert!(!crate::state::database_path(&store).exists());
        assert!(!dir.path().join("backups").exists());
    }
    #[test]
    fn unreadable_library_error_is_painted_inside_the_viewport() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.dialog = Some(Dialog::Error(
            "Cannot read library; original preserved".into(),
        ));
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.));
        let mut seen = false;
        for _ in 0..4 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ctx| {
                    v.hover.begin(&v.lib.settings);
                    v.central(ctx);
                    v.dialogs(ctx);
                },
            );
            for clipped in output.shapes {
                if let egui::Shape::Text(text) = clipped.shape {
                    if text.galley.job.text.contains("Action needs attention") {
                        let rect = Rect::from_min_size(text.pos, text.galley.size());
                        seen |= screen.intersects(rect) && clipped.clip_rect.intersects(rect);
                    }
                }
            }
        }
        assert!(
            seen,
            "load error must be visible, modal rect={:?}",
            ctx.memory(|m| m.area_rect(Id::new("confirmation")))
        );
    }
    #[test]
    fn raw_wheel_in_the_viewer_scrolls_up_to_next_and_setting_reversal_takes_effect() {
        use egui::{Event, Modifiers, MouseWheelUnit};
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..5000).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        let raw = |events| egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
            events,
            ..Default::default()
        };
        for _ in 0..2 {
            let _ = ctx.run(
                raw(vec![Event::PointerMoved(Pos2::new(640., 410.))]),
                |ctx| {
                    v.central(ctx);
                    v.edges(ctx);
                },
            );
        }
        for i in 1..=4 {
            let _ = ctx.run(
                raw(vec![Event::MouseWheel {
                    unit: MouseWheelUnit::Line,
                    delta: Vec2::new(0., 1.),
                    modifiers: Modifiers::NONE,
                }]),
                |ctx| {
                    v.central(ctx);
                    v.edges(ctx);
                },
            );
            assert_eq!(v.current, Some(v.files[i].clone()));
        }
        let _ = ctx.run(
            raw(vec![Event::MouseWheel {
                unit: MouseWheelUnit::Line,
                delta: Vec2::new(0., -1.),
                modifiers: Modifiers::NONE,
            }]),
            |ctx| {
                v.central(ctx);
                v.edges(ctx);
            },
        );
        assert_eq!(v.current, Some(v.files[3].clone()));
        v.lib.settings.wheel_up_next = false;
        let _ = ctx.run(
            raw(vec![Event::MouseWheel {
                unit: MouseWheelUnit::Line,
                delta: Vec2::new(0., -1.),
                modifiers: Modifiers::NONE,
            }]),
            |ctx| {
                v.central(ctx);
                v.edges(ctx);
            },
        );
        assert_eq!(v.current, Some(v.files[4].clone()));
    }
    #[test]
    fn random_and_shuffle_order_can_give_a_new_image_for_both_wheel_directions() {
        use egui::{Event, Modifiers, MouseWheelUnit};
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..500).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.lib.settings.order = crate::model::Order::Random;
        v.rebuild();
        finish_view(&mut v, &ctx);
        let wheel = |v: &mut Viewer, delta: f32| {
            let events = if delta == 0. {
                vec![Event::PointerMoved(Pos2::new(640., 410.))]
            } else {
                vec![Event::MouseWheel {
                    unit: MouseWheelUnit::Line,
                    delta: Vec2::new(0., delta),
                    modifiers: Modifiers::NONE,
                }]
            };
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                v.central(ctx);
                v.edges(ctx);
            });
        };
        wheel(&mut v, 0.);
        wheel(&mut v, 0.);
        // Default: up gives a new image, down walks back through the history.
        wheel(&mut v, 1.);
        let first = v.current.clone();
        wheel(&mut v, 1.);
        let seen = v.nav.history_len();
        wheel(&mut v, -1.);
        assert_eq!(v.current, first);
        assert_eq!(v.nav.history_len(), seen);
        // Setting on: down also asks for a new random image.
        v.lib.settings.random_wheel_both = true;
        wheel(&mut v, 1.);
        wheel(&mut v, 1.);
        let before = v.nav.history_len();
        wheel(&mut v, -1.);
        wheel(&mut v, -1.);
        assert_eq!(v.nav.history_len(), before + 2);
        // Shuffle too: down gives the deck's next, not yet shown image.
        v.lib.settings.order = crate::model::Order::Shuffle;
        wheel(&mut v, 1.);
        let mut seen: Vec<_> = vec![v.current.clone().unwrap()];
        let before = v.nav.history_len();
        for _ in 0..3 {
            wheel(&mut v, -1.);
            let now = v.current.clone().unwrap();
            assert!(!seen.contains(&now), "shuffle repeated {now:?}");
            seen.push(now);
        }
        assert_eq!(v.nav.history_len(), before + 3);
        // Setting off: Shuffle's down walks back again.
        v.lib.settings.random_wheel_both = false;
        let at = v.current.clone();
        wheel(&mut v, 1.);
        assert_ne!(v.current, at);
        wheel(&mut v, -1.);
        assert_eq!(v.current, at);
        // Sequential order is not affected by the setting.
        v.lib.settings.random_wheel_both = true;
        v.lib.settings.order = crate::model::Order::Sequential;
        let at = v.current.clone().unwrap();
        wheel(&mut v, -1.);
        let i = v.visible.position(&at).unwrap();
        let back = (i + v.visible.len() - 1) % v.visible.len();
        assert_eq!(v.current, Some(v.visible[back].clone()));
    }
    fn delete_dialog(v: &Viewer) -> Option<Vec<PathBuf>> {
        match &v.dialog {
            Some(Dialog::Delete(paths)) => Some(paths.clone()),
            _ => None,
        }
    }
    #[test]
    fn floating_trash_deletes_the_grid_selection_the_big_image_or_the_current_image() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..8).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        let f = v.visible.iter().cloned().collect::<Vec<_>>();
        v.lib.settings.confirm_delete = true;
        v.delete_float = true;
        let frame = |v: &mut Viewer, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| v.floating_delete(ctx));
        };
        let click = |v: &mut Viewer| {
            frame(v, vec![]);
            frame(v, vec![]);
            let min = v.lib.settings.delete_pos.unwrap();
            let size = v.lib.settings.delete_size;
            let at = Pos2::new(min[0], min[1])
                + Vec2::splat(close_margin(v.lib.settings.close_size) + size / 2.);
            frame(v, vec![egui::Event::PointerMoved(at)]);
            for pressed in [true, false] {
                frame(
                    v,
                    vec![egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
        };
        // Grid, current image 6, images 1 and 4 selected: those two.
        v.grid = true;
        v.current = Some(f[6].clone());
        click(&mut v);
        assert_eq!(delete_dialog(&v), None, "no selection: nothing to delete");
        v.delete_selection.insert(f[4].clone());
        v.delete_selection.insert(f[1].clone());
        assert_eq!(
            v.floating_delete_targets(),
            vec![f[1].clone(), f[4].clone()]
        );
        click(&mut v);
        assert_eq!(delete_dialog(&v), Some(vec![f[1].clone(), f[4].clone()]));
        v.dialog = None;
        // Normal mode: like the grid's own Trash button, nothing.
        v.grid_normal = true;
        assert!(v.floating_delete_targets().is_empty());
        v.grid_normal = false;
        // The big image opened from the grid: that image.
        v.grid_viewer = Some(f[2].clone());
        assert_eq!(v.floating_delete_targets(), vec![f[2].clone()]);
        // Single image view: the current image.
        v.grid_viewer = None;
        v.grid = false;
        assert_eq!(v.floating_delete_targets(), vec![f[6].clone()]);
        click(&mut v);
        assert_eq!(delete_dialog(&v), Some(vec![f[6].clone()]));
    }
    #[test]
    fn deleted_tiles_stay_marked_when_the_folder_reports_the_removal_first() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let paths: Vec<PathBuf> = (0..8).map(|n| PathBuf::from(format!("{n}.jpg"))).collect();
        v.visible = paths.clone().into();
        v.files = paths.clone();
        v.grid = true;
        v.lib.settings.grid_round_full = false;
        v.lib.settings.thumbnail_count = 6;
        let gone = [paths[1].clone(), paths[4].clone()];
        // The deletion starts: the page is kept as it is now.
        v.freeze_grid_page(&gone);
        // The folder watcher reports both removals before the deletion ends.
        v.visible = paths
            .iter()
            .filter(|p| !gone.contains(p))
            .cloned()
            .collect::<Vec<_>>()
            .into();
        v.files.retain(|p| !gone.contains(p));
        v.remove_from_view(&gone);
        let g = v.grid_ghosts.as_ref().unwrap();
        assert_eq!(g.page, paths[..6].to_vec());
        assert_eq!(g.deleted, gone.iter().cloned().collect());
        // A deletion that removed nothing leaves no frozen page behind.
        let mut v = viewer(&ctx);
        v.visible = paths.clone().into();
        v.grid = true;
        v.freeze_grid_page(&gone);
        v.remove_from_view(&[]);
        assert!(v.grid_ghosts.is_none());
    }
    #[test]
    fn floating_trash_over_the_big_image_from_the_grid_deletes_instead_of_returning() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..4).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        let f = v.visible.iter().cloned().collect::<Vec<_>>();
        v.lib.settings.confirm_delete = true;
        v.delete_float = true;
        v.grid = true;
        v.grid_viewer = Some(f[2].clone());
        v.current = Some(f[2].clone());
        let frame = |v: &mut Viewer, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                v.central(ctx);
                v.floating_delete(ctx);
            });
        };
        let click = |v: &mut Viewer, at: Pos2| {
            frame(v, vec![egui::Event::PointerMoved(at)]);
            for pressed in [true, false] {
                frame(
                    v,
                    vec![egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
        };
        frame(&mut v, vec![]);
        frame(&mut v, vec![]);
        let min = v.lib.settings.delete_pos.unwrap();
        let trash = Pos2::new(min[0], min[1])
            + Vec2::splat(
                close_margin(v.lib.settings.close_size) + v.lib.settings.delete_size / 2.,
            );
        click(&mut v, trash);
        assert_eq!(delete_dialog(&v), Some(vec![f[2].clone()]));
        assert_eq!(v.grid_viewer, Some(f[2].clone()), "stayed on the big image");
        // A click on the image itself still returns to the grid.
        v.dialog = None;
        click(&mut v, Pos2::new(400., 400.));
        assert_eq!(v.grid_viewer, None);
    }
    #[test]
    fn favorites_on_the_grid_page_apply_only_to_the_tag_selection() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let paths: Vec<PathBuf> = (0..4).map(|n| PathBuf::from(format!("{n}.jpg"))).collect();
        v.visible = paths.clone().into();
        v.files = paths.clone();
        v.current = Some(paths[0].clone());
        v.grid = true;
        // Tags from the toolbar on the grid page: tag mode, nothing selected.
        v.toggle_grid_tags();
        assert!(v.tag_mode && v.panel == Panel::Tags);
        assert!(
            v.favorite_targets().is_empty(),
            "not the page's first image"
        );
        v.tag_selection.insert(paths[2].clone());
        assert_eq!(v.favorite_targets(), vec![paths[2].clone()]);
        // Without tag mode the grid page has no target either.
        v.toggle_grid_tags();
        assert!(!v.tag_mode && v.favorite_targets().is_empty());
        // The big image from the grid and the single view: the shown image.
        v.grid_viewer = Some(paths[1].clone());
        v.current = Some(paths[1].clone());
        assert_eq!(v.favorite_targets(), vec![paths[1].clone()]);
        v.grid = false;
        v.grid_viewer = None;
        assert_eq!(v.favorite_targets(), vec![paths[1].clone()]);
    }
    #[test]
    fn grid_icon_bar_is_fixed_or_opens_like_the_toolbar_and_its_icons_work() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..15).map(|n| format!("{n:02}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        v.grid = true;
        v.lib.settings.grid_round_full = false;
        v.lib.settings.thumbnail_count = 12;
        v.lib.settings.grid_bar_icons = true;
        let frame = |v: &mut Viewer, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                v.central(ctx);
                v.edges(ctx);
                v.toolbar(ctx);
                v.grid_icon_bar(ctx);
            });
        };
        let rect = |name: &str| ctx.memory(|m| m.area_rect(Id::new(name)));
        // Always shown: the tiles end above it; the toolbar is hidden.
        v.bar = true;
        for _ in 0..3 {
            frame(&mut v, vec![]);
        }
        let bar = rect("grid_icon_bar").unwrap();
        assert!(v.grid_icon_bar_shown() && !v.toolbar_shown());
        let grid = v.grid_area.unwrap();
        assert!(grid.height <= bar.top(), "{} vs {bar:?}", grid.height);
        assert!(bar.height() < 70., "one row: {bar:?}");
        // Its Next page icon pages forward (second icon of the bar).
        let step = 38. + ctx.style().spacing.item_spacing.x;
        let next = Pos2::new(bar.left() + 12. + 19. + step, bar.center().y);
        frame(&mut v, vec![egui::Event::PointerMoved(next)]);
        for pressed in [true, false] {
            frame(
                &mut v,
                vec![egui::Event::PointerButton {
                    pos: next,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert_eq!(v.grid_start, 12, "Next page from the icon bar");
        // The page order choice opens above its icon.
        v.grid_menu = Some((GridMenu::Pages, bar));
        frame(&mut v, vec![]);
        frame(&mut v, vec![]);
        let menu = rect("grid_bar_menu").unwrap();
        assert!(menu.bottom() <= bar.top());
        v.grid_menu = None;
        // Like the toolbar, with the toolbar stacked above it.
        v.lib.settings.grid_bar_fixed = false;
        v.lib.settings.grid_hide_toolbar = false;
        v.bar = false;
        frame(
            &mut v,
            vec![egui::Event::PointerMoved(Pos2::new(640., 300.))],
        );
        assert!(!v.grid_icon_bar_shown());
        v.bar = true;
        for _ in 0..3 {
            frame(&mut v, vec![]);
        }
        assert!(v.grid_icon_bar_shown() && v.toolbar_shown());
        let (bar, toolbar) = (rect("grid_icon_bar").unwrap(), rect("toolbar").unwrap());
        assert!(
            toolbar.bottom() <= bar.top() - 7.,
            "{toolbar:?} above {bar:?}"
        );
        assert_eq!(v.bars_top(&ctx), Some(toolbar.top()));
        // Outside the grid page the icon bar is not shown.
        v.grid_viewer = Some(v.files[0].clone());
        assert!(!v.grid_icon_bar_shown() && v.toolbar_shown());
    }
    #[test]
    fn filters_are_kept_per_view_or_shared_and_random_pages_need_more_than_one_page() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..30).map(|n| format!("{n:02}.jpg").into()).collect();
        for p in &v.files[..2] {
            v.lib.photos.entry(p.clone()).or_default().favorite = true;
        }
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        v.toggle_grid();
        finish_view(&mut v, &ctx);
        // A favorites filter in the grid.
        v.filter.favorite = true;
        v.rebuild();
        finish_view(&mut v, &ctx);
        assert_eq!(v.visible.len(), 2);
        // Random order with everything on one page: no paging.
        v.lib.settings.grid_page_order = crate::model::Order::Random;
        v.lib.settings.thumbnail_count = 12;
        v.lib.settings.grid_round_full = false;
        let _ = v.page_paths();
        v.grid_page(true);
        v.grid_page(true);
        assert_eq!(v.grid_pages.number(), 1, "one page: the page stays");
        // The image view has its own (no) filter; the grid gets its back.
        v.toggle_grid();
        finish_view(&mut v, &ctx);
        assert!(!v.filter.active());
        assert_eq!(v.visible.len(), 30);
        v.toggle_grid();
        finish_view(&mut v, &ctx);
        assert!(v.filter.favorite);
        assert_eq!(v.visible.len(), 2);
        // One filter for both views.
        v.lib.settings.filter_per_view = false;
        v.toggle_grid();
        finish_view(&mut v, &ctx);
        assert!(v.filter.favorite && v.visible.len() == 2);
        v.toggle_grid();
        // No history: Previous also draws a new page.
        v.filter = Filter::default();
        v.rebuild();
        finish_view(&mut v, &ctx);
        let _ = v.page_paths();
        let before = v.grid_pages.number();
        v.grid_page(false);
        assert_eq!(
            v.grid_pages.number(),
            before,
            "history: Previous stays at the start"
        );
        v.lib.settings.grid_random_both = true;
        v.grid_page(false);
        assert_eq!(v.grid_pages.number(), before + 1);
    }
    #[test]
    fn the_grid_opens_at_the_page_of_the_image_being_viewed() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..30).map(|n| format!("{n:02}.jpg").into()).collect();
        v.current = Some(v.files[13].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        v.lib.settings.thumbnail_count = 12;
        v.lib.settings.grid_round_full = false;
        v.lib.settings.grid_open_at_current = true;
        v.toggle_grid();
        finish_view(&mut v, &ctx);
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| v.central(ctx));
        assert_eq!(v.grid_start, 12);
    }
    #[test]
    fn slipping_out_of_a_window_at_the_bottom_does_not_close_the_bar_it_opened() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..3).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        v.lib.settings.bar_delay_ms = 0;
        let run = |v: &mut Viewer, event: egui::Event| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                events: vec![event],
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| v.edges(ctx));
        };
        let bottom = || egui::Event::PointerMoved(Pos2::new(640., 818.));
        run(&mut v, bottom());
        assert!(v.bar);
        // Out through the bottom and straight back in: stays open.
        for _ in 0..3 {
            run(&mut v, egui::Event::PointerGone);
            run(&mut v, bottom());
            assert!(v.bar);
        }
        // Up into the window, then back to the edge: closes as before.
        run(&mut v, egui::Event::PointerMoved(Pos2::new(640., 500.)));
        run(&mut v, bottom());
        assert!(!v.bar);
    }
    #[test]
    fn settings_cancel_goes_back_to_the_settings_when_the_window_opened() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let frame = |v: &mut Viewer| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| v.settings_window(ctx));
        };
        v.lib.settings.side_px = 30.;
        v.lib.settings.grid_bar_icons = false;
        v.settings = true;
        frame(&mut v);
        v.lib.settings.side_px = 400.;
        v.lib.settings.grid_bar_icons = true;
        frame(&mut v);
        v.cancel_settings();
        assert!(!v.settings);
        assert_eq!(v.lib.settings.side_px, 30.);
        assert!(!v.lib.settings.grid_bar_icons);
        // Done keeps changes; the next opening takes a new snapshot.
        v.settings = true;
        frame(&mut v);
        v.lib.settings.side_px = 50.;
        v.settings = false;
        frame(&mut v);
        v.settings = true;
        frame(&mut v);
        v.cancel_settings();
        assert_eq!(v.lib.settings.side_px, 50.);
    }
    #[test]
    fn an_open_tag_panel_stays_put_when_the_bars_below_it_hide() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..15).map(|n| format!("{n:02}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        v.grid = true;
        v.lib.settings.grid_bar_icons = true;
        v.lib.settings.grid_bar_fixed = false;
        v.lib.settings.grid_bar_bottom = 200.;
        v.toggle_grid_tags();
        let frame = |v: &mut Viewer| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                v.central(ctx);
                v.toolbar(ctx);
                v.grid_icon_bar(ctx);
                v.panels(ctx);
            });
        };
        let panel = || {
            ctx.memory(|m| m.area_rect(Id::new("action_panel")))
                .unwrap()
        };
        v.bar = true;
        for _ in 0..3 {
            frame(&mut v);
        }
        let open = panel();
        let bar = ctx
            .memory(|m| m.area_rect(Id::new("grid_icon_bar")))
            .unwrap();
        assert!(open.bottom() <= bar.top());
        v.bar = false;
        for _ in 0..3 {
            frame(&mut v);
        }
        assert_eq!(panel().bottom(), open.bottom(), "stays where it was");
    }
    #[test]
    fn move_selected_plans_only_the_chosen_images_and_keeps_names_on_request() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let dir = tempfile::tempdir().unwrap();
        let (src, dest) = (dir.path().join("src"), dir.path().join("dest"));
        std::fs::create_dir(&src).unwrap();
        std::fs::create_dir(&dest).unwrap();
        let paths: Vec<PathBuf> = (0..4).map(|n| src.join(format!("img{n}.jpg"))).collect();
        for p in &paths {
            std::fs::write(p, b"x").unwrap();
        }
        v.visible = paths.clone().into();
        v.files = paths.clone();
        v.grid = true;
        v.lib.settings.destination = dest.clone();
        // A click without the mode: the whole folder.
        v.move_clicked();
        match &v.dialog {
            Some(Dialog::Move(plan)) => {
                assert!(!plan.selected && plan.sources.len() == 4);
                let first = &plan.items.as_ref().unwrap()[0];
                assert_eq!(first.target.file_name().unwrap(), "001.jpg");
            }
            _ => panic!("no Move dialog"),
        }
        v.dialog = None;
        // Move selected: images 3 and 1, in the grid's order, original names.
        v.lib.settings.move_keep_names = true;
        v.move_mode = true;
        v.move_selection = [paths[3].clone(), paths[1].clone()].into_iter().collect();
        v.move_clicked();
        match &v.dialog {
            Some(Dialog::Move(plan)) => {
                assert!(plan.selected && plan.keep_names);
                assert_eq!(plan.sources, vec![paths[1].clone(), paths[3].clone()]);
                let items = plan.items.as_ref().unwrap();
                // The plan uses the canonical destination (\\?\ on Windows).
                assert_eq!(
                    items[0].target,
                    dest.canonicalize().unwrap().join("img1.jpg")
                );
            }
            _ => panic!("no Move dialog"),
        }
        // Nothing chosen: the click leaves the mode.
        v.dialog = None;
        v.move_selection.clear();
        v.move_clicked();
        assert!(!v.move_mode && v.dialog.is_none());
    }
    #[test]
    fn the_big_image_from_the_grid_has_its_own_bar_and_delete_always_asks() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..3).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[1].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        v.grid = true;
        v.grid_viewer = Some(v.files[1].clone());
        v.lib.settings.confirm_delete = false;
        v.bar = true;
        let frame = |v: &mut Viewer, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                v.toolbar(ctx);
                v.prompt_window(ctx);
            });
        };
        frame(&mut v, vec![]);
        frame(&mut v, vec![]);
        let bar = ctx.memory(|m| m.area_rect(Id::new("toolbar"))).unwrap();
        let step = 38. + ctx.style().spacing.item_spacing.x;
        let icon = |n: f32| Pos2::new(bar.left() + 12. + 19. + n * step, bar.center().y);
        let click = |v: &mut Viewer, at: Pos2| {
            frame(v, vec![egui::Event::PointerMoved(at)]);
            for pressed in [true, false] {
                frame(
                    v,
                    vec![egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
        };
        // First icon: delete, with the dialog although confirmation is off.
        click(&mut v, icon(0.));
        assert_eq!(delete_dialog(&v), Some(vec![v.files[1].clone()]));
        v.dialog = None;
        // Fourth icon: the prompt window for this image.
        click(&mut v, icon(3.));
        assert_eq!(v.prompt_window, Some(v.files[1].clone()));
        frame(&mut v, vec![]);
        assert!(ctx
            .memory(|m| m.area_rect(Id::new("Prompt and seed")))
            .is_some());
    }
    #[test]
    fn grid_controls_keep_their_width_and_overflow_behind_more_in_a_narrow_window() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..300).map(|n| format!("{n:03}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        v.grid = true;
        v.lib.settings.grid_round_full = false;
        // Nothing in the group: only what does not fit goes behind ».
        let slots = crate::bar_layout::placed(&grid_defaults(), None, "goto", Place::Bar);
        let slots =
            crate::bar_layout::placed(&grid_defaults(), Some(&slots), "profile", Place::Bar);
        v.lib.settings.bar_buttons.insert("grid".into(), slots);
        v.lib.settings.grid_bar_icons = true;
        v.lib.settings.thumbnail_count = 9;
        let frame = |v: &mut Viewer, width: f32, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 820.))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                v.central(ctx);
                v.grid_icon_bar(ctx);
            });
        };
        let bar = || {
            ctx.memory(|m| m.area_rect(Id::new("grid_icon_bar")))
                .unwrap()
        };
        for _ in 0..4 {
            frame(&mut v, 1600., vec![]);
        }
        let wide = bar();
        // 9 → 10 images per page (and 34 → 30 pages): the bar keeps its width.
        v.lib.settings.thumbnail_count = 10;
        for _ in 0..3 {
            frame(&mut v, 1600., vec![]);
        }
        assert!(
            (bar().width() - wide.width()).abs() < 0.5,
            "{wide:?} → {:?}",
            bar()
        );
        assert!(!v.grid_overflow);
        // A narrow window: one line, the rest behind » at the right end.
        for _ in 0..4 {
            frame(&mut v, 700., vec![]);
        }
        let narrow = bar();
        assert!(narrow.height() < 70., "one line: {narrow:?}");
        assert!(narrow.width() <= 700. - 20., "fits the window: {narrow:?}");
        let more = Pos2::new(narrow.right() - 12. - 19., narrow.center().y);
        frame(&mut v, 700., vec![egui::Event::PointerMoved(more)]);
        for pressed in [true, false] {
            frame(
                &mut v,
                700.,
                vec![egui::Event::PointerButton {
                    pos: more,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert!(v.grid_overflow, "» opens the rest");
        frame(&mut v, 700., vec![]);
        frame(&mut v, 700., vec![]);
        let rest = ctx
            .memory(|m| m.area_rect(Id::new(("grid_overflow", true))))
            .unwrap();
        assert!(rest.bottom() <= bar().top(), "{rest:?} above {:?}", bar());
        // Wide again: everything fits, nothing to open.
        for _ in 0..3 {
            frame(&mut v, 1600., vec![]);
        }
        assert!(!v.grid_overflow);
    }
    #[test]
    fn text_controls_overflow_in_rows_above_them_and_keep_their_width() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..12).map(|n| format!("{n:02}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        v.grid = true;
        // The text buttons (the icon bar is the default).
        v.lib.settings.grid_bar_icons = false;
        v.lib.settings.grid_round_full = false;
        v.lib.settings.thumbnail_count = 5;
        let frame = |v: &mut Viewer, width: f32, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 1000.))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                v.central(ctx);
                v.panels(ctx);
            });
        };
        // Wide: 3 pages, then 12 pages (one image each): same width.
        for _ in 0..4 {
            frame(&mut v, 2600., vec![]);
        }
        let wide = v.controls_rect.unwrap();
        v.lib.settings.thumbnail_count = 1;
        for _ in 0..3 {
            frame(&mut v, 2600., vec![]);
        }
        let one = v.controls_rect.unwrap();
        assert!(
            (one.width() - wide.width()).abs() < 0.5,
            "{wide:?} → {one:?}"
        );
        // Narrow: open the overflow; it stays low (rows of controls), starts
        // at the controls' left edge and ends above them.
        v.lib.settings.thumbnail_count = 5;
        for _ in 0..4 {
            frame(&mut v, 760., vec![]);
        }
        v.grid_overflow = true;
        for _ in 0..3 {
            frame(&mut v, 760., vec![]);
        }
        let bar = v.controls_rect.unwrap();
        let rest = ctx
            .memory(|m| m.area_rect(Id::new(("grid_overflow", false))))
            .unwrap();
        assert!(rest.height() < 3. * CONTROLS_ROW + 40., "{rest:?}");
        assert!(rest.bottom() <= bar.top(), "{rest:?} above {bar:?}");
        // Ends where the bar (and its » button) ends.
        assert!(
            (rest.right() - bar.right()).abs() < 1.,
            "{rest:?} vs {bar:?}"
        );
        // The tag panel opens above the text controls.
        v.grid_overflow = false;
        v.toggle_grid_tags();
        for _ in 0..3 {
            frame(&mut v, 760., vec![]);
        }
        let panel = ctx
            .memory(|m| m.area_rect(Id::new("action_panel")))
            .unwrap();
        let bar = v.controls_rect.unwrap();
        // Centred on the Tags button (as far as the window allows).
        let tags = v.tags_anchor.unwrap();
        let half = panel.width() / 2.;
        let centre = tags.clamp(half + 8., 760. - half - 8.);
        assert!(
            (panel.center().x - centre).abs() < 2.,
            "{panel:?} vs Tags at {tags}"
        );
        assert!(
            panel.bottom() <= bar.top() && bar.top() - panel.bottom() < 30.,
            "{panel:?} {bar:?}"
        );
    }
    #[test]
    fn the_prompt_window_from_a_tile_menu_stays_and_the_big_images_closes_with_it() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let frame = |v: &mut Viewer| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| v.prompt_window(ctx));
        };
        v.grid = true;
        // From a grid tile's menu: stays open on the grid page.
        v.prompt_window = Some(PathBuf::from("tile.png"));
        v.prompt_follows = false;
        frame(&mut v);
        frame(&mut v);
        assert_eq!(v.prompt_window, Some(PathBuf::from("tile.png")));
        // From the big image's bar: closes when the big image does.
        v.prompt_follows = true;
        v.grid_viewer = Some(PathBuf::from("big.png"));
        frame(&mut v);
        assert!(v.prompt_window.is_some());
        v.grid_viewer = None;
        frame(&mut v);
        assert_eq!(v.prompt_window, None);
    }
    #[test]
    fn the_previous_image_stays_on_screen_while_the_next_loads_when_chosen() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..3).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        let first = v.images.insert_full_texture(&ctx, &v.files[0].clone());
        let draws = |v: &mut Viewer| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                ..Default::default()
            };
            let out = ctx.run(raw, |ctx| v.central(ctx));
            out.shapes.iter().any(|s| match &s.shape {
                egui::Shape::Mesh(m) => m.texture_id == first,
                _ => false,
            })
        };
        for keep in [false, true] {
            v.lib.settings.keep_previous_image = keep;
            v.current = Some(v.files[0].clone());
            assert!(draws(&mut v), "the loaded image is drawn");
            v.current = Some(v.files[1].clone());
            assert_eq!(
                draws(&mut v),
                keep,
                "while the next image loads the previous one is drawn only when chosen"
            );
        }
        // Not once it has left the list (deleted or filtered out).
        v.current = Some(v.files[0].clone());
        draws(&mut v);
        let gone = v.files.remove(0);
        v.rebuild();
        finish_view(&mut v, &ctx);
        v.current = Some(v.files[0].clone());
        assert!(!draws(&mut v), "{gone:?} is no longer in the list");
    }
    #[test]
    fn the_paint_summary_separates_frames_with_uploads() {
        let p = eframe::noirview_diag::PaintStats {
            upload_frames: 2,
            upload_bytes: 3 * 1_048_576,
            upload_ms: 30.,
            upload_max_ms: 20.,
            plain_frames: 4,
            plain_ms: 2.,
        };
        assert_eq!(
            paint_summary(&p),
            "2 uploads 3.0 MB avg 15.0 ms max 20.0 ms · 4 frames avg 0.50 ms"
        );
        assert_eq!(
            paint_summary(&Default::default()),
            "0 uploads 0.0 MB avg 0.0 ms max 0.0 ms · 0 frames avg 0.00 ms"
        );
    }
    #[test]
    fn a_click_on_the_big_image_closes_its_panel_first_and_returning_hides_the_bar() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..3).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[1].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        v.grid = true;
        v.grid_viewer = Some(v.files[1].clone());
        v.bar = true;
        v.panel = Panel::Tags;
        let click = |v: &mut Viewer, at: Pos2| {
            for events in [
                vec![egui::Event::PointerMoved(at)],
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            ] {
                let raw = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                    events,
                    ..Default::default()
                };
                let _ = ctx.run(raw, |ctx| v.central(ctx));
            }
        };
        let image = Pos2::new(300., 300.);
        click(&mut v, image);
        assert!(
            v.panel == Panel::None,
            "the first click closes the tag panel"
        );
        assert!(v.grid_viewer.is_some(), "still on the big image");
        click(&mut v, image);
        assert!(
            v.grid_viewer.is_none() && !v.bar,
            "back to the grid, toolbar hidden"
        );
    }
    #[test]
    fn holding_move_switches_move_selected_mode_directly() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..3).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        v.grid = true;
        v.tag_mode = true;
        v.toggle_move_mode();
        assert!(v.move_mode && !v.tag_mode && !v.grid_normal);
        v.move_selection.insert(v.files[1].clone());
        v.toggle_move_mode();
        assert!(!v.move_mode && v.move_selection.is_empty());
    }
    #[test]
    fn the_recycle_bin_dialog_lists_three_names_and_an_ellipsis_beyond_four() {
        let p = |n: usize| {
            (0..n)
                .map(|i| PathBuf::from(format!("d/{i}.jpg")))
                .collect::<Vec<_>>()
        };
        assert_eq!(delete_dialog_lines(&p(2)), ["0.jpg", "1.jpg"]);
        assert_eq!(
            delete_dialog_lines(&p(4)),
            ["0.jpg", "1.jpg", "2.jpg", "3.jpg"]
        );
        assert_eq!(delete_dialog_lines(&p(9)), ["0.jpg", "1.jpg", "2.jpg", "…"]);
    }
    #[test]
    fn clear_selection_for_the_page_keeps_selections_elsewhere() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let paths: Vec<PathBuf> = (0..6).map(|n| PathBuf::from(format!("{n}.jpg"))).collect();
        v.visible = paths.clone().into();
        v.grid = true;
        v.lib.settings.grid_round_full = false;
        v.lib.settings.thumbnail_count = 3;
        v.delete_selection = [0, 2, 4].iter().map(|i| paths[*i].clone()).collect();
        v.clear_selection(true);
        assert_eq!(v.delete_selection, HashSet::from([paths[4].clone()]));
        v.tag_mode = true;
        v.tag_selection = [1, 5].iter().map(|i| paths[*i].clone()).collect();
        v.clear_selection(false);
        assert!(v.tag_selection.is_empty());
        assert_eq!(
            v.delete_selection.len(),
            1,
            "the other mode's selection stays"
        );
        assert!(
            v.last_selection.is_some(),
            "Restore selection brings it back"
        );
    }
    #[test]
    fn toolbar_keeps_a_place_and_a_lock_per_view_or_one_universal_lock() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..3).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        // Settings from before 0.1.46: the grid takes the old distance.
        let mut old: Settings = serde_json::from_str(r#"{"bar_bottom":60}"#).unwrap();
        old.initialize_after_load();
        assert_eq!(old.bar_bottom_grid, Some(60.));
        v.lib.settings.initialize_after_load();
        v.lib.settings.bar_delay_ms = 0;
        v.lib.settings.bar_bottom = 100.;
        v.lib.settings.bar_bottom_grid = Some(300.);
        let frame = |v: &mut Viewer, at: Pos2| {
            for events in [vec![egui::Event::PointerMoved(at)], vec![]] {
                let raw = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                    events,
                    ..Default::default()
                };
                let _ = ctx.run(raw, |ctx| {
                    v.edges(ctx);
                    v.toolbar(ctx);
                });
            }
        };
        let middle = Pos2::new(640., 300.);
        let bottom_edge = Pos2::new(640., 815.);
        let bar = || ctx.memory(|m| m.area_rect(Id::new("toolbar"))).unwrap();
        // With the text buttons the toolbar is shown in the grid too.
        v.lib.settings.grid_bar_icons = false;
        // Each view has its own distance from the bottom.
        v.bar = true;
        frame(&mut v, middle);
        assert!((820. - bar().bottom() - 100.).abs() < 1., "{:?}", bar());
        v.grid = true;
        frame(&mut v, middle);
        assert!((820. - bar().bottom() - 300.).abs() < 1., "{:?}", bar());
        // A lock in the grid keeps it open there, not in the image view.
        v.set_bar_locked(true);
        assert!(v.lib.settings.bar_locked_grid && !v.lib.settings.bar_locked);
        frame(&mut v, bottom_edge);
        frame(&mut v, middle);
        assert!(
            v.bar,
            "locked in the grid: the bottom edge does not close it"
        );
        v.grid = false;
        frame(&mut v, bottom_edge);
        frame(&mut v, middle);
        assert!(!v.bar, "the image view has no lock");
        // Back in the grid the locked toolbar is open at once.
        v.grid = true;
        frame(&mut v, middle);
        assert!(v.bar);
        // Universal: the image view's lock applies everywhere.
        v.lib.settings.bar_lock_universal = true;
        v.lib.settings.bar_locked_grid = false;
        v.set_bar_locked(true);
        assert!(v.lib.settings.bar_locked);
        v.grid = false;
        v.bar = false;
        frame(&mut v, middle);
        assert!(v.bar, "universal lock opens it in the image view too");
        // It never leaves the screen.
        v.lib.settings.bar_bottom = 5000.;
        frame(&mut v, middle);
        assert!(bar().top() >= 0.);
    }
    #[test]
    fn the_tag_palettes_favorite_chooser_opens_above_its_button() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.panel = Panel::Tags;
        let anchor = Pos2::new(500., 600.);
        v.tag_favorite_popup = Some(anchor);
        let frame = |v: &mut Viewer| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| v.tag_favorite_chooser(ctx));
        };
        frame(&mut v);
        frame(&mut v);
        let rect = ctx
            .memory(|m| m.area_rect(Id::new("tag_favorite_chooser")))
            .unwrap();
        assert!(rect.bottom() <= anchor.y - 5.9, "{rect:?}");
        assert!((rect.center().x - anchor.x).abs() < 1., "{rect:?}");
        assert!(v.panel == Panel::Tags, "the palette stays");
        // Closing the palette closes the chooser.
        v.panel = Panel::None;
        frame(&mut v);
        assert_eq!(v.tag_favorite_popup, None);
    }
    #[test]
    fn close_button_sits_outside_the_trash_circle_and_inside_its_area() {
        for (size, close) in [(40., 16.), (100., 26.), (300., 64.)] {
            let margin = close_margin(close);
            let area = Rect::from_min_size(Pos2::ZERO, Vec2::splat(size + margin * 2.));
            let center = area.center();
            let radius = size / 2.;
            for angle in (0..360).step_by(15) {
                let c = close_center(center, radius, close, angle as f32);
                let gap = (c - center).length() - radius - close / 2.;
                assert!(gap > 0.5, "close button touches the circle at {angle}°");
                assert!(area.contains_rect(Rect::from_center_size(c, Vec2::splat(close))));
            }
        }
    }
    #[test]
    fn hover_activation_is_set_per_area_and_old_settings_keep_their_behaviour() {
        let ctx = egui::Context::default();
        let mut s = Settings {
            hover: false,
            hover_trash: Some(true),
            hover_settings: Some(false),
            ..Default::default()
        };
        let mut h = Hover::default();
        let count = |h: &mut Hover, s: &Settings, settings_window: bool| {
            h.begin(s);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if settings_window {
                        h.area = HoverArea::Settings;
                    }
                    h.watch(ui.button("general or settings"), false);
                    h.watch_in(HoverArea::Trash, ui.button("trash"), true);
                    h.area = HoverArea::General;
                });
            });
            h.responses.len()
        };
        assert_eq!(count(&mut h, &s, false), 1, "only the trash button");
        assert!(h.enabled && h.allows(HoverArea::Trash) && !h.allows(HoverArea::General));
        assert_eq!(count(&mut h, &s, true), 1, "the Settings window is off");
        s.hover_settings = Some(true);
        assert_eq!(count(&mut h, &s, true), 2);
        s.hover_trash = Some(false);
        assert_eq!(count(&mut h, &s, false), 0, "general controls are off");
        s.hover = true;
        assert_eq!(count(&mut h, &s, false), 1);
        s = Settings {
            hover: false,
            hover_trash: Some(false),
            hover_settings: Some(false),
            ..Default::default()
        };
        assert_eq!(count(&mut h, &s, true), 0);
        assert!(!h.enabled);
        // Settings saved before 0.1.45 had one switch for everything.
        for hover in [true, false] {
            let mut old: Settings =
                serde_json::from_str(&format!("{{\"hover\":{hover}}}")).unwrap();
            assert_eq!((old.hover_trash, old.hover_settings), (None, None));
            old.initialize_after_load();
            assert_eq!(old.hover_trash, Some(hover));
            assert_eq!(old.hover_settings, Some(hover));
        }
        let mut new: Settings =
            serde_json::from_str(r#"{"hover":true,"hover_trash":false}"#).unwrap();
        new.initialize_after_load();
        assert_eq!((new.hover_trash(), new.hover_settings()), (false, true));
    }
    #[test]
    fn edges_work_with_the_bar_open_but_not_over_it() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..5).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        let s = &mut v.lib.settings;
        s.edges = true;
        s.side_ms = 0;
        s.top_delete = true;
        s.top_ms = 0;
        s.confirm_delete = true;
        s.side_px = 800.;
        v.bar = true;
        let run = |v: &mut Viewer, at: Pos2| {
            for events in [vec![egui::Event::PointerMoved(at)], vec![]] {
                let raw = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                    events,
                    ..Default::default()
                };
                let _ = ctx.run(raw, |ctx| {
                    v.toolbar(ctx);
                    v.edges(ctx);
                });
                finish_view(v, &ctx);
            }
        };
        let away = Pos2::new(640., 300.);
        run(&mut v, away);
        let bar = ctx.memory(|m| m.area_rect(Id::new("toolbar"))).unwrap();
        // 800 px is limited to 45 % of the window: the middle stays free.
        run(&mut v, Pos2::new(600., 300.));
        assert_eq!(v.current, Some(v.files[0].clone()));
        // Over the bar inside the (wide) left strip: nothing.
        run(&mut v, Pos2::new(bar.left() + 6., bar.center().y));
        assert!(bar.left() + 6. < 1280. * EDGE_MAX_SHARE);
        assert_eq!(v.current, Some(v.files[0].clone()));
        // Left edge with the bar open: previous image, at once (0 ms).
        run(&mut v, Pos2::new(4., 300.));
        assert_eq!(v.current, Some(v.files[4].clone()));
        run(&mut v, away);
        run(&mut v, Pos2::new(1276., 300.));
        assert_eq!(v.current, Some(v.files[0].clone()));
        run(&mut v, away);
        // Top edge with the bar open: the Recycle Bin dialog for the image.
        v.lib.settings.side_px = 24.;
        run(&mut v, Pos2::new(640., 4.));
        assert_eq!(delete_dialog(&v), Some(vec![v.files[0].clone()]));
        v.dialog = None;
        run(&mut v, away);
        // The corner belongs to the side strip while side browsing is on ...
        run(&mut v, Pos2::new(4., 4.));
        assert_eq!(delete_dialog(&v), None);
        assert_eq!(v.current, Some(v.files[4].clone()));
        run(&mut v, away);
        // ... and to the top strip when it is off.
        v.lib.settings.edges = false;
        run(&mut v, Pos2::new(4., 4.));
        assert_eq!(delete_dialog(&v), Some(vec![v.files[4].clone()]));
    }
    #[test]
    fn leaving_a_window_through_the_bottom_opens_the_bar_but_not_in_fullscreen() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..5).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        v.lib.settings.bar_delay_ms = 0;
        let run = |v: &mut Viewer, event: egui::Event| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 820.))),
                events: vec![event],
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| v.edges(ctx));
        };
        // Leaving from the middle of the window does nothing.
        run(&mut v, egui::Event::PointerMoved(Pos2::new(640., 400.)));
        run(&mut v, egui::Event::PointerGone);
        run(&mut v, egui::Event::PointerGone);
        assert!(!v.bar);
        // Overshooting the bottom edge opens the bar once; it stays open.
        run(&mut v, egui::Event::PointerMoved(Pos2::new(640., 790.)));
        run(&mut v, egui::Event::PointerGone);
        run(&mut v, egui::Event::PointerGone);
        assert!(v.bar);
        run(&mut v, egui::Event::PointerGone);
        assert!(v.bar);
        // Fullscreen: the screen edge stops the pointer, leaving does nothing.
        v.bar = false;
        v.fullscreen = true;
        run(&mut v, egui::Event::PointerMoved(Pos2::new(640., 790.)));
        run(&mut v, egui::Event::PointerGone);
        run(&mut v, egui::Event::PointerGone);
        assert!(!v.bar);
    }
    #[test]
    fn big_image_magnifier_waits_for_a_hold_unless_moved_or_set_to_start_at_once() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        let hold = Duration::from_millis(v.lib.settings.grid_hold_ms);
        let at = |ctx: &egui::Context, v: &Viewer, pos: Pos2| {
            let raw = egui::RawInput {
                events: vec![egui::Event::PointerMoved(pos)],
                ..Default::default()
            };
            let mut delay = Duration::MAX;
            let _ = ctx.run(raw, |ctx| delay = v.magnifier_delay(ctx));
            delay
        };
        // The ordinary viewer never waits.
        assert_eq!(at(&ctx, &v, Pos2::new(100., 100.)), Duration::ZERO);
        v.grid_viewer = Some(PathBuf::from("a.jpg"));
        v.viewer_press = Some((Instant::now(), Pos2::new(100., 100.)));
        // Pressed in place: the hold time; moved beyond the click slop: at once.
        assert_eq!(at(&ctx, &v, Pos2::new(103., 100.)), hold);
        assert_eq!(at(&ctx, &v, Pos2::new(130., 100.)), Duration::ZERO);
        v.lib.settings.magnify_on_move = false;
        assert_eq!(at(&ctx, &v, Pos2::new(130., 100.)), hold);
        // Magnify at once: no wait even in place.
        v.lib.settings.magnify_at_once = true;
        assert_eq!(at(&ctx, &v, Pos2::new(100., 100.)), Duration::ZERO);
    }
    #[test]
    fn hover_circle_ring_appears_near_it_and_fades_in() {
        // Inside: full opacity.
        assert_eq!(ring_alpha(-5., 60., true, 1.), Some(255));
        // Beyond the reveal distance: hidden; 0 px = only inside.
        assert_eq!(ring_alpha(61., 60., true, 1.), None);
        assert_eq!(ring_alpha(1., 0., true, 1.), None);
        // Fading in on the way, steadily brighter.
        let far = ring_alpha(50., 60., true, 1.).unwrap_or(0);
        let near = ring_alpha(10., 60., true, 1.).unwrap();
        assert!(far < near && near < 255, "{far} {near}");
        // Without fading: the same faint ring anywhere in range.
        assert_eq!(
            ring_alpha(50., 60., false, 1.),
            ring_alpha(1., 60., false, 1.)
        );
        // Opacity scales everything.
        assert_eq!(ring_alpha(-1., 60., true, 0.5), Some(127));
        // Down to the faintest visible step.
        assert_eq!(ring_alpha(-1., 60., true, 0.004), Some(1));
    }
    #[test]
    fn recently_viewed_keeps_the_last_five_newest_first_without_repeats() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        for name in ["a", "b", "c", "b", "d", "e", "f", "f", "g"] {
            v.record_recent(Path::new(name));
        }
        let names: Vec<_> = v.recent.iter().map(|p| p.to_string_lossy()).collect();
        assert_eq!(names, ["g", "f", "e", "d", "b"]);
    }
    #[test]
    fn changing_history_limit_restarts_a_partial_view_without_losing_current_image() {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.files = (0..5000).map(|n| format!("{n}.jpg").into()).collect();
        v.current = Some(v.files[0].clone());
        v.rebuild();
        finish_view(&mut v, &ctx);
        for _ in 0..100 {
            v.navigate(true);
        }
        assert_eq!(v.nav.history_len(), 101);
        let current = v.current.clone();
        v.refresh_index_filter();
        v.poll_view(&ctx);
        assert!(v.view_pending);
        v.lib.settings.history_limit = 4;
        v.sync_history_limit();
        assert!(v.view_dirty);
        assert!(v.nav.history_len() <= 4);
        finish_view(&mut v, &ctx);
        assert_eq!(v.current, current);
        assert_eq!(v.nav.history_limit(), 4);
        assert!(v.nav.history_len() <= 4);
        v.navigate(false);
        assert_eq!(v.current, Some(v.files[99].clone()));
        v.navigate(true);
        assert_eq!(v.current, current);
    }
    #[test]
    fn history_preference_during_folder_loading_does_not_publish_an_empty_view_or_lose_preferred_image(
    ) {
        let ctx = egui::Context::default();
        let mut v = viewer(&ctx);
        v.lib.settings.history_limit = 24;
        v.loading = true;
        v.view_pending = true;
        v.current = Some("chosen.jpg".into());
        v.sync_history_limit();
        assert!(!v.view_dirty);
        assert!(v.view_build.is_none());
        v.poll_view(&ctx);
        assert!(v.loading && v.view_pending);
        assert_eq!(v.current, Some("chosen.jpg".into()));
        assert_eq!(v.nav.history_limit(), 24);
    }
}
