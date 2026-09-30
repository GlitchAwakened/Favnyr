//! UI coordination for managed archive recipes. Requests retain the original
//! selection while dialogs, workers and publication advance independently.

use super::*;
use crate::archives::{self, Action, Format, Job, Kind, Outcome, Output, TarCompression};

type Delivery = (i32, Job, Outcome, bool);

#[derive(Default)]
pub(super) struct State {
    queued: RefCell<VecDeque<Job>>,
    prompt: RefCell<Option<Job>>,
    running: Cell<bool>,
    ready: RefCell<Option<(i32, Output)>>,
    deliveries: Arc<std::sync::Mutex<VecDeque<Delivery>>>,
}

pub(super) fn run_opener(
    window: &MainWindow,
    state: &AppState,
    opener: &openers::Opener,
    paths: &[PathBuf],
) -> anyhow::Result<()> {
    let Some(action) = archives::action(opener) else {
        return actions::run_opener(opener, paths);
    };
    if opener.elevated || paths.is_empty() || !actions::program_is_valid(&opener.program) {
        anyhow::bail!("managed archive recipe requires a selection and a non-elevated executable");
    }
    let expanded = actions::expand_program_path(&opener.program);
    let program = if expanded.is_file() {
        std::fs::canonicalize(expanded)?
    } else {
        expanded
    };
    let mut queue = state.archive.queued.borrow_mut();
    match action {
        Action::Compress => {
            let context = openers::TagContext::from_path(&paths[0]);
            let name = if paths.len() == 1 {
                context.setname
            } else {
                context.dirname
            };
            queue.push_back(Job {
                program,
                inputs: paths.to_vec(),
                kind: Kind::Compress {
                    name,
                    format: Format::Zip,
                    tar_compression: TarCompression::None,
                    level: 5,
                },
            });
        }
        Action::ExtractHere | Action::ExtractFolder => {
            for path in paths {
                queue.push_back(Job {
                    program: program.clone(),
                    inputs: vec![path.clone()],
                    kind: Kind::Extract {
                        folder: action == Action::ExtractFolder,
                    },
                });
            }
        }
    }
    window.set_archive_work_pending(true);
    Ok(())
}

pub(super) fn install(window: &MainWindow, state: &AppState) {
    let st = state.clone();
    let weak = window.as_weak();
    window.on_archive_poll(move || {
        if let Some(w) = weak.upgrade() {
            poll(&w, &st);
        }
    });
    let st = state.clone();
    let weak = window.as_weak();
    window.on_archive_cancel(move || {
        st.archive.prompt.borrow_mut().take();
        if let Some(w) = weak.upgrade() {
            w.set_archive_password(SharedString::new());
            w.set_archive_confirm_password(SharedString::new());
            w.set_archive_open(false);
        }
    });
    let st = state.clone();
    let weak = window.as_weak();
    window.on_archive_submit(move || {
        let Some(w) = weak.upgrade() else { return };
        let lang = st.config.borrow().language;
        let compression = !w.get_archive_password_only();
        let format = Format::from_index(w.get_archive_format());
        let tar_compression = if format == Format::Tar {
            TarCompression::from_index(w.get_archive_tar_compression())
        } else {
            TarCompression::None
        };
        let use_password =
            !compression || (w.get_archive_use_password() && format.supports_password());
        // Hidden fields must never accidentally encrypt a later operation.
        let password = if use_password {
            w.get_archive_password().to_string()
        } else {
            String::new()
        };
        if !archives::valid_password(&password)
            || (use_password && password.is_empty())
            || (compression
                && use_password
                && password != w.get_archive_confirm_password().as_str())
        {
            w.set_archive_error(i18n::tr(lang, "archive_password_invalid").into());
            return;
        }
        let name = w.get_archive_name().to_string();
        if compression && format == Format::Zip && !archives::valid_zip_password(&password) {
            w.set_archive_error(i18n::tr(lang, "archive_zip_password_invalid").into());
            return;
        }
        let normalized_name = compression
            .then(|| archives::output_name(&name, format, tar_compression))
            .flatten();
        if compression && normalized_name.is_none() {
            w.set_archive_error(i18n::tr(lang, "archive_name_invalid").into());
            return;
        }
        let Some(mut job) = st.archive.prompt.borrow_mut().take() else {
            return;
        };
        if compression {
            let name = normalized_name.expect("validated compression name");
            let level = [0, 1, 3, 5, 7, 9][w.get_archive_level().clamp(0, 5) as usize];
            job.kind = Kind::Compress {
                name,
                format,
                tar_compression,
                level,
            };
        }
        w.set_archive_password(SharedString::new());
        w.set_archive_confirm_password(SharedString::new());
        w.set_archive_open(false);
        start(&w, &st, job, password);
    });
}

fn show_prompt(window: &MainWindow, state: &AppState, job: Job, wrong: bool) {
    let lang = state.config.borrow().language;
    let password_only = matches!(job.kind, Kind::Extract { .. });
    window.set_archive_password_only(password_only);
    window.set_archive_use_password(false);
    window.set_archive_password(SharedString::new());
    window.set_archive_confirm_password(SharedString::new());
    window.set_archive_error(if wrong {
        i18n::tr(lang, "archive_wrong_password").into()
    } else {
        SharedString::new()
    });
    window.set_archive_subject(job.inputs[0].display().to_string().into());
    if let Kind::Compress { ref name, .. } = job.kind {
        window.set_archive_name(name.as_str().into());
        window.set_archive_format(0);
        window.set_archive_tar_compression(0);
        window.set_archive_level(3);
    }
    *state.archive.prompt.borrow_mut() = Some(job);
    window.set_archive_open(true);
}

fn start(window: &MainWindow, state: &AppState, job: Job, password: String) {
    let cancel = Arc::new(AtomicBool::new(false));
    let id = state.ops.register(OpHandle {
        cancel: Some(cancel.clone()),
        pending_focus: None,
        targets: Vec::new(),
        thumbnail_invalidations: Vec::new(),
        transient_cleanup: None,
    });
    state.archive.running.set(true);
    sync_op_busy(window, state);
    let lang = state.config.borrow().language;
    let title = i18n::tr(
        lang,
        if matches!(job.kind, Kind::Compress { .. }) {
            "archive_compressing"
        } else {
            "archive_extracting"
        },
    );
    let detail = job.inputs[0]
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let deliveries = state.archive.deliveries.clone();
    let weak = window.as_weak();
    let _ = std::thread::Builder::new()
        .name("archive-worker".into())
        .spawn(move || {
            let begin = Instant::now();
            let mut last = Instant::now() - Duration::from_secs(1);
            let outcome = archives::run(&job, &password, &cancel, |percent| {
                if last.elapsed() >= Duration::from_millis(100) {
                    last = Instant::now();
                    push_op(
                        &weak,
                        id,
                        if percent.is_some() {
                            OP_RUNNING
                        } else {
                            OP_SCAN
                        },
                        begin.elapsed() >= Duration::from_millis(400),
                        f32::from(percent.unwrap_or(0)) / 100.0,
                        title.clone(),
                        detail.clone(),
                        percent.map(|n| format!("{n} %")).unwrap_or_default(),
                    );
                }
            });
            deliveries
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push_back((id, job, outcome, !password.is_empty()));
        })
        .map_err(|err| {
            state.archive.running.set(false);
            window.invoke_op_finished(id);
            error!(error = %err, "cannot start archive worker");
            notice(window, i18n::tr(lang, "archive_failed"), NoticeKind::Error);
        });
}

fn poll(window: &MainWindow, state: &AppState) {
    let archive = &state.archive;
    // Defer dialogs and publication until the current modal/conflict completes.
    // Keeping the result owned also keeps its staging directory alive.
    if !window.get_popup_overlay_open() {
        let delivery = archive
            .deliveries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop_front();
        if let Some((id, job, outcome, tried_password)) = delivery {
            archive.running.set(false);
            let cancelled = state
                .ops
                .active
                .borrow()
                .get(&id)
                .and_then(|op| op.cancel.as_ref())
                .is_some_and(|flag| flag.load(Ordering::Relaxed));
            // Cancellation may arrive after the worker exits but before this
            // delivery is consumed. Do not reopen a password prompt then.
            let outcome = if cancelled {
                Outcome::Cancelled
            } else {
                outcome
            };
            match outcome {
                Outcome::Ready(output) => {
                    *archive.ready.borrow_mut() = Some((id, output));
                }
                other => {
                    window.invoke_op_dismiss(id);
                    window.invoke_op_finished(id);
                    match other {
                        Outcome::PasswordRequired => {
                            show_prompt(window, state, job, tried_password)
                        }
                        Outcome::Failed => notice(
                            window,
                            i18n::tr(state.config.borrow().language, "archive_failed"),
                            NoticeKind::Error,
                        ),
                        Outcome::Cancelled => push_op(
                            &window.as_weak(),
                            id,
                            OP_CANCELLED,
                            true,
                            0.0,
                            i18n::tr(state.config.borrow().language, "op_cancelled"),
                            String::new(),
                            String::new(),
                        ),
                        Outcome::Ready(_) => unreachable!(),
                    }
                }
            }
        }
        if !window.get_popup_overlay_open() && state.paste_job.borrow().is_none() {
            let ready = archive.ready.borrow_mut().take();
            if let Some((id, mut output)) = ready {
                let cancelled = state
                    .ops
                    .active
                    .borrow()
                    .get(&id)
                    .and_then(|op| op.cancel.as_ref())
                    .is_some_and(|flag| flag.load(Ordering::Relaxed));
                window.invoke_op_dismiss(id);
                window.invoke_op_finished(id);
                if !cancelled {
                    let cleanup = TransientDropGuard::new(std::mem::take(&mut output.staging.0));
                    // Copy from the private tree: this does not consume an
                    // unrelated user clipboard cut selection.
                    begin_paste_with_cleanup(
                        window,
                        state,
                        ClipOp::Copy,
                        output.destination,
                        output.sources,
                        Some(cleanup),
                    );
                }
            }
        }
        if !window.get_popup_overlay_open()
            && !archive.running.get()
            && archive.prompt.borrow().is_none()
            && archive.ready.borrow().is_none()
        {
            let next = archive.queued.borrow_mut().pop_front();
            if let Some(job) = next {
                if matches!(job.kind, Kind::Compress { .. }) {
                    show_prompt(window, state, job, false);
                } else {
                    start(window, state, job, String::new());
                }
            }
        }
    }
    window.set_archive_work_pending(
        archive.running.get()
            || archive.ready.borrow().is_some()
            || archive.prompt.borrow().is_some()
            || !archive.queued.borrow().is_empty(),
    );
}
