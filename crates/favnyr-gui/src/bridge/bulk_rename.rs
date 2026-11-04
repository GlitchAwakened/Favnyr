//! Frozen selections, asynchronous previews and delivery of batch rename results.

use super::*;
use crate::{BulkRenameInputs, BulkRenameOptions, BulkRenameRow};
use favnyr_core::bulk_rename as engine;

struct PreviewDelivery {
    generation: u64,
    plan: Option<engine::Plan>,
}

#[derive(Default)]
pub(super) struct State {
    sources: RefCell<Vec<PathBuf>>,
    plan: RefCell<Option<engine::Plan>>,
    generation: Arc<AtomicU64>,
    requested: Cell<u64>,
    due: Cell<Option<Instant>>,
    worker: Arc<AtomicBool>,
    delivery: Arc<Mutex<Option<PreviewDelivery>>>,
    operation: Cell<Option<i32>>,
}

pub(super) fn install(window: &MainWindow, state: &AppState) {
    let st = state.clone();
    let weak = window.as_weak();
    window.on_bulk_edited(move || {
        if let Some(w) = weak.upgrade() {
            edited(&w, &st);
        }
    });
    let st = state.clone();
    let weak = window.as_weak();
    window.on_bulk_poll(move || {
        if let Some(w) = weak.upgrade() {
            poll(&w, &st);
        }
    });
    let st = state.clone();
    let weak = window.as_weak();
    window.on_bulk_submit(move || {
        if let Some(w) = weak.upgrade() {
            submit(&w, &st);
        }
    });
    let st = state.clone();
    let weak = window.as_weak();
    window.on_bulk_cancel(move || {
        let Some(w) = weak.upgrade() else { return };
        if let Some(id) = st.bulk_rename.operation.get() {
            st.ops.cancel(id);
        } else {
            dismiss_dialog(&w, &st);
        }
    });
}

fn dismiss_dialog(window: &MainWindow, state: &AppState) {
    state.bulk_rename.generation.fetch_add(1, Ordering::Relaxed);
    state.bulk_rename.sources.borrow_mut().clear();
    state.bulk_rename.plan.borrow_mut().take();
    window.set_bulk_open(false);
    window.set_bulk_rows(ModelRc::default());
}

pub(super) fn open(window: &MainWindow, state: &AppState, sources: Vec<PathBuf>) {
    if shutdown::stopping(state) || window.get_bulk_open() {
        return;
    }
    if state.ops.in_flight() {
        show_notice(
            window,
            i18n::tr(state.config.borrow().language, "bulk_busy"),
        );
        return;
    }
    *state.bulk_rename.sources.borrow_mut() = sources;
    window.global::<BulkRenameInputs>().invoke_reset();
    window.set_bulk_rows(ModelRc::default());
    window.set_bulk_running(false);
    window.set_bulk_finished(false);
    window.set_bulk_open(true);
    edited(window, state);
}

fn edited(window: &MainWindow, state: &AppState) {
    if window.get_bulk_running() || window.get_bulk_finished() {
        return;
    }
    state.bulk_rename.generation.fetch_add(1, Ordering::Relaxed);
    state
        .bulk_rename
        .due
        .set(Some(Instant::now() + Duration::from_millis(150)));
    state.bulk_rename.plan.borrow_mut().take();
    window.set_bulk_ready(false);
    window.set_bulk_summary(i18n::tr(state.config.borrow().language, "bulk_preparing").into());
}

fn snapshot_options(window: &MainWindow) -> BulkRenameOptions {
    let inputs = window.global::<BulkRenameInputs>();
    BulkRenameOptions {
        common_enabled: inputs.get_common_enabled(),
        common_name: inputs.get_common_name(),
        replace_enabled: inputs.get_replace_enabled(),
        find: inputs.get_find(),
        replacement: inputs.get_replacement(),
        case_sensitive: inputs.get_case_sensitive(),
        replace_all: inputs.get_replace_all(),
        affixes_enabled: inputs.get_affixes_enabled(),
        prefix: inputs.get_prefix(),
        suffix: inputs.get_suffix(),
        numbering_enabled: inputs.get_numbering_enabled(),
        start: inputs.get_start(),
        increment: inputs.get_increment(),
        digits: inputs.get_digits(),
        position: inputs.get_position(),
        separator: inputs.get_separator(),
        case_mode: inputs.get_case_mode(),
        extension_enabled: inputs.get_extension_enabled(),
        extension: inputs.get_extension(),
    }
}

fn options(value: BulkRenameOptions) -> engine::Options {
    engine::Options {
        common: value.common_enabled.then(|| value.common_name.to_string()),
        replacement: value.replace_enabled.then(|| engine::Replacement {
            find: value.find.to_string(),
            with: value.replacement.to_string(),
            case_sensitive: value.case_sensitive,
            all: value.replace_all,
        }),
        affixes: value
            .affixes_enabled
            .then(|| (value.prefix.to_string(), value.suffix.to_string())),
        numbering: value.numbering_enabled.then(|| engine::Numbering {
            start: value.start.max(0) as u64,
            step: value.increment.max(1) as u64,
            digits: value.digits.clamp(1, 12) as usize,
            before: value.position == 0,
            separator: value.separator.to_string(),
        }),
        case: match value.case_mode {
            1 => engine::Case::Lower,
            2 => engine::Case::Upper,
            _ => engine::Case::Unchanged,
        },
        extension: value.extension_enabled.then(|| value.extension.to_string()),
    }
}

fn poll(window: &MainWindow, state: &AppState) {
    let session = &state.bulk_rename;
    if !window.get_bulk_open() || window.get_bulk_running() || window.get_bulk_finished() {
        return;
    }
    let result = session
        .delivery
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    if let Some(PreviewDelivery {
        generation,
        plan: Some(plan),
    }) = result
        && generation == session.generation.load(Ordering::Relaxed)
    {
        let lang = state.config.borrow().language;
        let rows: Vec<_> = plan
            .entries
            .iter()
            .map(|entry| preview_row(entry, lang))
            .collect();
        let errors = plan
            .entries
            .iter()
            .filter(|entry| entry.issue.is_some())
            .count();
        window.set_bulk_rows(Rc::new(VecModel::from(rows)).into());
        window.set_bulk_summary(
            i18n::tr(lang, "bulk_summary")
                .replace("{changed}", &plan.changes().to_string())
                .replace(
                    "{unchanged}",
                    &(plan.entries.len() - plan.changes()).to_string(),
                )
                .replace("{errors}", &errors.to_string())
                .into(),
        );
        window.set_bulk_ready(plan.valid());
        *session.plan.borrow_mut() = Some(plan);
    }
    let generation = session.generation.load(Ordering::Relaxed);
    if session.requested.get() == generation
        || session.worker.load(Ordering::Acquire)
        || session.due.get().is_some_and(|due| Instant::now() < due)
    {
        return;
    }
    let sources = session.sources.borrow().clone();
    let options = options(snapshot_options(window));
    let current = session.generation.clone();
    let delivery = session.delivery.clone();
    let worker = session.worker.clone();
    session.requested.set(generation);
    session.worker.store(true, Ordering::Release);
    // Previews touch metadata only. One worker coalesces edits, including edits
    // made while an unavailable network directory delays a metadata query.
    if let Err(error) = std::thread::Builder::new()
        .name("rename-preview".into())
        .spawn(move || {
            let plan = engine::preview(&sources, &options, || {
                current.load(Ordering::Relaxed) != generation
            });
            *delivery.lock().unwrap_or_else(|p| p.into_inner()) =
                Some(PreviewDelivery { generation, plan });
            worker.store(false, Ordering::Release);
        })
    {
        session.worker.store(false, Ordering::Release);
        error!(%error, "could not start batch rename preview");
        window.set_bulk_summary(
            i18n::file_error_reason(state.config.borrow().language, Some(error.kind())).into(),
        );
    }
}

fn issue_text(issue: engine::Issue, lang: Lang) -> String {
    use engine::Issue;
    let key = match issue {
        Issue::InvalidName(reason) => match reason {
            ops::NameRejection::Empty => "bulk_invalid_options",
            ops::NameRejection::DotEntry => "name_error_dot_entry",
            ops::NameRejection::ReservedDevice => "name_error_reserved",
            ops::NameRejection::TrailingDotOrSpace => "name_error_trailing",
            ops::NameRejection::ForbiddenChar => "name_error_chars",
        },
        Issue::InvalidOptions => "bulk_invalid_options",
        Issue::InvalidEncoding => "bulk_invalid_encoding",
        Issue::Unavailable(kind) => {
            return format!(
                "{} {}",
                i18n::tr(lang, "bulk_unavailable"),
                i18n::file_error_reason(lang, Some(kind))
            );
        }
        Issue::Duplicate => "bulk_duplicate",
        Issue::Occupied => "paste_conflict_taken",
        Issue::NestedSelection => "bulk_nested",
        Issue::Changed => "bulk_changed",
    };
    i18n::tr(lang, key).replace("{chars}", ops::FORBIDDEN_NAME_CHARS)
}

fn file_name(path: &Path) -> SharedString {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .as_ref()
        .into()
}

fn preview_row(entry: &engine::Entry, lang: Lang) -> BulkRenameRow {
    BulkRenameRow {
        original: file_name(&entry.source),
        proposed: file_name(&entry.target),
        parent_path: entry
            .source
            .parent()
            .unwrap_or(Path::new(""))
            .to_string_lossy()
            .as_ref()
            .into(),
        status: entry
            .issue
            .map(|issue| issue_text(issue, lang))
            .unwrap_or_else(|| {
                if entry.changes() {
                    String::new()
                } else {
                    i18n::tr(lang, "bulk_unchanged")
                }
            })
            .into(),
        error: entry.issue.is_some(),
    }
}

fn submit(window: &MainWindow, state: &AppState) {
    if shutdown::stopping(state) || !window.get_bulk_ready() || window.get_bulk_running() {
        return;
    }
    let lang = state.config.borrow().language;
    if state.ops.in_flight() {
        window.set_bulk_summary(i18n::tr(lang, "bulk_busy").into());
        return;
    }
    let Some(plan) = state
        .bulk_rename
        .plan
        .borrow()
        .clone()
        .filter(|plan| plan.valid())
    else {
        return;
    };
    let targets: Vec<_> = plan
        .entries
        .iter()
        .flat_map(|entry| [entry.source.clone(), entry.target.clone()])
        .collect();
    let cancel = Arc::new(AtomicBool::new(false));
    let op_id = state.ops.register(OpHandle {
        cancel: Some(cancel.clone()),
        pending_focus: None,
        thumbnail_invalidations: targets.clone(),
        targets,
        transient_cleanup: None,
        manual_insertion: None,
        completed_targets: Arc::default(),
    });
    state.bulk_rename.operation.set(Some(op_id));
    window.set_bulk_running(true);
    window.set_bulk_ready(false);
    sync_op_busy(window, state);
    let weak = window.as_weak();
    let deliveries = state.op_deliveries.clone();
    let title = i18n::tr(lang, "bulk_title");
    window.invoke_op_progress(OpProgress {
        id: op_id,
        state: OP_RUNNING,
        title: title.clone().into(),
        ..Default::default()
    });
    let lease = crate::background_work::acquire();
    if let Err(error) = std::thread::Builder::new()
        .name("batch-rename".into())
        .spawn(move || {
            let _lease = lease;
            let mut last = Instant::now();
            let outcome = engine::execute(
                &plan,
                || cancel.load(Ordering::Relaxed),
                |done, total| {
                    if last.elapsed() >= Duration::from_millis(80) {
                        last = Instant::now();
                        push_op(
                            &weak,
                            op_id,
                            OP_RUNNING,
                            true,
                            done as f32 / total.max(1) as f32,
                            title.clone(),
                            String::new(),
                            format!("{} %", done * 100 / total.max(1)),
                        );
                    }
                },
            );
            deliver_op_event(
                &weak,
                &deliveries,
                OpDelivery::BulkRenamed {
                    op_id,
                    plan,
                    outcome,
                    lang,
                },
            );
        })
    {
        error!(%error, "could not start batch rename");
        state.bulk_rename.operation.set(None);
        window.set_bulk_running(false);
        window.set_bulk_ready(true);
        window.set_bulk_summary(i18n::file_error_reason(lang, Some(error.kind())).into());
        window.invoke_op_dismiss(op_id);
        window.invoke_op_finished(op_id);
    }
}

pub(super) fn complete(
    window: &MainWindow,
    state: &AppState,
    op_id: i32,
    plan: engine::Plan,
    outcome: engine::Outcome,
    lang: Lang,
) {
    if state.bulk_rename.operation.get() != Some(op_id) {
        return;
    }
    let failed = outcome.failure.is_some() || outcome.cancelled;
    let needs_review =
        outcome.failure.is_some() || (outcome.cancelled && !outcome.locations.is_empty());
    if !outcome.steps.is_empty() {
        invalidate_content(state, &plan, &outcome);
    }
    let locations: HashMap<_, _> = outcome
        .locations
        .iter()
        .map(|(source, current)| (reservation_key(source), current.clone()))
        .collect();
    if !outcome.locations.is_empty() {
        track_renamed_paths(window, state, &outcome.steps);
        // Preserve selected identities through the subsequent re-listing,
        // including rows inside a renamed directory.
        for panel in state.panels.borrow().iter() {
            for index in 0..panel.rows_model.row_count() {
                let Some(mut row) = panel.rows_model.row_data(index) else {
                    continue;
                };
                let old = row_path(&row);
                let path = final_location(&old, &locations);
                if path != old {
                    row.name = file_name(&path);
                    row.path = path
                        .parent()
                        .unwrap_or(Path::new(""))
                        .to_string_lossy()
                        .as_ref()
                        .into();
                    panel.rows_model.set_row_data(index, row);
                }
            }
        }
    }
    let diagnostic = if let Some(failure) = &outcome.failure {
        error!(path = %failure.path.display(), kind = ?failure.kind, "batch rename failed");
        let reason = failure
            .issue
            .map(|issue| issue_text(issue, lang))
            .unwrap_or_else(|| i18n::file_error_reason(lang, Some(failure.kind)));
        i18n::rename_failed(lang, &failure.path.to_string_lossy(), &reason)
    } else if outcome.cancelled {
        i18n::tr(lang, "bulk_cancelled")
    } else {
        i18n::tr(lang, "bulk_done").replace("{count}", &plan.changes().to_string())
    };
    let detail = if failed {
        format!(
            "{diagnostic}\n{}",
            i18n::tr(
                lang,
                if outcome.steps.is_empty() {
                    "bulk_not_changed"
                } else if outcome.locations.is_empty() {
                    "bulk_restored"
                } else {
                    "bulk_recovery"
                }
            )
        )
    } else {
        diagnostic.clone()
    };
    window.set_bulk_running(false);
    state.bulk_rename.operation.set(None);
    if needs_review {
        let rows = plan
            .entries
            .iter()
            .map(|entry| {
                let current = locations
                    .get(&reservation_key(&entry.source))
                    .unwrap_or(&entry.source);
                let failure_here = outcome.failure.as_ref().is_some_and(|failure| {
                    failure.path == entry.source || failure.path == entry.target
                });
                BulkRenameRow {
                    original: file_name(&entry.source),
                    proposed: file_name(current),
                    parent_path: current
                        .parent()
                        .unwrap_or(Path::new(""))
                        .to_string_lossy()
                        .as_ref()
                        .into(),
                    status: if current != &entry.source {
                        i18n::tr(lang, "bulk_recovery").into()
                    } else if failure_here {
                        diagnostic.clone().into()
                    } else {
                        SharedString::new()
                    },
                    error: failure_here || current != &entry.source,
                }
            })
            .collect::<Vec<_>>();
        window.set_bulk_rows(Rc::new(VecModel::from(rows)).into());
        window.set_bulk_summary(detail.clone().into());
        window.set_bulk_finished(true);
    } else {
        dismiss_dialog(window, state);
    }
    window.invoke_op_progress(terminal_op_progress(
        op_id,
        if needs_review {
            OP_ERROR
        } else if outcome.cancelled {
            OP_CANCELLED
        } else {
            OP_SUCCESS
        },
        i18n::tr(lang, "bulk_title"),
        detail,
    ));
    window.invoke_op_finished(op_id);
}

pub(super) fn relocated(path: &Path, steps: &[(PathBuf, PathBuf)]) -> PathBuf {
    steps.iter().fold(path.to_owned(), |path, (from, to)| {
        ops::relocated_path(&path, from, to).unwrap_or(path)
    })
}

/// Resolve an original path once, without repeatedly following a cycle's final
/// names. Ancestor lookups also carry rows inside a renamed directory.
fn final_location(path: &Path, locations: &HashMap<PathBuf, PathBuf>) -> PathBuf {
    for ancestor in path.ancestors() {
        if let Some(target) = locations.get(&reservation_key(ancestor)) {
            return ops::relocated_path(path, ancestor, target).unwrap_or_else(|| path.to_owned());
        }
    }
    path.to_owned()
}

fn invalidate_content(state: &AppState, plan: &engine::Plan, outcome: &engine::Outcome) {
    let roots: HashSet<_> = plan
        .entries
        .iter()
        .filter(|entry| entry.changes())
        .flat_map(|entry| [&entry.source, &entry.target])
        .chain(outcome.locations.iter().map(|(_, current)| current))
        .map(|path| reservation_key(path))
        .collect();
    let affected = |path: &Path| {
        path.ancestors()
            .any(|parent| roots.contains(&reservation_key(parent)))
    };
    let mut paths: Vec<_> = {
        let cache = state.thumb_cache.borrow();
        cache
            .map
            .keys()
            .chain(cache.failed.iter())
            .map(PathBuf::from)
            .chain(cache.kinds.paths().cloned())
            .filter(|path| affected(path))
            .collect()
    };
    {
        let queue = state
            .thumb_scheduler
            .queue
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        paths.extend(
            queue
                .pending
                .keys()
                .chain(queue.in_flight.keys())
                .filter(|path| affected(path))
                .cloned(),
        );
    }
    invalidate_thumbnail_paths(state, &paths);
    state.imgmeta_gen.fetch_add(1, Ordering::Relaxed);
    state
        .imgmeta_cache
        .borrow_mut()
        .retain(|path, _| !affected(Path::new(path)));
}

pub(super) fn redirect_shortcuts(
    redirects: &mut std::collections::BTreeMap<String, String>,
    visible: &[PathBuf],
    steps: &[(PathBuf, PathBuf)],
) {
    // Keep original shortcut keys; temporary staging names must not become keys.
    for destination in redirects.values_mut() {
        *destination = relocated(Path::new(destination), steps)
            .to_string_lossy()
            .into_owned();
    }
    for path in visible {
        let target = relocated(path, steps);
        if target != *path {
            redirects.insert(
                path.to_string_lossy().into_owned(),
                target.to_string_lossy().into_owned(),
            );
        }
    }
}

#[cfg(test)]
mod tests;
