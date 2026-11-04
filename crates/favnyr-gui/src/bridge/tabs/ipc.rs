//! Acknowledged tab batches. Lost replies retain the source instead of guessing.

use super::{
    Tab, TabBook,
    batch::{Batch, Destination},
    drag,
    transfer::{self, Removal},
};
use crate::{
    bridge::{
        AppState, MainWindow, Panel, SUPPRESS_WORKSPACE_PERSIST, add_paths_to_favorite,
        clear_external_hover, i18n, refresh_all_panels, set_external_hover, show_notice, shutdown,
    },
    tab_transfer::{Offer, Outcome, Packet, Receipts, TIMEOUT_MS},
    winmsg,
};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct Outgoing {
    target: isize,
    offer: Offer,
    batch: Batch,
}
struct Incoming {
    sender: isize,
    offer: Offer,
    query: i32,
}

#[derive(Default)]
pub(in crate::bridge) struct State {
    outgoing: RefCell<Option<Outgoing>>,
    incoming: RefCell<Option<Incoming>>,
    receipts: RefCell<Receipts>,
    serial: Cell<u64>,
    query: Cell<i32>,
}

impl State {
    fn take_reply(&self, sender: isize, id: &str, now: u64) -> Option<Outgoing> {
        let mut outgoing = self.outgoing.borrow_mut();
        if outgoing
            .as_ref()
            .is_some_and(|o| o.target == sender && o.offer.id == id && now <= o.offer.expires)
        {
            outgoing.take()
        } else {
            None
        }
    }
}

pub(in crate::bridge) fn pending(state: &AppState) -> bool {
    state.tab_ipc.outgoing.borrow().is_some() || state.tab_ipc.incoming.borrow().is_some()
}
pub(in crate::bridge) fn receiving(state: &AppState) -> bool {
    state.tab_ipc.incoming.borrow().is_some()
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn warn(window: &MainWindow, state: &AppState, key: &str) {
    show_notice(window, i18n::tr(state.config.borrow().language, key));
}
fn reply(target: isize, id: String, outcome: Outcome) {
    if let Some(payload) = (Packet::Reply { id, outcome }).encode() {
        winmsg::send_batch(target, &payload);
    }
}

pub(in crate::bridge) fn cancel(window: &MainWindow, state: &AppState) {
    state.tab_ipc.outgoing.borrow_mut().take();
    let incoming = state.tab_ipc.incoming.borrow_mut().take();
    if let Some(incoming) = incoming {
        finish(window, state, incoming, Outcome::Rejected);
    }
}

pub(in crate::bridge) fn send(
    window: &MainWindow,
    state: &AppState,
    batch: Batch,
    target: isize,
    at: (i32, i32),
) {
    if pending(state) || shutdown::stopping(state) {
        warn(window, state, "tab_transfer_busy");
        return;
    }
    let data = {
        let panels = state.panels.borrow();
        let Some(p) = panels.iter().find(|p| p.tabs.identity == batch.owner) else {
            return;
        };
        if panels.len() == 1 && batch.ids.len() == p.tabs.tabs.len() && shutdown::pending(state) {
            drop(panels);
            shutdown::request(window, state);
            return;
        }
        let Some(data) = drag::snapshot(p, &batch) else {
            return;
        };
        data
    };
    let serial = state.tab_ipc.serial.get().wrapping_add(1);
    state.tab_ipc.serial.set(serial);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let id = format!("{:x}-{stamp:x}-{serial:x}", std::process::id());
    let offer = Offer {
        id: id.clone(),
        expires: now().saturating_add(TIMEOUT_MS),
        at,
        data,
    };
    let packet = Packet::Offer(offer);
    let Some(payload) = packet.encode() else {
        warn(window, state, "tab_transfer_failed");
        return;
    };
    let Packet::Offer(offer) = packet else {
        unreachable!()
    };
    let expires = offer.expires;
    *state.tab_ipc.outgoing.borrow_mut() = Some(Outgoing {
        target,
        offer,
        batch,
    });
    if !winmsg::send_batch(target, &payload) {
        state.tab_ipc.outgoing.borrow_mut().take();
        warn(window, state, "tab_transfer_uncertain");
        return;
    }
    let weak = window.as_weak();
    let st = state.clone();
    slint::Timer::single_shot(
        Duration::from_millis(expires.saturating_sub(now())),
        move || {
            let expired = st
                .tab_ipc
                .outgoing
                .borrow()
                .as_ref()
                .is_some_and(|o| o.offer.id == id);
            if expired {
                st.tab_ipc.outgoing.borrow_mut().take();
                if let Some(w) = weak.upgrade() {
                    warn(&w, &st, "tab_transfer_uncertain");
                }
            }
        },
    );
}

fn acknowledge(window: &MainWindow, state: &AppState, sender: isize, id: &str, outcome: Outcome) {
    let Some(outgoing) = state.tab_ipc.take_reply(sender, id, now()) else {
        return;
    };
    if outcome == Outcome::Bookmarked {
        return;
    }
    if outcome == Outcome::Rejected {
        warn(window, state, "tab_transfer_failed");
        return;
    }
    let unchanged = state
        .panels
        .borrow()
        .iter()
        .find(|p| p.tabs.identity == outgoing.batch.owner)
        .and_then(|p| drag::snapshot(p, &outgoing.batch))
        .is_some_and(|data| data == outgoing.offer.data);
    if !unchanged || shutdown::stopping(state) {
        warn(window, state, "tab_transfer_uncertain");
        return;
    }
    let last = {
        let p = state.panels.borrow();
        p.len() == 1 && p[0].tabs.tabs.len() == outgoing.batch.ids.len()
    };
    if last && shutdown::pending(state) {
        warn(window, state, "tab_transfer_uncertain");
        return;
    }
    let removed = transfer::remove_batch(
        &mut state.panels.borrow_mut(),
        &mut state.layout.borrow_mut(),
        &mut state.active_panel.borrow_mut(),
        &outgoing.batch,
    );
    match removed {
        Some(Removal::LastView) => {
            SUPPRESS_WORKSPACE_PERSIST.store(true, std::sync::atomic::Ordering::SeqCst);
            window.invoke_request_close();
        }
        Some(Removal::Remaining) => {
            refresh_all_panels(window, state);
            state.persist_workspace();
        }
        None => warn(window, state, "tab_transfer_uncertain"),
    }
}

fn finish(window: &MainWindow, state: &AppState, incoming: Incoming, outcome: Outcome) {
    state
        .tab_ipc
        .receipts
        .borrow_mut()
        .record(incoming.sender, &incoming.offer, outcome);
    clear_external_hover(window);
    reply(incoming.sender, incoming.offer.id, outcome);
}

pub(in crate::bridge) fn receive(
    window: &MainWindow,
    state: &AppState,
    sender: isize,
    payload: &str,
) {
    let Some(packet) = Packet::decode(payload) else {
        return;
    };
    let offer = match packet {
        Packet::Reply { id, outcome } => {
            acknowledge(window, state, sender, &id, outcome);
            return;
        }
        Packet::Offer(offer) => offer,
    };
    let now = now();
    let known = state
        .tab_ipc
        .receipts
        .borrow_mut()
        .lookup(sender, &offer.id, now);
    if let Some(outcome) = known {
        reply(sender, offer.id, outcome);
        return;
    }
    if state
        .tab_ipc
        .incoming
        .borrow()
        .as_ref()
        .is_some_and(|i| i.sender == sender && i.offer.id == offer.id)
    {
        return;
    }
    if pending(state)
        || shutdown::stopping(state)
        || window.get_popup_overlay_open()
        || window.get_file_drag_active()
        || (window.get_drag_active() && window.get_drag_source_panel() >= 0)
        || !state.tab_ipc.receipts.borrow().accepts(offer.expires, now)
    {
        reply(sender, offer.id, Outcome::Rejected);
        return;
    }
    let query = state.tab_ipc.query.get().checked_add(1).unwrap_or(1);
    state.tab_ipc.query.set(query);
    set_external_hover(window, offer.at.0, offer.at.1);
    window.set_external_tab_legacy(false);
    window.set_tab_drag_count(offer.data.dirs.len() as i32);
    if window.get_fav_hover_panel() {
        let success = add_paths_to_favorite(
            window,
            state,
            &offer.data.dirs,
            &window.get_fav_hover_container(),
        );
        finish(
            window,
            state,
            Incoming {
                sender,
                offer,
                query,
            },
            if success {
                Outcome::Bookmarked
            } else {
                Outcome::Rejected
            },
        );
        return;
    }
    *state.tab_ipc.incoming.borrow_mut() = Some(Incoming {
        sender,
        offer,
        query,
    });
    window.set_external_tab_drop_query(query);
    let weak = window.as_weak();
    let st = state.clone();
    // A query with no responding view is an invalid destination, not an append fallback.
    slint::Timer::single_shot(Duration::from_millis(250), move || {
        let waiting = st
            .tab_ipc
            .incoming
            .borrow()
            .as_ref()
            .is_some_and(|i| i.query == query);
        if waiting {
            let incoming = st.tab_ipc.incoming.borrow_mut().take().unwrap();
            if let Some(w) = weak.upgrade() {
                finish(&w, &st, incoming, Outcome::Rejected);
            }
        }
    });
}

pub(in crate::bridge) fn install(window: &MainWindow, state: &AppState) {
    let weak = window.as_weak();
    let st = state.clone();
    window.on_external_tab_drop_resolved(move |query, panel, zone, gap, group| {
        let Some(w) = weak.upgrade() else { return };
        if st
            .tab_ipc
            .incoming
            .borrow()
            .as_ref()
            .is_none_or(|i| i.query != query)
        {
            return;
        }
        let incoming = st.tab_ipc.incoming.borrow_mut().take().unwrap();
        if now() > incoming.offer.expires || shutdown::stopping(&st) {
            finish(&w, &st, incoming, Outcome::Rejected);
            return;
        }
        let saved = &incoming.offer.data;
        let book = TabBook {
            identity: Default::default(),
            selection: Default::default(),
            focused_group: None,
            active_group: None,
            groups: saved.view.tab_groups.clone(),
            tabs: saved
                .view
                .tabs
                .iter()
                .zip(&saved.dirs)
                .map(|(t, p)| Tab::restored(p.clone(), t))
                .collect(),
            active: saved.view.active_tab,
        };
        let mut candidate = Panel::from_book(
            book,
            saved.view.columns.clone(),
            saved.view.tab_bar_mode,
            saved.view.vbar_width,
        );
        candidate.tab_bar_restore_mode = saved.view.tab_bar_restore_mode;
        let dest = (gap >= 0).then(|| Destination {
            gap: gap as usize,
            group: (!group.is_empty()).then(|| group.to_string()),
        });
        let renamed = group.is_empty()
            && (gap >= 0 || zone == 1)
            && st.panels.borrow().get(panel as usize).is_some_and(|p| {
                transfer::group_names_overlap(&saved.view.tab_groups, &p.tabs.groups)
            });
        let target = transfer::receive(
            &mut st.panels.borrow_mut(),
            &mut st.layout.borrow_mut(),
            candidate,
            panel as usize,
            zone,
            dest.as_ref(),
        );
        if let Some(target) = target {
            *st.active_panel.borrow_mut() = target;
            st.persist_workspace();
            finish(&w, &st, incoming, Outcome::Moved);
            refresh_all_panels(&w, &st);
            if renamed {
                crate::bridge::notice(
                    &w,
                    i18n::tr(st.config.borrow().language, "tab_drag_groups_renamed"),
                    crate::bridge::NoticeKind::Success,
                );
            }
            #[cfg(windows)]
            winmsg::focus_self();
        } else {
            finish(&w, &st, incoming, Outcome::Rejected);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outgoing() -> Outgoing {
        let book = super::super::tests::sample(3, 1);
        let batch = Batch::single(&book, 1).unwrap();
        let panel = Panel::from_book(book, favnyr_core::columns::default_columns(), 0, 0.0);
        Outgoing {
            target: 42,
            offer: Offer {
                id: "a1".into(),
                expires: 8000,
                at: (300, 400),
                data: drag::snapshot(&panel, &batch).unwrap(),
            },
            batch,
        }
    }

    #[test]
    fn only_an_on_time_matching_reply_consumes_the_pending_transfer() {
        let state = State::default();
        *state.outgoing.borrow_mut() = Some(outgoing());
        assert!(state.take_reply(43, "a1", 100).is_none());
        assert!(state.take_reply(42, "a2", 100).is_none());
        assert!(state.take_reply(42, "a1", 8001).is_none());
        assert!(state.outgoing.borrow().is_some());
        assert!(state.take_reply(42, "a1", 8000).is_some());
        assert!(state.take_reply(42, "a1", 8000).is_none());
        *state.outgoing.borrow_mut() = Some(outgoing());
        state.outgoing.borrow_mut().take();
        assert!(state.take_reply(42, "a1", 100).is_none());
    }

    #[test]
    fn a_changed_or_removed_source_no_longer_matches_its_offer() {
        let book = super::super::tests::sample(3, 1);
        let batch = Batch::single(&book, 1).unwrap();
        let mut panel = Panel::from_book(book, favnyr_core::columns::default_columns(), 0, 0.0);
        let saved = drag::snapshot(&panel, &batch).unwrap();
        panel.tabs.tabs[1].current_path = "folder04".into();
        assert_ne!(drag::snapshot(&panel, &batch).unwrap(), saved);
        panel.tabs.take_tab(1);
        assert!(drag::snapshot(&panel, &batch).is_none());
    }
}
