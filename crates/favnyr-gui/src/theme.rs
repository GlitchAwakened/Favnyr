//! Theme discovery and application, with no work in item-rendering paths.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use favnyr_core::theme::{
    self as themes, BuiltinTheme, ColorRole, Entry, Palette, Scheme, ThemePalettes,
};
use favnyr_core::{Config, Lang, Theme as Preference, paths};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::{MainWindow, Theme, i18n};

#[derive(Clone, PartialEq, Eq)]
struct Choice {
    preference: Preference,
    id: Option<String>,
}

/// Which of the two choices a load is for.
///
/// Keeping them apart is what stops a workspace's theme from being written to
/// the configuration: only the application's own choice is ever persisted, and
/// an identifier that reached the screen through a workspace must not become
/// the default everywhere else.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    /// The application's own choice, the one the settings show. `persist`
    /// tells a selection the user just made, which is saved, from the first
    /// read of a choice already on disk.
    Own { persist: bool },
    /// The theme the loaded workspace imposes, stored in its own file.
    Imposed,
}

struct State {
    choice: Choice,
    /// Theme the loaded workspace imposes, `None` to follow `choice`. Held
    /// separately because it is painted without ever being persisted, and
    /// because taking it away has to find the way back to `choice`.
    imposed: Option<String>,
    entries: Vec<Entry>,
    /// The palettes of the painted theme file, both appearances, so changing
    /// the appearance never has to read the file again. `None` means the
    /// painted theme is a built-in family, named by [`State::painted`].
    active: Option<ThemePalettes>,
    unavailable: bool,
    invalid_files: usize,
    generation: u64,
    pending_load: Option<Level>,
    refresh_pending: bool,
}

impl State {
    /// The theme actually on screen: the one the loaded workspace imposes,
    /// and otherwise the application's own.
    fn painted(&self) -> Option<String> {
        self.imposed.clone().or_else(|| self.choice.id.clone())
    }
}

enum Task {
    Discover(Option<String>),
    Load(String, Level),
    Folder,
}

enum ResultKind {
    Catalog(themes::Catalog),
    Loaded(String, Level, Result<ThemePalettes, String>),
    Folder(Result<(), String>),
}

type Deliveries = Arc<Mutex<Vec<(u64, ResultKind)>>>;
type Persist = Rc<dyn Fn(Preference, Option<String>)>;
type Notice = Rc<dyn Fn(&MainWindow, String)>;

struct Controller {
    state: RefCell<State>,
    deliveries: Deliveries,
    persist: Persist,
    notice: Notice,
    translate: fn(Lang, &str) -> String,
}

fn language(window: &MainWindow) -> Lang {
    Lang::all()
        .get(window.get_language_index().max(0) as usize)
        .copied()
        .unwrap_or_default()
}

/// Keep a selector's model identity when only its status or palette changes.
fn update_labels(
    current: ModelRc<SharedString>,
    values: Vec<SharedString>,
    set: impl FnOnce(ModelRc<SharedString>),
) {
    if current.row_count() != values.len() || current.iter().ne(values.iter().cloned()) {
        set(ModelRc::new(VecModel::from(values)));
    }
}

fn index(preference: Preference) -> i32 {
    match preference {
        Preference::Auto => 0,
        Preference::Light => 1,
        Preference::Dark => 2,
    }
}

/// The appearance a preference resolves to, Auto following the desktop.
fn scheme(window: &MainWindow, preference: Preference) -> Scheme {
    let dark = match preference {
        Preference::Auto => window.get_system_dark(),
        Preference::Dark => true,
        Preference::Light => false,
    };
    if dark { Scheme::Dark } else { Scheme::Light }
}

fn apply(window: &MainWindow, palette: &Palette, automatic: bool, custom: bool) {
    window.set_theme_scheme(if automatic {
        0
    } else {
        index(palette.scheme.preference())
    });
    let colors = window.global::<Theme>();
    colors.set_custom_palette(custom);
    colors.set_hidden_opacity(palette.hidden_opacity);
    colors.set_age_opacity(palette.age_opacity);
    fn color(value: themes::Rgba) -> slint::Color {
        let [r, g, b, a] = value.0;
        slint::Color::from_argb_u8(a, r, g, b)
    }
    colors.set_background(color(palette.color(ColorRole::Background)));
    colors.set_surface(color(palette.color(ColorRole::Surface)));
    colors.set_toolbar(color(palette.color(ColorRole::Toolbar)));
    colors.set_alternate_background(color(palette.color(ColorRole::AlternateBackground)));
    colors.set_foreground(color(palette.color(ColorRole::Foreground)));
    colors.set_border(color(palette.color(ColorRole::Border)));
    colors.set_accent_background(color(palette.color(ColorRole::AccentBackground)));
    colors.set_accent_foreground(color(palette.color(ColorRole::AccentForeground)));
    colors.set_danger(color(palette.color(ColorRole::Danger)));
    colors.set_danger_foreground(color(palette.color(ColorRole::DangerForeground)));
    colors.set_success(color(palette.color(ColorRole::Success)));
    colors.set_warning(color(palette.color(ColorRole::Warning)));
    colors.set_extension(color(palette.color(ColorRole::Extension)));
    colors.set_track(color(palette.color(ColorRole::Track)));
    colors.set_disk_normal(color(palette.color(ColorRole::DiskNormal)));
    colors.set_disk_warning(color(palette.color(ColorRole::DiskWarning)));
    colors.set_disk_critical(color(palette.color(ColorRole::DiskCritical)));
    colors.set_disk_normal_foreground(color(palette.color(ColorRole::DiskNormalForeground)));
    colors.set_disk_warning_foreground(color(palette.color(ColorRole::DiskWarningForeground)));
    colors.set_disk_critical_foreground(color(palette.color(ColorRole::DiskCriticalForeground)));
    colors.set_toast_success(color(palette.color(ColorRole::ToastSuccess)));
    colors.set_toast_info(color(palette.color(ColorRole::ToastInfo)));
    colors.set_toast_danger(color(palette.color(ColorRole::ToastDanger)));
    colors.set_tooltip_background(color(palette.color(ColorRole::TooltipBackground)));
    colors.set_tooltip_foreground(color(palette.color(ColorRole::TooltipForeground)));
    colors.set_shadow(color(palette.color(ColorRole::Shadow)));
    colors.set_modal_overlay(color(palette.color(ColorRole::ModalOverlay)));
    colors.set_file_application(color(palette.color(ColorRole::FileApplication)));
    colors.set_file_archive(color(palette.color(ColorRole::FileArchive)));
    colors.set_file_audio(color(palette.color(ColorRole::FileAudio)));
    colors.set_file_document(color(palette.color(ColorRole::FileDocument)));
    colors.set_file_image(color(palette.color(ColorRole::FileImage)));
    colors.set_file_video(color(palette.color(ColorRole::FileVideo)));
    colors.set_file_registry(color(palette.color(ColorRole::FileRegistry)));
    colors.set_folder_default(color(palette.color(ColorRole::FolderDefault)));
    colors.set_folder_red(color(palette.color(ColorRole::FolderRed)));
    colors.set_folder_green(color(palette.color(ColorRole::FolderGreen)));
    colors.set_folder_teal(color(palette.color(ColorRole::FolderTeal)));
    colors.set_folder_blue(color(palette.color(ColorRole::FolderBlue)));
    colors.set_folder_violet(color(palette.color(ColorRole::FolderViolet)));
    colors.set_folder_pink(color(palette.color(ColorRole::FolderPink)));
    colors.set_folder_slate(color(palette.color(ColorRole::FolderSlate)));
    colors.set_age_hour(color(palette.color(ColorRole::AgeHour)));
    colors.set_age_hours(color(palette.color(ColorRole::AgeHours)));
    colors.set_age_day(color(palette.color(ColorRole::AgeDay)));
    colors.set_age_week(color(palette.color(ColorRole::AgeWeek)));
    colors.set_age_month(color(palette.color(ColorRole::AgeMonth)));
    colors.set_age_year(color(palette.color(ColorRole::AgeYear)));
    colors.set_age_older(color(palette.color(ColorRole::AgeOlder)));
    colors.set_folder_red_foreground(color(palette.color(ColorRole::FolderRedForeground)));
    colors.set_folder_green_foreground(color(palette.color(ColorRole::FolderGreenForeground)));
    colors.set_folder_teal_foreground(color(palette.color(ColorRole::FolderTealForeground)));
    colors.set_folder_blue_foreground(color(palette.color(ColorRole::FolderBlueForeground)));
    colors.set_folder_violet_foreground(color(palette.color(ColorRole::FolderVioletForeground)));
    colors.set_folder_pink_foreground(color(palette.color(ColorRole::FolderPinkForeground)));
    colors.set_folder_slate_foreground(color(palette.color(ColorRole::FolderSlateForeground)));
}

impl Controller {
    fn new(
        config: &Config,
        persist: Persist,
        notice: Notice,
        translate: fn(Lang, &str) -> String,
    ) -> Self {
        Self {
            state: RefCell::new(State {
                choice: Choice {
                    preference: config.theme,
                    id: config.theme_id.clone(),
                },
                imposed: None,
                entries: Vec::new(),
                active: None,
                unavailable: false,
                invalid_files: 0,
                generation: 0,
                pending_load: None,
                refresh_pending: false,
            }),
            deliveries: Arc::new(Mutex::new(Vec::new())),
            persist,
            notice,
            translate,
        }
    }

    /// Paints the built-in family of the painted choice. This is also the
    /// recovery path for a theme file that is missing or unreadable, where
    /// the choice names no family and the base one is used.
    fn paint_builtin(&self, window: &MainWindow) {
        let (preference, family) = {
            let state = self.state.borrow();
            (
                state.choice.preference,
                BuiltinTheme::from_id(state.painted().as_deref()).unwrap_or_default(),
            )
        };
        apply(
            window,
            &family.palette(scheme(window, preference)),
            preference == Preference::Auto,
            family != BuiltinTheme::Nord,
        );
    }

    /// Paints the current choice in its current appearance, taking it from the
    /// loaded theme file when there is one. Theme files and built-in families
    /// carry the same two appearances, so neither needs a path of its own
    /// beyond where the palettes come from.
    fn paint(&self, window: &MainWindow) {
        let (preference, palettes) = {
            let state = self.state.borrow();
            (state.choice.preference, state.active.clone())
        };
        match palettes {
            Some(palettes) => apply(
                window,
                palettes.variant(scheme(window, preference)),
                preference == Preference::Auto,
                true,
            ),
            None => self.paint_builtin(window),
        }
    }

    fn labels(&self, window: &MainWindow) {
        let lang = language(window);
        let appearance_labels: Vec<SharedString> = ["theme_auto", "theme_light", "theme_dark"]
            .iter()
            .map(|key| (self.translate)(lang, key).into())
            .collect();
        let (labels, ids, selected, preference, reloadable, status) = {
            let mut state = self.state.borrow_mut();
            if let Some(id) = state.choice.id.clone()
                && BuiltinTheme::from_id(Some(&id)).is_none()
                && !state.entries.iter().any(|e| e.id == id)
            {
                let name = themes::user_file_name(&id)
                    .and_then(|n| n.strip_suffix(".toml"))
                    .map(str::to_owned)
                    .unwrap_or_else(|| (self.translate)(lang, "theme_unavailable_name"));
                state.entries.push(Entry { id, name });
            }
            let mut labels: Vec<SharedString> = BuiltinTheme::ALL
                .iter()
                .map(|family| (self.translate)(lang, family.label_key()).into())
                .collect();
            labels.extend(
                state
                    .entries
                    .iter()
                    .map(|e| SharedString::from(e.name.as_str())),
            );
            // Identifiers in the same order as the labels, so a selector that
            // stores its answer somewhere other than the configuration can
            // name a theme instead of pointing at a row whose position moves
            // whenever the themes directory is scanned again.
            let mut ids: Vec<SharedString> = BuiltinTheme::ALL
                .iter()
                .map(|family| family.stored_id().into())
                .collect();
            ids.extend(
                state
                    .entries
                    .iter()
                    .map(|e| SharedString::from(e.id.as_str())),
            );
            let family = BuiltinTheme::from_id(state.choice.id.as_deref());
            let selected = family
                .and_then(|family| BuiltinTheme::ALL.iter().position(|entry| *entry == family))
                .or_else(|| {
                    state
                        .choice
                        .id
                        .as_ref()
                        .and_then(|id| state.entries.iter().position(|e| &e.id == id))
                        .map(|i| i + BuiltinTheme::ALL.len())
                })
                .unwrap_or(0) as i32;
            // A workspace imposing a theme makes the selector above look
            // broken: the choice is saved and applies everywhere else, but it
            // is not what is on screen. Saying so here puts the explanation
            // directly under the control the user just moved.
            let overriding = window.get_theme_imposed_by();
            let status = if state.unavailable {
                (self.translate)(lang, "theme_unavailable")
            } else if state.invalid_files != 0 {
                (self.translate)(lang, "theme_invalid_files")
                    .replace("{count}", &state.invalid_files.to_string())
            } else if state.imposed.is_some() && !overriding.is_empty() {
                (self.translate)(lang, "theme_workspace_override")
                    .replace("{name}", &overriding)
                    .replace("{theme}", &window.get_theme_imposed_label())
            } else {
                String::new()
            };
            // The button reads again the file of the theme on screen, which is
            // the one being adjusted, and a usable file name is exactly what
            // says there is a file to read.
            let reloadable = state
                .painted()
                .as_deref()
                .and_then(themes::user_file_name)
                .is_some();
            (
                labels,
                ids,
                selected,
                state.choice.preference,
                reloadable,
                status,
            )
        };
        update_labels(window.get_theme_ids(), ids, |model| {
            window.set_theme_ids(model)
        });
        update_labels(window.get_theme_labels(), labels, |model| {
            window.set_theme_labels(model)
        });
        window.set_theme_index(selected);
        update_labels(
            window.get_theme_appearance_labels(),
            appearance_labels,
            |model| window.set_theme_appearance_labels(model),
        );
        window.set_theme_appearance_index(index(preference));
        window.set_theme_user_file(reloadable);
        window.set_theme_status(status.into());
    }

    fn begin_task(&self, window: &MainWindow, task: &Task) -> u64 {
        let mut state = self.state.borrow_mut();
        if !matches!(task, Task::Folder) {
            state.generation = state.generation.wrapping_add(1);
            state.pending_load = match task {
                Task::Load(_, level) => Some(*level),
                _ => None,
            };
        }
        let generation = state.generation;
        let loading = state.pending_load.is_some();
        drop(state);
        window.set_theme_loading(loading);
        generation
    }

    fn invalidate_load(&self, window: &MainWindow) {
        let mut state = self.state.borrow_mut();
        state.generation = state.generation.wrapping_add(1);
        state.pending_load = None;
        drop(state);
        window.set_theme_loading(false);
    }

    fn submit(&self, window: &MainWindow, task: Task) {
        let folder = matches!(&task, Task::Folder);
        // Only applying a theme blocks the controls. Listing the directory
        // must not, because the selector raising this flag would disable, and
        // therefore close, the very list whose opening asked for the scan. A
        // result that arrives after the user has moved on is dropped by the
        // generation check either way.
        let blocking = matches!(&task, Task::Load(..));
        let generation = self.begin_task(window, &task);
        let deliveries = self.deliveries.clone();
        let weak = window.as_weak();
        if let Err(error) = std::thread::Builder::new()
            .name("theme-reader".into())
            .spawn(move || {
                let dir = paths::themes_dir();
                let result = match task {
                    Task::Discover(id) => {
                        ResultKind::Catalog(themes::discover(&dir, id.as_deref()))
                    }
                    Task::Load(id, level) => {
                        let result = themes::load(&dir, &id);
                        ResultKind::Loaded(id, level, result)
                    }
                    Task::Folder => {
                        #[cfg(windows)]
                        let _apartment = crate::winutil::ComApartment::sta();
                        let result = std::fs::create_dir_all(&dir)
                            .map_err(|e| e.to_string())
                            .and_then(|_| {
                                crate::actions::open_path(&dir)
                                    .map(|_| ())
                                    .map_err(|e| e.to_string())
                            });
                        ResultKind::Folder(result)
                    }
                };
                deliveries
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push((generation, result));
                let _ = weak.upgrade_in_event_loop(|w| w.invoke_theme_results_ready());
            })
        {
            tracing::warn!(%error, "starting theme reader failed");
            if blocking {
                self.state.borrow_mut().pending_load = None;
                window.set_theme_loading(false);
            }
            (self.notice)(
                window,
                (self.translate)(
                    language(window),
                    if folder {
                        "theme_folder_failed"
                    } else {
                        "theme_failed"
                    },
                ),
            );
        }
    }

    fn received(&self, window: &MainWindow) {
        let results =
            std::mem::take(&mut *self.deliveries.lock().unwrap_or_else(|p| p.into_inner()));
        for (generation, result) in results {
            if let ResultKind::Folder(result) = result {
                if let Err(error) = result {
                    tracing::warn!(%error, "opening themes directory failed");
                    (self.notice)(
                        window,
                        (self.translate)(language(window), "theme_folder_failed"),
                    );
                }
                continue;
            }
            if generation != self.state.borrow().generation {
                continue;
            }
            window.set_theme_loading(false);
            self.state.borrow_mut().pending_load = None;
            match result {
                ResultKind::Catalog(catalog) => {
                    for error in &catalog.errors {
                        tracing::warn!(%error, "theme discovery ignored an entry");
                    }
                    {
                        let mut state = self.state.borrow_mut();
                        state.entries = catalog.entries;
                        state.invalid_files = catalog.errors.len();
                    }
                    if let Some(result) = catalog.selected {
                        let (id, level) = self.painted_level();
                        self.loaded(window, id.unwrap_or_default(), level, result);
                    }
                }
                ResultKind::Loaded(id, level, result) => self.loaded(window, id, level, result),
                ResultKind::Folder(_) => unreachable!(),
            }
            self.labels(window);
        }
        let refresh = {
            let mut state = self.state.borrow_mut();
            if !window.get_theme_loading() && state.refresh_pending {
                state.refresh_pending = false;
                true
            } else {
                false
            }
        };
        if refresh {
            let (id, _) = self.painted_level();
            self.submit(window, Task::Discover(id));
        }
    }

    fn loaded(
        &self,
        window: &MainWindow,
        id: String,
        level: Level,
        result: Result<ThemePalettes, String>,
    ) {
        match result {
            Ok(palettes) => {
                // The appearance stays the user's, as with a built-in family:
                // a theme file carries both, so it imposes neither.
                let (painted, own) = {
                    let mut state = self.state.borrow_mut();
                    match level {
                        Level::Own { persist } => {
                            state.choice.id = Some(id.clone());
                            // A workspace imposing a theme stays on screen:
                            // the application's own choice is recorded, not
                            // painted over what the workspace asked for.
                            let painted = state.imposed.is_none();
                            if painted {
                                state.active = Some(palettes);
                                state.unavailable = false;
                            }
                            let own = persist.then(|| (state.choice.preference, Some(id.clone())));
                            (painted, own)
                        }
                        // What a workspace imposes is kept in its own file, so
                        // the configuration must never learn this identifier:
                        // writing it there would promote one workspace's
                        // choice to the application's default.
                        Level::Imposed => {
                            state.imposed = Some(id);
                            state.active = Some(palettes);
                            state.unavailable = false;
                            (true, None)
                        }
                    }
                };
                if painted {
                    self.paint(window);
                }
                if let Some((preference, id)) = own {
                    (self.persist)(preference, id);
                }
            }
            Err(error) => {
                tracing::warn!(%id, %error, "theme could not be loaded");
                let (unavailable, fall_back) = {
                    let mut state = self.state.borrow_mut();
                    match level {
                        // The workspace keeps the identifier it stores:
                        // erasing it would destroy the only trace of what the
                        // user asked for, should the file come back. Only the
                        // painting falls back, onto the application's own
                        // choice rather than onto the base family, which is
                        // both shorter and truer.
                        Level::Imposed => {
                            state.imposed = None;
                            state.unavailable = true;
                            (true, true)
                        }
                        Level::Own { persist } => {
                            let current = !persist || state.choice.id.as_ref() == Some(&id);
                            if current {
                                state.unavailable = true;
                            }
                            (current, false)
                        }
                    }
                };
                if fall_back {
                    self.restore_own(window);
                }
                if unavailable {
                    (self.notice)(
                        window,
                        (self.translate)(language(window), "theme_unavailable"),
                    );
                } else {
                    (self.notice)(window, (self.translate)(language(window), "theme_failed"));
                }
            }
        }
    }

    /// The theme on screen and the choice it came from, for a read that
    /// refreshes what is already displayed rather than choosing something new.
    fn painted_level(&self) -> (Option<String>, Level) {
        let state = self.state.borrow();
        match &state.imposed {
            Some(id) => (Some(id.clone()), Level::Imposed),
            None => (state.choice.id.clone(), Level::Own { persist: false }),
        }
    }

    /// Paints the application's own choice, reading its file again when it has
    /// one. This is the way back from a workspace override, whether the user
    /// took it away or its file could not be read.
    fn restore_own(&self, window: &MainWindow) {
        let own = {
            let mut state = self.state.borrow_mut();
            state.imposed = None;
            state.active = None;
            state.choice.id.clone()
        };
        self.paint_builtin(window);
        match own.filter(|id| themes::user_file_name(id).is_some()) {
            Some(id) => self.submit(window, Task::Load(id, Level::Own { persist: false })),
            None => window.set_theme_loading(false),
        }
    }

    /// Imposes the theme a freshly loaded workspace carries, or takes the
    /// imposition away with `None`. Nothing is written to the configuration
    /// and nothing is written to the workspace: this only decides what is on
    /// screen, from what the workspace already stores.
    fn set_imposed(&self, window: &MainWindow, id: Option<&str>) {
        // A pending workspace load must be invalidated even when the requested
        // theme is already painted: its result belongs to the previous choice.
        let (unchanged, pending_imposed) = {
            let state = self.state.borrow();
            (
                state.imposed.as_deref() == id,
                state.pending_load == Some(Level::Imposed),
            )
        };
        if unchanged {
            if pending_imposed {
                self.invalidate_load(window);
            }
            return;
        }
        self.invalidate_load(window);
        match id {
            Some(id) if themes::user_file_name(id).is_some() => {
                self.submit(window, Task::Load(id.to_owned(), Level::Imposed));
            }
            Some(id) => {
                {
                    let mut state = self.state.borrow_mut();
                    state.imposed = Some(id.to_owned());
                    // A built-in family needs no reading: it is parsed once
                    // and cached, so it is painted on the spot.
                    state.active = None;
                    state.unavailable = false;
                }
                self.paint_builtin(window);
                self.labels(window);
            }
            None => {
                self.state.borrow_mut().unavailable = false;
                self.restore_own(window);
                self.labels(window);
            }
        }
    }

    fn activate_builtin(&self, window: &MainWindow, family: BuiltinTheme, preference: Preference) {
        let id = family.id().map(str::to_owned);
        self.invalidate_load(window);
        {
            let mut state = self.state.borrow_mut();
            state.choice = Choice {
                preference,
                id: id.clone(),
            };
            // The palettes in memory belong to whatever is painted. A
            // workspace imposing a theme keeps painting it, so they are only
            // dropped when the application's own choice is the one on screen.
            if state.imposed.is_none() {
                state.active = None;
                state.unavailable = false;
            }
        }
        self.paint(window);
        (self.persist)(preference, id);
        self.labels(window);
    }

    fn select_appearance(&self, window: &MainWindow, selected: i32) {
        let Some(&preference) = Preference::all().get(selected as usize) else {
            self.labels(window);
            return;
        };
        // The appearance belongs to the application and never to a workspace:
        // every theme carries both of them, so a workspace imposing a family
        // is already right under either one. Switching therefore keeps what is
        // on screen and only changes the appearance it is served in, taking
        // the other palette from the file already in memory with no reread.
        let id = {
            let mut state = self.state.borrow_mut();
            state.choice.preference = preference;
            state.choice.id.clone()
        };
        self.paint(window);
        (self.persist)(preference, id);
        self.labels(window);
    }

    /// Reads the file of the theme on screen again and applies what it now
    /// contains. Nothing is persisted, since no choice has changed.
    fn reload(&self, window: &MainWindow) {
        let (id, level) = self.painted_level();
        if let Some(id) = id.filter(|id| themes::user_file_name(id).is_some()) {
            self.submit(window, Task::Load(id, level));
        }
    }

    fn select(&self, window: &MainWindow, selected: i32) {
        if let Some(&family) = BuiltinTheme::ALL.get(selected as usize) {
            let preference = self.state.borrow().choice.preference;
            self.activate_builtin(window, family, preference);
        } else {
            let id = (selected as usize)
                .checked_sub(BuiltinTheme::ALL.len())
                .and_then(|i| self.state.borrow().entries.get(i).map(|e| e.id.clone()));
            self.labels(window);
            if let Some(id) = id {
                self.submit(window, Task::Load(id, Level::Own { persist: true }));
            }
        }
    }
}

pub fn install(
    window: &MainWindow,
    config: &Config,
    persist: impl Fn(Preference, Option<String>) + 'static,
    notice: impl Fn(&MainWindow, String) + 'static,
) {
    let controller = Rc::new(Controller::new(
        config,
        Rc::new(persist),
        Rc::new(notice),
        i18n::tr,
    ));
    connect(window, &controller);
    controller.paint_builtin(window);
    controller.labels(window);
    if let Some(id) = config.theme_id.clone()
        && BuiltinTheme::from_id(Some(&id)).is_none()
    {
        controller.submit(window, Task::Load(id, Level::Own { persist: false }));
    }
}

fn connect(window: &MainWindow, controller: &Rc<Controller>) {
    let weak = window.as_weak();
    let c = controller.clone();
    window.on_theme_changed(move |selected| {
        if let Some(w) = weak.upgrade() {
            c.select(&w, selected);
        }
    });
    let weak = window.as_weak();
    let c = controller.clone();
    window.on_theme_appearance_changed(move |selected| {
        if let Some(w) = weak.upgrade() {
            c.select_appearance(&w, selected);
        }
    });
    let weak = window.as_weak();
    let c = controller.clone();
    window.on_refresh_themes(move || {
        if let Some(w) = weak.upgrade() {
            if w.get_theme_loading() {
                c.state.borrow_mut().refresh_pending = true;
                return;
            }
            let (id, _) = c.painted_level();
            c.submit(&w, Task::Discover(id));
        }
    });
    let weak = window.as_weak();
    let c = controller.clone();
    window.on_apply_workspace_theme(move |id| {
        if let Some(w) = weak.upgrade() {
            let id = id.to_string();
            c.set_imposed(&w, (!id.is_empty()).then_some(id.as_str()));
        }
    });
    let weak = window.as_weak();
    let c = controller.clone();
    window.on_reload_theme(move || {
        if let Some(w) = weak.upgrade() {
            c.reload(&w);
        }
    });
    let weak = window.as_weak();
    let c = controller.clone();
    window.on_refresh_theme_labels(move || {
        if let Some(w) = weak.upgrade() {
            c.labels(&w);
        }
    });
    let weak = window.as_weak();
    let c = controller.clone();
    window.on_theme_results_ready(move || {
        if let Some(w) = weak.upgrade() {
            c.received(&w);
        }
    });
    let weak = window.as_weak();
    let c = controller.clone();
    window.on_system_theme_changed(move || {
        // Auto follows the desktop whatever is selected, a theme file
        // included, since every theme carries both appearances.
        let automatic = c.state.borrow().choice.preference == Preference::Auto;
        if automatic && let Some(w) = weak.upgrade() {
            c.paint(&w);
        }
    });
    let weak = window.as_weak();
    let c = controller.clone();
    window.on_open_themes_folder(move || {
        if let Some(w) = weak.upgrade() {
            c.submit(&w, Task::Folder);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    fn test_tr(lang: Lang, key: &str) -> String {
        let source = if lang == Lang::Fr {
            include_str!("../i18n/fr.toml")
        } else {
            include_str!("../i18n/en.toml")
        };
        let catalog: toml::Table = toml::from_str(source).unwrap();
        catalog
            .get(key)
            .and_then(toml::Value::as_str)
            .unwrap_or(key)
            .to_owned()
    }

    fn fixture(config: &Config) -> (Controller, Rc<RefCell<Config>>) {
        let saved = Rc::new(RefCell::new(config.clone()));
        let output = saved.clone();
        let controller = Controller::new(
            config,
            Rc::new(move |preference, id| {
                let mut config = output.borrow_mut();
                config.theme = preference;
                config.theme_id = id;
            }),
            Rc::new(|_, _| {}),
            test_tr,
        );
        (controller, saved)
    }

    fn test_palettes(family: BuiltinTheme) -> ThemePalettes {
        ThemePalettes {
            light: family.palette(Scheme::Light),
            dark: family.palette(Scheme::Dark),
        }
    }

    #[test]
    fn extended_builtin_selection_persists_and_follows_appearance_changes() {
        crate::test_support::run(|| {
            for family in BuiltinTheme::ALL
                .iter()
                .copied()
                .filter(|family| *family != BuiltinTheme::Nord)
            {
                let ui = MainWindow::new().unwrap();
                let (controller, saved) = fixture(&Config::default());
                let controller = Rc::new(controller);
                connect(&ui, &controller);
                controller.paint_builtin(&ui);
                controller.labels(&ui);
                let selected = BuiltinTheme::ALL
                    .iter()
                    .position(|entry| *entry == family)
                    .unwrap() as i32;
                ui.invoke_theme_changed(selected);
                assert_eq!(saved.borrow().theme_id.as_deref(), family.id());
                assert!(!ui.get_theme_loading());
                assert!(ui.global::<Theme>().get_custom_palette());
                assert_eq!(
                    ui.get_theme_labels().row_data(selected as usize).unwrap(),
                    test_tr(Lang::En, family.label_key())
                );
                let assert_palette = |scheme| {
                    let palette = family.palette(scheme);
                    let [r, g, b, a] = palette.color(ColorRole::Background).0;
                    assert_eq!(
                        ui.global::<Theme>().get_background(),
                        slint::Color::from_argb_u8(a, r, g, b)
                    );
                };
                for (appearance, scheme) in [(1, Scheme::Light), (2, Scheme::Dark)] {
                    ui.invoke_theme_appearance_changed(appearance);
                    assert_palette(scheme);
                    assert_eq!(ui.get_theme_index(), selected);
                    let serialized = toml::to_string(&*saved.borrow()).unwrap();
                    let restored: Config = toml::from_str(&serialized).unwrap();
                    let (restored, _) = fixture(&restored);
                    restored.paint_builtin(&ui);
                    restored.labels(&ui);
                    assert_palette(scheme);
                    assert_eq!(ui.get_theme_index(), selected);
                    assert_eq!(ui.get_theme_appearance_index(), appearance);
                }
                ui.invoke_theme_appearance_changed(0);
                ui.invoke_system_theme_changed();
                assert_palette(if ui.get_system_dark() {
                    Scheme::Dark
                } else {
                    Scheme::Light
                });
                assert_eq!(saved.borrow().theme, Preference::Auto);
                assert_eq!(saved.borrow().theme_id.as_deref(), family.id());
                ui.invoke_theme_changed(0);
                assert!(!ui.global::<Theme>().get_custom_palette());
                assert!(saved.borrow().theme_id.is_none());
                assert_eq!(saved.borrow().theme, Preference::Auto);
            }
        });
    }

    #[test]
    fn separate_selectors_preserve_appearance_and_existing_configuration() {
        crate::test_support::run(|| {
            for theme in Preference::all() {
                let config = Config {
                    theme: *theme,
                    clock_utc: true,
                    ..Config::default()
                };
                let saved = Rc::new(RefCell::new(config.clone()));
                let output = saved.clone();
                let ui = MainWindow::new().unwrap();
                let controller = Rc::new(Controller::new(
                    &config,
                    Rc::new(move |preference, id| {
                        let mut config = output.borrow_mut();
                        config.theme = preference;
                        config.theme_id = id;
                    }),
                    Rc::new(|_, _| panic!("unexpected theme failure")),
                    test_tr,
                ));
                connect(&ui, &controller);
                controller.paint_builtin(&ui);
                controller.labels(&ui);
                assert_eq!(ui.get_theme_labels().row_count(), BuiltinTheme::ALL.len());
                assert_eq!(
                    ui.get_theme_labels().row_data(0).unwrap(),
                    test_tr(Lang::En, "theme_nord")
                );
                assert_eq!(ui.get_theme_appearance_labels().row_count(), 3);
                assert_eq!(ui.get_theme_appearance_index(), index(*theme));
                assert_eq!(ui.get_theme_index(), 0);
                assert!(!ui.get_theme_loading());
                ui.invoke_theme_appearance_changed(1);
                assert_eq!(ui.get_theme_scheme(), 1);
                assert_eq!(saved.borrow().theme, Preference::Light);
                ui.invoke_theme_changed(0);
                assert_eq!(saved.borrow().theme, Preference::Light);
                assert!(saved.borrow().theme_id.is_none());
                assert!(saved.borrow().clock_utc);
                ui.set_language_index(
                    Lang::all()
                        .iter()
                        .position(|lang| *lang == Lang::Fr)
                        .unwrap() as i32,
                );
                ui.invoke_refresh_theme_labels();
                assert_eq!(
                    ui.get_theme_labels().row_data(0).unwrap(),
                    test_tr(Lang::Fr, "theme_nord")
                );
                assert_eq!(
                    ui.get_theme_appearance_labels().row_data(1).unwrap(),
                    test_tr(Lang::Fr, "theme_light")
                );
                assert_eq!(ui.get_theme_index(), 0);
                assert_eq!(ui.get_theme_appearance_index(), 1);
                ui.invoke_theme_appearance_changed(0);
                ui.invoke_system_theme_changed();
                assert_eq!(saved.borrow().theme, Preference::Auto);
                assert_eq!(ui.get_theme_scheme(), 0);
            }
        });
    }

    #[test]
    fn a_theme_file_answers_both_appearances_and_can_return_to_a_builtin() {
        crate::test_support::run(|| {
            let ui = MainWindow::new().unwrap();
            let (controller, saved) = fixture(&Config::default());
            let controller = Rc::new(controller);
            connect(&ui, &controller);
            let palettes = themes::parse(
                "schema_version = 1\n[light.colors]\nsurface = '#FAFAFA'\n[dark.colors]\nsurface = '#203040'",
            )
            .unwrap();
            controller.state.borrow_mut().entries.push(Entry {
                id: "user:my_theme.toml".into(),
                name: "my_theme".into(),
            });
            controller.loaded(
                &ui,
                "user:my_theme.toml".into(),
                Level::Own { persist: true },
                Ok(palettes),
            );
            controller.labels(&ui);
            let expected = |dark: bool| {
                if dark {
                    slint::Color::from_rgb_u8(0x20, 0x30, 0x40)
                } else {
                    slint::Color::from_rgb_u8(0xFA, 0xFA, 0xFA)
                }
            };
            assert_eq!(ui.get_theme_index(), BuiltinTheme::ALL.len() as i32);
            assert!(ui.global::<Theme>().get_custom_palette());
            // There is a file behind this choice, so the reload button shows.
            assert!(ui.get_theme_user_file());
            assert_eq!(
                saved.borrow().theme_id.as_deref(),
                Some("user:my_theme.toml")
            );
            // A theme file imposes no appearance: the default stays Auto, and
            // Auto follows the desktop.
            assert_eq!(saved.borrow().theme, Preference::Auto);
            assert_eq!(ui.get_theme_appearance_index(), 0);
            assert_eq!(ui.get_theme_scheme(), 0);
            assert_eq!(
                ui.global::<Theme>().get_surface(),
                expected(ui.get_system_dark())
            );
            // Both appearances come from the same file, and the choice between
            // them is stored like any other.
            for (appearance, dark) in [(1, false), (2, true)] {
                ui.invoke_theme_appearance_changed(appearance);
                assert_eq!(ui.get_theme_appearance_index(), appearance);
                assert_eq!(ui.get_theme_scheme(), appearance);
                assert_eq!(ui.global::<Theme>().get_surface(), expected(dark));
                assert_eq!(ui.get_theme_index(), BuiltinTheme::ALL.len() as i32);
                assert_eq!(
                    saved.borrow().theme_id.as_deref(),
                    Some("user:my_theme.toml")
                );
            }
            assert_eq!(saved.borrow().theme, Preference::Dark);
            let (restored, _) = fixture(&saved.borrow());
            restored.labels(&ui);
            assert_eq!(ui.get_theme_index(), BuiltinTheme::ALL.len() as i32);
            ui.invoke_theme_appearance_changed(0);
            ui.invoke_system_theme_changed();
            assert_eq!(saved.borrow().theme, Preference::Auto);
            assert_eq!(
                ui.global::<Theme>().get_surface(),
                expected(ui.get_system_dark())
            );
            controller.select(&ui, 0);
            assert_eq!(ui.get_theme_index(), 0);
            assert!(!ui.global::<Theme>().get_custom_palette());
            assert!(saved.borrow().theme_id.is_none());
            assert_eq!(saved.borrow().theme, Preference::Auto);
            // A built-in family has no file, so the button is gone and asking
            // for a reload anyway starts no reader at all. Reloading a real
            // selection is not exercised here: it would read the themes
            // directory of whoever runs the tests.
            assert!(!ui.get_theme_user_file());
            controller.reload(&ui);
            assert!(!ui.get_theme_loading());
        });
    }

    /// A theme a workspace imposes is painted but never saved as the
    /// application's default. The trap is that the settings keep persisting
    /// their own choice while something else is on screen: an appearance
    /// change writes the theme identifier alongside the appearance, so the
    /// two levels have to stay apart all the way down to that write.
    #[test]
    fn a_workspace_theme_is_painted_without_becoming_the_default() {
        crate::test_support::run(|| {
            let ui = MainWindow::new().unwrap();
            let (controller, saved) = fixture(&Config::default());
            let controller = Rc::new(controller);
            connect(&ui, &controller);
            controller.paint_builtin(&ui);
            controller.labels(&ui);
            let background = |family: BuiltinTheme, scheme| {
                let [r, g, b, a] = family.palette(scheme).color(ColorRole::Background).0;
                slint::Color::from_argb_u8(a, r, g, b)
            };
            let painted = || ui.global::<Theme>().get_background();

            // The application's own choice is a theme file, saved as usual.
            let own = themes::parse(
                "schema_version = 1\n[light.colors]\nbackground = '#FAFAFA'\n[dark.colors]\nbackground = '#203040'",
            )
            .unwrap();
            controller.state.borrow_mut().entries.push(Entry {
                id: "user:theme01.toml".into(),
                name: "theme01".into(),
            });
            controller.loaded(
                &ui,
                "user:theme01.toml".into(),
                Level::Own { persist: true },
                Ok(own),
            );
            ui.invoke_theme_appearance_changed(1);
            assert_eq!(painted(), slint::Color::from_rgb_u8(0xFA, 0xFA, 0xFA));
            assert_eq!(
                saved.borrow().theme_id.as_deref(),
                Some("user:theme01.toml")
            );

            // A workspace imposes a built-in family. It reaches the screen at
            // once, no reading being needed, and the configuration is left
            // naming the user's own file.
            let imposed = BuiltinTheme::GruvboxHard;
            let imposed_label = test_tr(Lang::En, imposed.label_key());
            ui.set_theme_imposed_by("workspace01".into());
            ui.set_theme_imposed_label(imposed_label.as_str().into());
            ui.invoke_apply_workspace_theme(imposed.stored_id().into());
            assert_eq!(painted(), background(imposed, Scheme::Light));
            assert_eq!(
                saved.borrow().theme_id.as_deref(),
                Some("user:theme01.toml")
            );
            // The settings keep showing, and offering to reload, the user's
            // own choice, and say why it is not what is on screen.
            assert_eq!(ui.get_theme_index(), BuiltinTheme::ALL.len() as i32);
            // The sentence names the workspace and the theme it imposes, both
            // supplied by the bridge, which knows them.
            let expected = test_tr(Lang::En, "theme_workspace_override")
                .replace("{name}", "workspace01")
                .replace("{theme}", &imposed_label);
            assert_eq!(ui.get_theme_status(), expected);
            assert!(expected.contains("workspace01") && expected.contains(&imposed_label));

            // The regression guarded here: the appearance belongs to the
            // application, so changing it repaints what the workspace imposes
            // and still saves the user's own theme, never the imposed one.
            ui.invoke_theme_appearance_changed(2);
            assert_eq!(painted(), background(imposed, Scheme::Dark));
            assert_eq!(saved.borrow().theme, Preference::Dark);
            assert_eq!(
                saved.borrow().theme_id.as_deref(),
                Some("user:theme01.toml")
            );

            // Choosing another own theme while a workspace imposes one records
            // the choice without taking the screen from the workspace.
            controller.select(&ui, 0);
            assert!(saved.borrow().theme_id.is_none());
            assert_eq!(ui.get_theme_index(), 0);
            assert_eq!(painted(), background(imposed, Scheme::Dark));

            // Taking the imposition away hands the screen back to that choice.
            ui.set_theme_imposed_by("".into());
            ui.set_theme_imposed_label("".into());
            ui.invoke_apply_workspace_theme("".into());
            assert_eq!(painted(), background(BuiltinTheme::Nord, Scheme::Dark));
            assert_eq!(ui.get_theme_status(), "");
            assert!(!ui.get_theme_user_file());

            // Loading a workspace that imposes nothing, while nothing is
            // imposed, must not repaint: it is what every ordinary load does.
            ui.global::<Theme>()
                .set_background(slint::Color::from_rgb_u8(1, 2, 3));
            ui.invoke_apply_workspace_theme("".into());
            assert_eq!(painted(), slint::Color::from_rgb_u8(1, 2, 3));
        });
    }

    /// A workspace naming a theme file that has since been deleted falls back
    /// to the application's own choice rather than to the base family, and
    /// keeps its stored identifier: erasing it would destroy the only trace of
    /// what the user asked for, should the file come back.
    #[test]
    fn an_unreadable_imposed_theme_falls_back_to_the_own_choice() {
        crate::test_support::run(|| {
            let ui = MainWindow::new().unwrap();
            let config = Config {
                theme: Preference::Dark,
                theme_id: Some(BuiltinTheme::TokyoNight.stored_id().into()),
                ..Config::default()
            };
            let (controller, saved) = fixture(&config);
            let controller = Rc::new(controller);
            connect(&ui, &controller);
            controller.paint_builtin(&ui);
            controller.labels(&ui);
            let expected = {
                let [r, g, b, a] = BuiltinTheme::TokyoNight
                    .palette(Scheme::Dark)
                    .color(ColorRole::Background)
                    .0;
                slint::Color::from_argb_u8(a, r, g, b)
            };
            assert_eq!(ui.global::<Theme>().get_background(), expected);

            // Delivered the way the reader thread delivers, so the whole
            // receiving path runs: the status line is written by the relabel
            // that follows, and nothing else writes it.
            controller.deliveries.lock().unwrap().push((
                0,
                ResultKind::Loaded(
                    "user:theme01.toml".into(),
                    Level::Imposed,
                    Err("no such file".into()),
                ),
            ));
            controller.received(&ui);
            assert_eq!(ui.global::<Theme>().get_background(), expected);
            assert_eq!(
                ui.get_theme_status(),
                test_tr(Lang::En, "theme_unavailable")
            );
            // Nothing of this reached the configuration.
            assert_eq!(
                saved.borrow().theme_id.as_deref(),
                Some(BuiltinTheme::TokyoNight.stored_id())
            );
            assert_eq!(saved.borrow().theme, Preference::Dark);
        });
    }

    // Opening the theme list scans the folder, so a scan that finds nothing
    // new has to be invisible: handing Slint a new model would rebuild the
    // list that is open under the pointer.
    #[test]
    fn scanning_an_unchanged_themes_folder_leaves_the_list_untouched() {
        crate::test_support::run(|| {
            let ui = MainWindow::new().unwrap();
            let (controller, _) = fixture(&Config::default());
            let entry = |name: &str| Entry {
                id: format!("user:{name}.toml"),
                name: name.to_owned(),
            };
            let deliver = |entries: Vec<Entry>| {
                controller.deliveries.lock().unwrap().push((
                    0,
                    ResultKind::Catalog(themes::Catalog {
                        entries,
                        ..Default::default()
                    }),
                ));
                controller.received(&ui);
            };
            deliver(vec![entry("theme01")]);
            assert_eq!(
                ui.get_theme_labels().row_count(),
                BuiltinTheme::ALL.len() + 1
            );
            let before = ui.get_theme_labels();
            let ids = ui.get_theme_ids();
            let appearances = ui.get_theme_appearance_labels();
            deliver(vec![entry("theme01")]);
            assert!(ui.get_theme_labels() == before);
            assert!(ui.get_theme_ids() == ids);
            assert!(ui.get_theme_appearance_labels() == appearances);
            // A new file does reach the list, under its own name.
            deliver(vec![entry("theme01"), entry("theme02")]);
            assert_eq!(ui.get_theme_status(), "");
            let labels = ui.get_theme_labels();
            assert!(labels != before);
            assert_eq!(labels.row_count(), BuiltinTheme::ALL.len() + 2);
            assert_eq!(
                labels.row_data(BuiltinTheme::ALL.len() + 1).unwrap(),
                "theme02"
            );
        });
    }

    #[test]
    fn leaving_a_pending_workspace_theme_discards_its_success_or_failure() {
        crate::test_support::run(|| {
            for previous in [None, Some(BuiltinTheme::Atom.stored_id())] {
                for target in [None, previous] {
                    for succeeds in [false, true] {
                        let ui = MainWindow::new().unwrap();
                        let config = Config {
                            theme: Preference::Dark,
                            theme_id: Some(BuiltinTheme::TokyoNight.stored_id().into()),
                            ..Config::default()
                        };
                        let (controller, saved) = fixture(&config);
                        controller.paint_builtin(&ui);
                        controller.set_imposed(&ui, previous);
                        let generation = controller.begin_task(
                            &ui,
                            &Task::Load("user:theme01.toml".into(), Level::Imposed),
                        );
                        assert!(ui.get_theme_loading());
                        controller.set_imposed(&ui, target);
                        assert!(!ui.get_theme_loading());
                        assert!(controller.state.borrow().pending_load.is_none());
                        let expected = ui.global::<Theme>().get_background();
                        let result = if succeeds {
                            Ok(test_palettes(BuiltinTheme::GruvboxHard))
                        } else {
                            Err("unavailable theme file".into())
                        };
                        controller.deliveries.lock().unwrap().push((
                            generation,
                            ResultKind::Loaded("user:theme01.toml".into(), Level::Imposed, result),
                        ));
                        controller.received(&ui);
                        assert_eq!(controller.state.borrow().imposed.as_deref(), target);
                        assert_eq!(ui.global::<Theme>().get_background(), expected);
                        assert!(!controller.state.borrow().unavailable);
                        assert_eq!(saved.borrow().theme_id, config.theme_id);
                    }
                }
            }
        });
    }

    #[test]
    fn an_unimposed_workspace_keeps_the_pending_global_theme_load() {
        crate::test_support::run(|| {
            let ui = MainWindow::new().unwrap();
            let config = Config {
                theme_id: Some("user:theme01.toml".into()),
                ..Config::default()
            };
            let (controller, saved) = fixture(&config);
            let level = Level::Own { persist: false };
            let generation =
                controller.begin_task(&ui, &Task::Load("user:theme01.toml".into(), level));
            controller.set_imposed(&ui, None);
            assert!(ui.get_theme_loading());
            assert_eq!(controller.state.borrow().generation, generation);
            controller.deliveries.lock().unwrap().push((
                generation,
                ResultKind::Loaded(
                    "user:theme01.toml".into(),
                    level,
                    Ok(test_palettes(BuiltinTheme::Atom)),
                ),
            ));
            controller.received(&ui);
            assert!(!ui.get_theme_loading());
            assert!(controller.state.borrow().active.is_some());
            assert_eq!(saved.borrow().theme_id, config.theme_id);
        });
    }

    #[test]
    fn scanning_a_selected_file_updates_its_palette_without_rebuilding_selectors() {
        crate::test_support::run(|| {
            let ui = MainWindow::new().unwrap();
            let config = Config {
                theme: Preference::Dark,
                theme_id: Some("user:theme01.toml".into()),
                ..Config::default()
            };
            let (controller, saved) = fixture(&config);
            let deliver = |family: BuiltinTheme, errors: Vec<String>| {
                controller.deliveries.lock().unwrap().push((
                    0,
                    ResultKind::Catalog(themes::Catalog {
                        entries: vec![Entry {
                            id: "user:theme01.toml".into(),
                            name: "theme01".into(),
                        }],
                        errors,
                        selected: Some(Ok(test_palettes(family))),
                    }),
                ));
                controller.received(&ui);
            };
            deliver(BuiltinTheme::Atom, Vec::new());
            let labels = ui.get_theme_labels();
            let ids = ui.get_theme_ids();
            let appearances = ui.get_theme_appearance_labels();
            let before = ui.global::<Theme>().get_background();
            deliver(BuiltinTheme::GruvboxHard, vec!["invalid theme file".into()]);
            assert!(ui.get_theme_labels() == labels);
            assert!(ui.get_theme_ids() == ids);
            assert!(ui.get_theme_appearance_labels() == appearances);
            assert_ne!(ui.global::<Theme>().get_background(), before);
            assert!(!ui.get_theme_status().is_empty());
            assert_eq!(saved.borrow().theme_id, config.theme_id);
        });
    }

    #[test]
    fn stale_theme_results_cannot_replace_a_new_builtin_selection() {
        crate::test_support::run(|| {
            let ui = MainWindow::new().unwrap();
            let config = Config {
                theme_id: Some("user:my_theme.toml".into()),
                ..Config::default()
            };
            let (controller, saved) = fixture(&config);
            controller.labels(&ui);
            assert_eq!(ui.get_theme_index(), BuiltinTheme::ALL.len() as i32);
            controller.select(&ui, 0);
            controller.deliveries.lock().unwrap().push((
                0,
                ResultKind::Loaded(
                    "user:my_theme.toml".into(),
                    Level::Own { persist: true },
                    themes::parse("schema_version = 1\n[light]\n[dark]"),
                ),
            ));
            controller.received(&ui);
            assert_eq!(ui.get_theme_index(), 0);
            assert!(!ui.global::<Theme>().get_custom_palette());
            assert!(saved.borrow().theme_id.is_none());
            let alias = Config {
                theme: Preference::Light,
                theme_id: Some("builtin:base".into()),
                ..Config::default()
            };
            let (controller, _) = fixture(&alias);
            controller.paint_builtin(&ui);
            controller.labels(&ui);
            assert_eq!(ui.get_theme_index(), 0);
            assert_eq!(ui.get_theme_appearance_index(), 1);
            assert_eq!(ui.get_theme_labels().row_count(), BuiltinTheme::ALL.len());
        });
    }

    #[test]
    fn palettes_update_live_without_leaking_between_windows() {
        crate::test_support::run(|| {
            let first = MainWindow::new().unwrap();
            let second = MainWindow::new().unwrap();
            apply(&second, &themes::builtin(Scheme::Light), false, false);
            let untouched = second.global::<Theme>().get_background();
            let custom = themes::parse(
                "schema_version = 1\n[light]\n[dark.colors]\nsurface = \"#203040\"\naccent_background = \"#101010\"",
            ).unwrap();
            apply(&first, &custom.dark, false, true);
            assert!(first.global::<Theme>().get_custom_palette());
            assert_eq!(
                first.global::<Theme>().get_surface(),
                slint::Color::from_rgb_u8(0x20, 0x30, 0x40)
            );
            assert_eq!(
                first.global::<Theme>().get_accent_foreground(),
                slint::Color::from_rgb_u8(255, 255, 255)
            );
            assert_eq!(second.global::<Theme>().get_background(), untouched);
            assert_eq!(first.get_theme_scheme(), 2);
            apply(&first, &themes::builtin(Scheme::Light), true, false);
            assert!(!first.global::<Theme>().get_custom_palette());
            assert_eq!(first.get_theme_scheme(), 0);
            assert_eq!(first.global::<Theme>().get_background(), untouched);
        });
    }
}
