//! Window close protection shared by native and application close requests.

use super::*;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Phase {
    #[default]
    Idle,
    Prompt,
    Armed,
    Stopping,
}

impl Phase {
    fn confirm(self) -> Self {
        match self {
            Self::Prompt => Self::Armed,
            Self::Armed => Self::Stopping,
            other => other,
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    phase: Cell<Phase>,
    timer: slint::Timer,
}

pub(super) fn stopping(state: &AppState) -> bool {
    state.shutdown.phase.get() == Phase::Stopping
}

pub(super) fn pending(state: &AppState) -> bool {
    tabs::ipc::pending(state)
        || state.ops.in_flight()
        || state.paste_job.borrow().is_some()
        || archive::pending(state)
        || crate::background_work::in_flight()
}

/// Return true only when the native close request may hide the window.
pub(super) fn request(window: &MainWindow, state: &AppState) -> bool {
    if !pending(state) {
        return true;
    }
    if state.shutdown.phase.get() == Phase::Idle {
        state.shutdown.phase.set(Phase::Prompt);
        window.set_exit_confirm(false);
        window.set_exit_stopping(false);
        window.set_exit_warning_open(true);
    }
    false
}

fn hide(window: &MainWindow, state: &AppState) {
    state.shutdown.timer.stop();
    if let Err(error) = window.window().hide() {
        error!(%error, "cannot close application window");
        stay(window, state);
    }
}

fn stay(window: &MainWindow, state: &AppState) {
    state.shutdown.timer.stop();
    state.shutdown.phase.set(Phase::Idle);
    window.set_exit_warning_open(false);
    window.set_exit_confirm(false);
    window.set_exit_stopping(false);
}

fn cancel_pending(window: &MainWindow, state: &AppState) {
    tabs::ipc::cancel(window, state);
    state.ops.cancel_all();
    state.paste_job.borrow_mut().take();
    state.pending_file_drop.borrow_mut().take();
    state.external_drop_paths.borrow_mut().clear();
    window.set_paste_conflict_open(false);
    window.set_file_drop_menu_open(false);
    archive::cancel_for_shutdown(window, state);
}

fn poll(window: &MainWindow, state: &AppState) {
    if !stopping(state) {
        return;
    }
    cancel_pending(window, state);
    // Apply completion metadata before persistence, even if its queued wakeup
    // has not yet been dispatched by the event loop.
    window.invoke_process_op_events();
    if !pending(state) {
        hide(window, state);
    }
}

pub(super) fn install(window: &MainWindow, state: &AppState) {
    let weak = window.as_weak();
    let st = state.clone();
    window.window().on_close_requested(move || {
        if weak.upgrade().is_none_or(|w| request(&w, &st)) {
            slint::CloseRequestResponse::HideWindow
        } else {
            slint::CloseRequestResponse::KeepWindowShown
        }
    });

    let weak = window.as_weak();
    let st = state.clone();
    window.on_request_close(move || {
        if let Some(w) = weak.upgrade()
            && request(&w, &st)
        {
            hide(&w, &st);
        }
    });

    let weak = window.as_weak();
    let st = state.clone();
    window.on_exit_stay(move || {
        if let Some(w) = weak.upgrade() {
            stay(&w, &st);
        }
    });

    let weak = window.as_weak();
    let st = state.clone();
    window.on_exit_cancel_operations(move || {
        let Some(w) = weak.upgrade() else { return };
        let phase = st.shutdown.phase.get().confirm();
        st.shutdown.phase.set(phase);
        if phase == Phase::Armed {
            w.set_exit_confirm(true);
        } else if phase == Phase::Stopping {
            w.set_exit_stopping(true);
            cancel_pending(&w, &st);
            let weak = w.as_weak();
            let state = st.clone();
            // Active only during explicit cancellation. No work is added to
            // ordinary rendering, navigation or operation progress updates.
            st.shutdown.timer.start(
                slint::TimerMode::Repeated,
                Duration::from_millis(100),
                move || {
                    if let Some(w) = weak.upgrade() {
                        poll(&w, &state);
                    }
                },
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_requires_two_explicit_choices() {
        assert_eq!(Phase::Idle.confirm(), Phase::Idle);
        assert_eq!(Phase::Prompt.confirm(), Phase::Armed);
        assert_eq!(Phase::Armed.confirm(), Phase::Stopping);
        assert_eq!(Phase::Stopping.confirm(), Phase::Stopping);
    }
}
