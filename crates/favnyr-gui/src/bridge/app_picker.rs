//! Application selection is provisional until the user explicitly runs it.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    System,
    Program,
}

#[derive(Clone)]
struct Candidate {
    id: String,
    handler: openwith::AppHandler,
    origin: Origin,
}

pub(super) struct Context {
    ext: String,
    paths: Vec<PathBuf>,
    candidates: Vec<Candidate>,
    items: Vec<OpenerItem>,
    selected: Option<String>,
    launching: bool,
}

impl Context {
    fn new(paths: Vec<PathBuf>) -> Self {
        let ext = paths.first().map(|path| ext_of(path)).unwrap_or_default();
        Self {
            ext,
            paths,
            candidates: Vec::new(),
            items: Vec::new(),
            selected: None,
            launching: false,
        }
    }

    fn replace_system(
        &mut self,
        mut handlers: Vec<openwith::AppHandler>,
        icon: impl Fn(&str) -> Image,
    ) {
        let manual: Vec<_> = self
            .candidates
            .iter()
            .filter(|candidate| candidate.origin == Origin::Program)
            .cloned()
            .collect();
        let cached = std::mem::take(&mut self.items);
        handlers.sort_by(|a, b| {
            b.recommended
                .cmp(&a.recommended)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.candidates = handlers
            .into_iter()
            .map(|handler| Candidate {
                id: format!("system:{}", handler.key),
                handler,
                origin: Origin::System,
            })
            .collect();
        self.candidates.extend(manual);
        self.items = self
            .candidates
            .iter()
            .map(|candidate| OpenerItem {
                id: candidate.id.as_str().into(),
                label: candidate.handler.name.as_str().into(),
                available: true,
                icon: cached
                    .iter()
                    .find(|item| item.id == candidate.id)
                    .map(|item| item.icon.clone())
                    .unwrap_or_else(|| icon(candidate.handler.exe.as_deref().unwrap_or_default())),
                icon_kind: openers::OpenerIcon::None.as_i32(),
            })
            .collect();
    }

    fn visible(&mut self, filter: &str) -> (Vec<OpenerItem>, i32) {
        let visible = filtered_ow_picker_items(&self.items, filter);
        let index = self.selected.as_deref().and_then(|id| {
            visible
                .iter()
                .position(|item| item.available && item.id == id)
        });
        if index.is_none() {
            self.selected = None;
        }
        (visible, index.map_or(-1, |index| index as i32))
    }

    fn select(&mut self, id: &str, filter: &str) -> i32 {
        let (visible, current) = self.visible(filter);
        if self.launching {
            return current;
        }
        let index = visible
            .iter()
            .position(|item| item.available && item.id == id);
        self.selected = index.map(|index| visible[index].id.to_string());
        index.map_or(-1, |index| index as i32)
    }

    fn stage_program(&mut self, program: String, icon: impl Fn(&str) -> Image) {
        if self.launching {
            return;
        }
        // A browsed executable keeps its command route, even when a native
        // association refers to that executable with different launch options.
        if let Some(candidate) = self.candidates.iter().find(|candidate| {
            candidate.origin == Origin::Program
                && candidate
                    .handler
                    .exe
                    .as_deref()
                    .is_some_and(|exe| ops::paths_equal(Path::new(exe), Path::new(&program)))
        }) {
            self.selected = Some(candidate.id.clone());
            return;
        }
        let name = Path::new(&program)
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| program.clone());
        let id = format!("program:{program}");
        self.items.push(OpenerItem {
            id: id.as_str().into(),
            label: name.as_str().into(),
            available: true,
            icon: icon(&program),
            icon_kind: openers::OpenerIcon::None.as_i32(),
        });
        self.candidates.push(Candidate {
            id: id.clone(),
            origin: Origin::Program,
            handler: openwith::AppHandler {
                key: program.clone(),
                name,
                exe: Some(program),
                recommended: false,
            },
        });
        self.selected = Some(id);
    }

    fn prepare(&mut self, filter: &str) -> Option<Candidate> {
        if self.launching || self.paths.is_empty() {
            return None;
        }
        let (_, index) = self.visible(filter);
        if index < 0 {
            return None;
        }
        let candidate = self
            .candidates
            .iter()
            .find(|candidate| Some(&candidate.id) == self.selected.as_ref())?
            .clone();
        self.launching = true;
        Some(candidate)
    }
}

/// Builds an ephemeral command or reuses the saved configuration, without
/// registering a browsed program merely because a native picker returned it.
fn program_opener(store: &openers::OpenerStore, handler: &openwith::AppHandler) -> openers::Opener {
    let program = handler.exe.as_deref().unwrap_or_default();
    if let Some(opener) = store
        .openers
        .iter()
        .find(|opener| opener.program == program)
    {
        return opener.clone();
    }
    let mut temporary = openers::OpenerStore::default();
    let id = temporary.add(&handler.name, program, vec!["{file}".into()]);
    temporary
        .get(&id)
        .cloned()
        .expect("temporary opener was added")
}

fn record_success(
    store: &mut openers::OpenerStore,
    handler: &openwith::AppHandler,
    ext: &str,
    set_default: bool,
) {
    let id = match handler.exe.as_deref() {
        Some(program) => store
            .openers
            .iter()
            .find(|opener| opener.program == program)
            .map(|opener| opener.id.clone())
            .unwrap_or_else(|| store.add(&handler.name, program, vec!["{file}".into()])),
        None => store
            .openers
            .iter()
            .find(|opener| opener.assoc.as_deref() == Some(&handler.key))
            .map(|opener| opener.id.clone())
            .unwrap_or_else(|| store.add_assoc(&handler.name, &handler.key)),
    };
    store.record_use(&id, Some(ext));
    if set_default && !ext.is_empty() {
        store.set_default_ext(&id, ext, true);
    }
}

fn publish(window: &MainWindow, state: &AppState) {
    let filter = window.get_ow_picker_search_text();
    let (items, index) = state
        .ow_pick_ctx
        .borrow_mut()
        .as_mut()
        .map(|context| context.visible(filter.as_str()))
        .unwrap_or((Vec::new(), -1));
    // Invalidate the old index before replacing its model, even if the new
    // selection happens to occupy that same row number.
    window.set_ow_picker_selected_index(-1);
    window.set_ow_picker_handlers(ModelRc::new(VecModel::from(items)));
    window.set_ow_picker_selected_index(index);
}

pub(super) fn install(window: &MainWindow, state: &AppState) {
    let weak = window.as_weak();
    let st = state.clone();
    window.on_ow_open_picker(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if shutdown::stopping(&st) || window.get_ow_picker_busy() {
            return;
        }
        let paths = selected_paths(&st);
        if paths.is_empty() {
            return;
        }
        let mut context = Context::new(paths);
        context.replace_system(openwith::handlers_for_ext(&context.ext, false), opener_icon);
        *st.ow_pick_ctx.borrow_mut() = Some(context);
        window.set_ow_picker_search_text(SharedString::new());
        window.set_ow_picker_show_other_apps(false);
        window.set_ow_picker_set_default(false);
        window.set_ow_picker_busy(false);
        publish(&window, &st);
        window.set_ow_picker_open(true);
    });

    let weak = window.as_weak();
    let st = state.clone();
    window.on_ow_picker_search(move |_| {
        if let Some(window) = weak.upgrade() {
            publish(&window, &st);
        }
    });

    let weak = window.as_weak();
    let st = state.clone();
    window.on_ow_picker_select(move |id| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if !window.get_ow_picker_open() {
            return;
        }
        let index = st
            .ow_pick_ctx
            .borrow_mut()
            .as_mut()
            .map(|context| context.select(id.as_str(), window.get_ow_picker_search_text().as_str()))
            .unwrap_or(-1);
        // Keyboard navigation changes only the highlight, not the list model.
        window.set_ow_picker_selected_index(index);
    });

    let weak = window.as_weak();
    let st = state.clone();
    window.on_ow_picker_show_other_apps_changed(move |show_other| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let ext = st
            .ow_pick_ctx
            .borrow()
            .as_ref()
            .filter(|context| !context.launching)
            .map(|context| context.ext.clone());
        let Some(ext) = ext else {
            return;
        };
        let handlers = openwith::handlers_for_ext(&ext, show_other);
        if let Some(context) = st.ow_pick_ctx.borrow_mut().as_mut() {
            context.replace_system(handlers, opener_icon);
        }
        publish(&window, &st);
    });

    let weak = window.as_weak();
    let st = state.clone();
    window.on_ow_pick_browse(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if !window.get_ow_picker_open() || window.get_ow_picker_busy() {
            return;
        }
        let Some(program) = openwith::browse_for_exe(st.snapshot_config().language) else {
            return;
        };
        if !window.get_ow_picker_open() {
            return;
        }
        if let Some(context) = st.ow_pick_ctx.borrow_mut().as_mut() {
            context.stage_program(program, opener_icon);
        }
        // The explicitly browsed choice must be visible before Run can confirm it.
        window.set_ow_picker_search_text(SharedString::new());
        publish(&window, &st);
    });

    let weak = window.as_weak();
    let st = state.clone();
    window.on_ow_pick(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if !window.get_ow_picker_open() || shutdown::stopping(&st) {
            return;
        }
        let prepared = {
            let mut context = st.ow_pick_ctx.borrow_mut();
            let Some(context) = context.as_mut() else {
                return;
            };
            context
                .prepare(window.get_ow_picker_search_text().as_str())
                .map(|candidate| (candidate, context.ext.clone(), context.paths.clone()))
        };
        let Some((candidate, ext, paths)) = prepared else {
            publish(&window, &st);
            return;
        };
        let set_default = window.get_ow_picker_set_default();
        window.set_ow_picker_busy(true);
        let result = match candidate.origin {
            Origin::System => openwith::launch(&candidate.handler.key, &ext, &paths[0]),
            Origin::Program => {
                let opener = program_opener(&st.openers.borrow(), &candidate.handler);
                archive::run_opener(&window, &st, &opener, &paths)
            }
        };
        match result {
            Ok(()) => {
                record_success(
                    &mut st.openers.borrow_mut(),
                    &candidate.handler,
                    &ext,
                    set_default,
                );
                save_openers(&st);
                push_openers_ui(&window, &st);
                window.set_ow_picker_open(false);
            }
            Err(error) => {
                error!(error = %error, "open-with launch failed");
                if let Some(context) = st.ow_pick_ctx.borrow_mut().as_mut() {
                    context.launching = false;
                }
                window.set_ow_picker_busy(false);
                notice(
                    &window,
                    i18n::tr(st.snapshot_config().language, "ow_run_failed")
                        .replace("{name}", &candidate.handler.name),
                    NoticeKind::Error,
                );
            }
        }
    });

    let weak = window.as_weak();
    let st = state.clone();
    window.on_ow_picker_closed(move || {
        *st.ow_pick_ctx.borrow_mut() = None;
        if let Some(window) = weak.upgrade() {
            window.set_ow_picker_selected_index(-1);
            window.set_ow_picker_handlers(ModelRc::default());
            window.set_ow_picker_busy(false);
        }
    });
}

#[cfg(test)]
mod tests;
