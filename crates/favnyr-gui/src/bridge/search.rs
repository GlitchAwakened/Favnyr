//! On-demand descendant search. Filesystem work never runs on the UI thread.

use super::*;

const BATCH_SIZE: usize = 256;
const RESULT_LIMIT: usize = 10_000;
const DIRECTORY_LIMIT: usize = 100_000;
const POLL_INTERVAL: Duration = Duration::from_millis(120);

pub(super) struct Session {
    id: u64,
    pub root: PathBuf,
    pub query: String,
    show_hidden: bool,
    cancel: Arc<AtomicBool>,
    receiver: Option<mpsc::Receiver<Delivery>>,
    entries: Vec<Entry>,
    pending: VecDeque<Entry>,
    pub finished: bool,
    skipped: usize,
    limited: bool,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

struct Request {
    root: PathBuf,
    query: String,
    show_hidden: bool,
    cancel: Arc<AtomicBool>,
    sender: mpsc::SyncSender<Delivery>,
    result_limit: usize,
    directory_limit: usize,
}

enum Delivery {
    Batch(Vec<Entry>),
    Finished { skipped: usize, limited: bool },
}

thread_local! {
    // One lazy worker bounds disk concurrency across all views. Cancelled queued
    // requests are discarded before touching the filesystem.
    static WORKER: RefCell<Option<mpsc::Sender<Request>>> = const { RefCell::new(None) };
}

fn submit(request: Request) {
    WORKER.with(|slot| {
        let mut slot = slot.borrow_mut();
        let sender = slot.get_or_insert_with(|| {
            let (sender, receiver) = mpsc::channel::<Request>();
            std::thread::spawn(move || {
                while let Ok(request) = receiver.recv() {
                    if !request.cancel.load(Ordering::Relaxed) {
                        walk(request);
                    }
                }
            });
            sender
        });
        let _ = sender.send(request);
    });
}

fn walk(request: Request) {
    let mut pending = vec![request.root];
    let needle = request.query.to_lowercase();
    let mut batch = Vec::with_capacity(BATCH_SIZE);
    let mut matches = 0;
    let mut directories = 0;
    let mut skipped = 0;
    let limited = Cell::new(false);
    let mut last_send = Instant::now();
    while let Some(path) = pending.pop() {
        if request.cancel.load(Ordering::Relaxed) || limited.get() {
            break;
        }
        directories += 1;
        if directories > request.directory_limit {
            limited.set(true);
            break;
        }
        let result = rfs::visit_dir_while(
            &path,
            request.show_hidden,
            || request.cancel.load(Ordering::Relaxed) || limited.get(),
            |entry| {
                // Links remain searchable, but are never traversed. This also
                // keeps junctions from escaping the requested tree or cycling.
                if may_descend(&entry) {
                    if pending.len() < request.directory_limit {
                        pending.push(entry.path.clone());
                    } else {
                        limited.set(true);
                    }
                }
                if name_contains_filter(&entry.name, &needle) {
                    if matches == request.result_limit {
                        limited.set(true);
                        return;
                    }
                    matches += 1;
                    batch.push(entry);
                }
                if !batch.is_empty()
                    && (batch.len() >= BATCH_SIZE || last_send.elapsed() >= POLL_INTERVAL)
                {
                    if request
                        .sender
                        .send(Delivery::Batch(std::mem::take(&mut batch)))
                        .is_err()
                    {
                        request.cancel.store(true, Ordering::Relaxed);
                    }
                    last_send = Instant::now();
                }
            },
        );
        if let Err(err) = result {
            skipped += 1;
            debug!(path = %path.display(), error = %err, "search skipped directory");
        }
    }
    if request.cancel.load(Ordering::Relaxed) {
        return;
    }
    if !batch.is_empty() && request.sender.send(Delivery::Batch(batch)).is_err() {
        return;
    }
    let _ = request.sender.send(Delivery::Finished {
        skipped,
        limited: limited.get(),
    });
}

fn may_descend(entry: &Entry) -> bool {
    if !entry.is_dir || entry.is_symlink {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Reparse directories can redirect traversal even when their type is
        // not reported as a symbolic link. Never follow them during a search.
        std::fs::symlink_metadata(&entry.path)
            .is_ok_and(|metadata| metadata.file_attributes() & 0x400 == 0)
    }
    #[cfg(not(windows))]
    true
}

pub(super) fn active(state: &AppState) -> bool {
    state
        .panels
        .borrow()
        .get(*state.active_panel.borrow())
        .is_some_and(|panel| panel.search.is_some())
}

pub(super) fn reject_ambiguous_destination(window: &MainWindow, state: &AppState) -> bool {
    if !active(state) {
        return false;
    }
    show_notice(
        window,
        i18n::tr(state.config.borrow().language, "search_choose_folder"),
    );
    true
}

fn containing_folder(root: &Path, paths: &[PathBuf]) -> Option<PathBuf> {
    let [path] = paths else { return None };
    let parent = path.parent()?;
    (!ops::paths_equal(parent, root) && ops::is_within(parent, root)).then(|| parent.to_path_buf())
}

pub(super) fn reveal_target(state: &AppState, paths: &[PathBuf]) -> Option<PathBuf> {
    let panels = state.panels.borrow();
    let search = panels.get(*state.active_panel.borrow())?.search.as_ref()?;
    containing_folder(&search.root, paths)
}

pub(super) fn install(window: &MainWindow, state: &AppState) {
    let weak = window.as_weak();
    let st = state.clone();
    window.on_search_root_action_allowed(move || {
        weak.upgrade()
            .is_some_and(|window| !reject_ambiguous_destination(&window, &st))
    });
    let weak = window.as_weak();
    let st = state.clone();
    window.on_search_scope_toggle(move || {
        let Some(window) = weak.upgrade() else { return };
        let index = *st.active_panel.borrow();
        if active(&st) {
            clear(&st, index);
            refresh_listing(&window, &st, &st.current_path());
        } else if !st.active_manual_order() && !st.active_ext_filter_on() {
            let query = st.filter.borrow().clone();
            if !query.is_empty() {
                start(&window, &st, index, query);
            }
        }
    });
    let weak = window.as_weak();
    let st = state.clone();
    window.on_action_reveal_result(move |new_tab| {
        let Some(window) = weak.upgrade() else { return };
        let paths = selected_paths(&st);
        let Some(parent) = reveal_target(&st, &paths) else {
            return;
        };
        if new_tab {
            open_dir_in_new_tab(&window, &st, &parent);
        } else {
            load_directory(&window, &st, &parent, true);
        }
        select_child_from(&window, &st, &parent, &paths[0]);
    });
}

/// Debouncing uses a session identity rather than a panel index, which may
/// change during a split, swap or close. Dropping a session cancels its worker.
pub(super) fn start(window: &MainWindow, state: &AppState, index: usize, query: String) {
    let id = state.listing_serial.fetch_add(1, Ordering::Relaxed) + 1;
    {
        let mut panels = state.panels.borrow_mut();
        let Some(panel) = panels.get_mut(index) else {
            return;
        };
        let tab = &panel.tabs.tabs[panel.tabs.active];
        panel.search = Some(Session {
            id,
            root: tab.current_path.clone(),
            query,
            show_hidden: tab.show_hidden,
            cancel: Arc::new(AtomicBool::new(false)),
            receiver: None,
            entries: Vec::new(),
            pending: VecDeque::new(),
            finished: false,
            skipped: 0,
            limited: false,
        });
        panel.pending_initial = false;
        panel.pending_listing = false;
        panel.pending_select = None;
        panel.unavailable = false;
        panel.listing_failed = false;
        panel.last_recovery = None;
        panel.hidden_count = 0;
        panel.reset_rows_viewport();
        panel.replace_rows(Vec::new());
        let tab = &mut panel.tabs.tabs[panel.tabs.active];
        tab.selection_anchor = -1;
        tab.cursor = -1;
    }
    if index == *state.active_panel.borrow() {
        // Search results are a snapshot. Refresh explicitly rescans the tree;
        // a root watcher must not repeatedly restart a descendant search.
        state.watcher_gen.fetch_add(1, Ordering::SeqCst);
    }
    update_panels_ui(window, state);
    let weak = window.as_weak();
    let st = state.clone();
    slint::Timer::single_shot(Duration::from_millis(250), move || {
        let Some(window) = weak.upgrade() else { return };
        {
            let mut panels = st.panels.borrow_mut();
            let Some(session) = panels
                .iter_mut()
                .filter_map(|p| p.search.as_mut())
                .find(|session| session.id == id)
            else {
                return;
            };
            let (sender, receiver) = mpsc::sync_channel(2);
            session.receiver = Some(receiver);
            submit(Request {
                root: session.root.clone(),
                query: session.query.clone(),
                show_hidden: session.show_hidden,
                cancel: session.cancel.clone(),
                sender,
                result_limit: RESULT_LIMIT,
                directory_limit: DIRECTORY_LIMIT,
            });
        }
        poll(&window, &st, id);
    });
}

pub(super) fn restart(window: &MainWindow, state: &AppState, index: usize) -> bool {
    let query = {
        let panels = state.panels.borrow();
        let Some(panel) = panels.get(index) else {
            return false;
        };
        let tab = &panel.tabs.tabs[panel.tabs.active];
        if tab.ext_filter_on || folder_state_now(state).is_manual(&tab.current_path) {
            None
        } else {
            panel.search.as_ref().map(|s| s.query.clone())
        }
    };
    if query.is_none() {
        clear(state, index);
    }
    if let Some(query) = query {
        start(window, state, index, query);
        true
    } else {
        false
    }
}

pub(super) fn clear(state: &AppState, index: usize) {
    let mut panels = state.panels.borrow_mut();
    if let Some(panel) = panels.get_mut(index)
        && panel.search.take().is_some()
    {
        // A local listing must not preserve a descendant's selection by name.
        panel.replace_rows(Vec::new());
        panel.tabs.tabs[panel.tabs.active].selection_anchor = -1;
        panel.tabs.tabs[panel.tabs.active].cursor = -1;
    }
}

/// Sort/display changes reuse the snapshot; editing the query starts a new scan.
pub(super) fn refresh(window: &MainWindow, state: &AppState, path: &Path) -> bool {
    let index = *state.active_panel.borrow();
    let query = state.filter.borrow().clone();
    let decision = {
        let panels = state.panels.borrow();
        let panel = &panels[index];
        let Some(session) = &panel.search else {
            return false;
        };
        let tab = &panel.tabs.tabs[panel.tabs.active];
        if session.root != path
            || query.is_empty()
            || tab.ext_filter_on
            || folder_state_now(state).is_manual(path)
        {
            0
        } else if query != session.query || tab.show_hidden != session.show_hidden {
            1
        } else {
            2
        }
    };
    match decision {
        0 => {
            clear(state, index);
            return false;
        }
        1 => start(window, state, index, query),
        _ => {
            if !window.get_rows_interaction_busy() {
                reorder(state, index);
            }
            update_panels_ui(window, state);
            request_thumbnails(state);
        }
    }
    true
}

fn poll(window: &MainWindow, state: &AppState, id: u64) {
    let index = state
        .panels
        .borrow()
        .iter()
        .position(|p| p.search.as_ref().is_some_and(|s| s.id == id));
    let Some(index) = index else { return };
    // Publishing rows during a gesture can invalidate row indexes under the
    // pointer. Bounded channels apply backpressure while interaction owns them.
    if !window.get_rows_interaction_busy() {
        let delivery = state.panels.borrow()[index]
            .search
            .as_ref()
            .filter(|s| s.pending.is_empty())
            .and_then(|s| s.receiver.as_ref())
            .and_then(|r| match r.try_recv() {
                Ok(delivery) => Some(delivery),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    warn!("search worker disconnected before completion");
                    Some(Delivery::Finished {
                        skipped: 1,
                        limited: false,
                    })
                }
            });
        match delivery {
            Some(Delivery::Batch(entries)) => {
                state.panels.borrow_mut()[index]
                    .search
                    .as_mut()
                    .unwrap()
                    .pending
                    .extend(entries);
            }
            Some(Delivery::Finished { skipped, limited }) => {
                {
                    let mut panels = state.panels.borrow_mut();
                    let session = panels[index].search.as_mut().unwrap();
                    session.finished = true;
                    session.skipped = skipped;
                    session.limited = limited;
                    session.receiver = None;
                }
                reorder(state, index);
                publish(window, state, index);
                request_imgmeta(state);
                return;
            }
            None => {}
        }
        if !state.panels.borrow()[index]
            .search
            .as_ref()
            .unwrap()
            .pending
            .is_empty()
        {
            append(state, index);
            publish(window, state, index);
        }
    }
    let interval = if !window.get_rows_interaction_busy()
        && !state.panels.borrow()[index]
            .search
            .as_ref()
            .unwrap()
            .pending
            .is_empty()
    {
        Duration::from_millis(16)
    } else {
        POLL_INTERVAL
    };
    let weak = window.as_weak();
    let st = state.clone();
    slint::Timer::single_shot(interval, move || {
        if let Some(window) = weak.upgrade() {
            poll(&window, &st, id);
        }
    });
}

fn append(state: &AppState, index: usize) {
    let config = state.snapshot_config();
    let annotations = annotations_now(state);
    let mut panels = state.panels.borrow_mut();
    let panel = &mut panels[index];
    let tab = &panel.tabs.tabs[panel.tabs.active];
    let context = RowContext {
        lang: config.language,
        now_unix: now_unix(),
        big_icon: tab.preview,
        grid: tab.item_arrangement == ItemArrangement::Grid,
        zoom: tab.zoom,
        compact_icon_rows: config.compact_icon_rows_in_preview,
        item_spacing: config.view_item_spacing,
        group_mode: GroupMode::Mixed,
        annotations: &annotations,
    };
    // Shell icon lookups can be expensive. Yield between entries instead of
    // turning one delivery into a long, uninterrupted UI-thread conversion.
    let started = Instant::now();
    let mut entries = Vec::new();
    let mut rows = Vec::new();
    let session = panel.search.as_mut().unwrap();
    let parent_display = session.root.display().to_string();
    while let Some(entry) = session.pending.pop_front() {
        rows.push(entry_to_row(&entry, &parent_display, &context));
        entries.push(entry);
        if started.elapsed() >= Duration::from_millis(8) {
            break;
        }
    }
    layout_rows(
        &mut rows,
        tab.zoom,
        config.compact_icon_rows_in_preview,
        config.view_item_spacing,
    );
    let clip = state.clipboard.borrow();
    for row in &mut rows {
        row.cut = matches!(clip.op, Some(ClipOp::Cut)) && clip.paths.contains(&row_path(row));
    }
    append_rows(panel, rows);
    panel.search.as_mut().unwrap().entries.extend(entries);
}

/// Extends the dense model without touching existing rows or their selection.
fn append_rows(panel: &Panel, mut rows: Vec<FileRow>) {
    let count = panel.rows_model.row_count();
    let offset = count
        .checked_sub(1)
        .and_then(|i| panel.rows_model.row_data(i))
        .map_or(0.0, |r| r.visual_y + r.visual_h);
    for row in &mut rows {
        row.model_index += count as i32;
        row.visual_y += offset;
    }
    let range = ItemBands::new(count + rows.len(), |i| {
        panel.item_grid.get().map_or_else(
            || {
                if i < count {
                    item_band(&panel.rows_model.row_data(i).unwrap())
                } else {
                    item_band(&rows[i - count])
                }
            },
            |layout| layout.rect(i).band(),
        )
    })
    .render_range(panel.viewport_top.get(), panel.viewport_height.get());
    for row in &mut rows {
        row.rendered = range.contains(&(row.model_index as usize));
    }
    panel.rendered_first.set(range.start);
    panel.rendered_end.set(range.end);
    panel.rows_model.extend(rows);
    panel.bump_rows_revision();
}

fn reorder(state: &AppState, index: usize) {
    let config = state.snapshot_config();
    let mut panels = state.panels.borrow_mut();
    reorder_panel(&mut panels[index], &config);
}

fn reorder_panel(panel: &mut Panel, config: &Config) {
    let tab = &panel.tabs.tabs[panel.tabs.active];
    let cursor = usize::try_from(tab.cursor)
        .ok()
        .and_then(|i| panel.rows_model.row_data(i))
        .map(|r| row_path(&r));
    let anchor = usize::try_from(tab.selection_anchor)
        .ok()
        .and_then(|i| panel.rows_model.row_data(i))
        .map(|r| row_path(&r));
    let mut existing: HashMap<PathBuf, FileRow> = (0..panel.rows_model.row_count())
        .filter_map(|i| panel.rows_model.row_data(i))
        .map(|r| (row_path(&r), r))
        .collect();
    let Some(session) = &mut panel.search else {
        return;
    };
    rfs::sort_with_secondary(
        &mut session.entries,
        tab.sort.column,
        tab.sort.order,
        tab.group_mode,
        tab.sort.secondary,
        mtime_offset(),
    );
    let mut rows: Vec<FileRow> = session
        .entries
        .iter()
        .filter_map(|e| existing.remove(&e.path))
        .collect();
    if tab.group_mode == GroupMode::ColoredFoldersFirst {
        group_colored_folder_rows(&mut rows);
    }
    layout_rows(
        &mut rows,
        tab.zoom,
        config.compact_icon_rows_in_preview,
        config.view_item_spacing,
    );
    let tab = &mut panel.tabs.tabs[panel.tabs.active];
    let previous_cursor = tab.cursor;
    tab.cursor = cursor
        .and_then(|p| rows.iter().position(|r| row_path(r) == p))
        .map_or(-1, |i| i as i32);
    tab.selection_anchor = anchor
        .and_then(|p| rows.iter().position(|r| row_path(r) == p))
        .map_or(-1, |i| i as i32);
    if tab.cursor >= 0 && tab.cursor != previous_cursor {
        tab.scroll_gen += 1;
    }
    panel.replace_rows(rows);
}

fn publish(window: &MainWindow, state: &AppState, index: usize) {
    // Only the receiving view changes. Other repeaters stay untouched.
    let panels = state.panels.borrow();
    let panel = &panels[index];
    let model = window.get_panels();
    if let Some(mut view) = model.row_data(index) {
        view.total_count = panel.rows_model.row_count() as i32;
        view.rows_revision = panel.rows_revision.get();
        view.cursor_row = panel.tabs.tabs[panel.tabs.active].cursor;
        view.scroll_gen = panel.tabs.tabs[panel.tabs.active].scroll_gen;
        model.set_row_data(index, view);
    }
    let text = footer(
        panel,
        state.config.borrow().language,
        count_selected(&*panel.rows_model) as usize,
    );
    window.get_panel_footers().set_row_data(index, text.into());
    drop(panels);
    request_thumbnails(state);
}

pub(super) fn footer(panel: &Panel, lang: Lang, selected: usize) -> String {
    let session = panel.search.as_ref().unwrap();
    let mut text =
        i18n::tr(lang, "search_results").replace("{n}", &panel.rows_model.row_count().to_string());
    if selected > 0 {
        text.push_str(&i18n::tr(lang, "separator_dot"));
        text.push_str(&i18n::footer_selection_text(lang, selected));
    }
    for (enabled, key) in [
        (!session.finished, "search_running"),
        (session.limited, "search_limited"),
    ] {
        if enabled {
            text.push_str(&i18n::tr(lang, "separator_dot"));
            text.push_str(&i18n::tr(lang, key));
        }
    }
    if session.skipped > 0 {
        text.push_str(&i18n::tr(lang, "separator_dot"));
        text.push_str(
            &i18n::tr(lang, "search_skipped").replace("{n}", &session.skipped.to_string()),
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containing_folder_is_only_offered_for_one_descendant_result() {
        let root = PathBuf::from("folder01");
        for item in ["report.txt", "folder02"] {
            assert!(containing_folder(&root, &[root.join(item)]).is_none());
        }
        for item in ["folder02/report.txt", "folder02/folder03"] {
            assert_eq!(
                containing_folder(&root, &[root.join(item)]),
                Some(root.join("folder02"))
            );
        }
        assert!(containing_folder(&root, &[]).is_none());
        assert!(containing_folder(&root, &[PathBuf::from("folder04/report.txt")]).is_none());
        #[cfg(windows)]
        {
            let same_root = PathBuf::from("FOLDER01");
            assert!(containing_folder(&same_root, &[root.join("report.txt")]).is_none());
            assert_eq!(
                containing_folder(&same_root, &[root.join("folder02/report.txt")]),
                Some(root.join("folder02"))
            );
        }
        assert!(
            containing_folder(
                &root,
                &[
                    root.join("folder02/report.txt"),
                    root.join("folder03/report.txt")
                ]
            )
            .is_none()
        );
    }

    struct Tree(PathBuf);

    impl Tree {
        fn new() -> Self {
            static SERIAL: AtomicU64 = AtomicU64::new(0);
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/search-tests")
                .join(format!(
                    "{}-{}",
                    std::process::id(),
                    SERIAL.fetch_add(1, Ordering::Relaxed)
                ));
            std::fs::create_dir_all(&root).unwrap();
            Self(root.canonicalize().unwrap())
        }

        fn file(&self, relative: &str) {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"test").unwrap();
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target")
                .canonicalize()
                .unwrap();
            assert!(self.0.starts_with(workspace));
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn collect(
        root: &Path,
        query: &str,
        hidden: bool,
        limit: usize,
        dir_limit: usize,
    ) -> (Vec<Entry>, usize, bool) {
        let (sender, receiver) = mpsc::sync_channel(2);
        let request = Request {
            root: root.into(),
            query: query.into(),
            show_hidden: hidden,
            cancel: Arc::new(AtomicBool::new(false)),
            sender,
            result_limit: limit,
            directory_limit: dir_limit,
        };
        let thread = std::thread::spawn(move || walk(request));
        let mut entries = Vec::new();
        let result = loop {
            match receiver.recv_timeout(Duration::from_secs(10)).unwrap() {
                Delivery::Batch(batch) => entries.extend(batch),
                Delivery::Finished { skipped, limited } => break (entries, skipped, limited),
            }
        };
        thread.join().unwrap();
        result
    }

    #[test]
    fn recursive_search_matches_names_and_keeps_homonyms_distinct() {
        let tree = Tree::new();
        tree.file("folder01/report.txt");
        tree.file("folder02/report.txt");
        tree.file("folder02/nested/REPORT.png");
        tree.file("report_folder/unrelated.txt");
        let (entries, skipped, limited) = collect(&tree.0, "report", false, 100, 100);
        let paths: HashSet<_> = entries.into_iter().map(|entry| entry.path).collect();
        assert_eq!(
            paths,
            [
                "folder01/report.txt",
                "folder02/report.txt",
                "folder02/nested/REPORT.png",
                "report_folder"
            ]
            .map(|path| tree.0.join(path))
            .into_iter()
            .collect()
        );
        assert_eq!(skipped, 0);
        assert!(!limited);
    }

    #[test]
    fn recursive_search_respects_hidden_subtrees_and_reports_unreadable_roots() {
        let tree = Tree::new();
        tree.file("visible.txt");
        tree.file(".hidden/visible.txt");
        assert_eq!(collect(&tree.0, ".txt", false, 100, 100).0.len(), 1);
        assert_eq!(collect(&tree.0, ".txt", true, 100, 100).0.len(), 2);
        let (entries, skipped, limited) = collect(&tree.0.join("missing"), "txt", false, 100, 100);
        assert!(entries.is_empty());
        assert_eq!(skipped, 1);
        assert!(!limited);
    }

    #[test]
    fn recursive_search_reports_limits_instead_of_silently_truncating() {
        let tree = Tree::new();
        for index in 0..6 {
            tree.file(&format!("item{index}.txt"));
        }
        tree.file("folder01/item.txt");
        let (entries, _, limited) = collect(&tree.0, ".txt", false, 3, 100);
        assert_eq!(entries.len(), 3);
        assert!(limited);
        assert!(collect(&tree.0, "no-match", false, 100, 1).2);
    }

    #[test]
    fn recursive_search_cancels_while_delivery_is_backpressured() {
        let tree = Tree::new();
        for index in 0..BATCH_SIZE * 3 {
            tree.file(&format!("item{index}.txt"));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(1);
        let request = Request {
            root: tree.0.clone(),
            query: ".txt".into(),
            show_hidden: false,
            cancel: cancel.clone(),
            sender,
            result_limit: RESULT_LIMIT,
            directory_limit: DIRECTORY_LIMIT,
        };
        let thread = std::thread::spawn(move || walk(request));
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(10)).unwrap(),
            Delivery::Batch(_)
        ));
        cancel.store(true, Ordering::Relaxed);
        drop(receiver);
        thread.join().unwrap();
    }

    #[test]
    fn recursive_search_directory_reader_stops_between_entries() {
        let tree = Tree::new();
        for index in 0..10 {
            tree.file(&format!("item{index}.txt"));
        }
        let visited = Cell::new(0);
        rfs::visit_dir_while(
            &tree.0,
            false,
            || visited.get() == 3,
            |_| visited.set(visited.get() + 1),
        )
        .unwrap();
        assert_eq!(visited.get(), 3);
    }

    #[test]
    fn recursive_search_does_not_follow_directory_links() {
        let tree = Tree::new();
        tree.file("folder01/report.txt");
        let link = tree.0.join("folder01/loop");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&tree.0, &link).unwrap();
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let made = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&tree.0)
                .creation_flags(0x0800_0000)
                .output()
                .unwrap();
            assert!(made.status.success(), "test junction creation failed");
        }
        let (entries, skipped, limited) = collect(&tree.0, "report", false, 20, 20);
        #[cfg(windows)]
        std::fs::remove_dir(&link).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(skipped, 0);
        assert!(!limited);
    }

    fn session(root: &Path, entries: Vec<Entry>) -> Session {
        Session {
            id: 1,
            root: root.into(),
            query: "report".into(),
            show_hidden: false,
            cancel: Arc::new(AtomicBool::new(false)),
            receiver: None,
            entries,
            pending: VecDeque::new(),
            finished: true,
            skipped: 0,
            limited: false,
        }
    }

    #[test]
    fn recursive_search_batches_preserve_selection_and_virtualization_in_both_layouts() {
        for grid in [None, Some(GridLayout::fit(480.0, 80.0, 13.0, 4.0, 8.0))] {
            let panel = Panel::with_mode(
                PathBuf::from("folder01"),
                columns::default_columns(),
                0,
                true,
            );
            panel.item_grid.set(grid);
            panel.viewport_height.set(300.0);
            let initial_model = panel.rows_model.clone();
            for batch in 0..5 {
                let mut rows: Vec<_> = (0..200)
                    .map(|index| FileRow {
                        name: format!("item{}", batch * 200 + index).into(),
                        preview_capable: index % 2 == 0,
                        selected: batch == 0 && index == 0,
                        ..Default::default()
                    })
                    .collect();
                layout_rows(&mut rows, THUMB_DEFAULT_ZOOM, true, 4);
                append_rows(&panel, rows);
            }
            assert!(Rc::ptr_eq(&panel.rows_model, &initial_model));
            assert!(panel.rows_model.row_data(0).unwrap().selected);
            assert_eq!(count_selected(&*panel.rows_model), 1);
            let range = arranged_model_geometry(&*panel.rows_model, grid).render_range(0.0, 300.0);
            assert_eq!(panel.rendered_rows_model.row_count(), range.len());
            assert!(range.len() < 100);
            let mut next_y = 0.0;
            for (index, row) in panel.rows_model.iter().enumerate() {
                assert_eq!(row.model_index, index as i32);
                assert_eq!(row.visual_y, next_y);
                assert_eq!(row.rendered, range.contains(&index));
                next_y += row.visual_h;
            }
        }
    }

    #[test]
    fn recursive_search_rows_preserve_descendant_and_share_paths() {
        let annotations = favnyr_core::annotations::AnnotationStore::default();
        let context = RowContext {
            lang: Lang::En,
            now_unix: 0,
            big_icon: false,
            grid: false,
            zoom: LIST_DEFAULT_ZOOM,
            compact_icon_rows: false,
            item_spacing: 0,
            group_mode: GroupMode::Mixed,
            annotations: &annotations,
        };
        // A slice rather than an array literal: the second case is compiled
        // on Windows only, which would otherwise leave a one-element array
        // here and read as a loop that cannot loop.
        let cases: &[(&str, &str, &str)] = &[
            ("folder01", "folder01/folder02/folder03", "folder03"),
            #[cfg(windows)]
            (r"\\device01", r"\\device01\share01", "share01"),
        ];
        for &(root, path, name) in cases {
            let entry = Entry {
                name: name.into(),
                path: PathBuf::from(path),
                size_bytes: None,
                mtime_unix: None,
                created_unix: None,
                is_dir: true,
                kind: FileKind::Folder,
                hidden: false,
                is_symlink: false,
                is_share: false,
                executable: false,
            };
            assert_eq!(row_path(&entry_to_row(&entry, root, &context)), entry.path);
        }
    }

    #[test]
    fn recursive_search_sort_and_cut_use_full_paths_for_homonyms() {
        let root = PathBuf::from("folder01");
        let paths = [
            root.join("folder02/report.txt"),
            root.join("folder03/report.txt"),
        ];
        let mut panel = Panel::with_mode(root.clone(), columns::default_columns(), 0, false);
        let entries = paths
            .iter()
            .enumerate()
            .map(|(index, path)| Entry {
                name: "report.txt".into(),
                path: path.clone(),
                size_bytes: Some(index as u64),
                mtime_unix: None,
                created_unix: None,
                is_dir: false,
                kind: FileKind::File,
                hidden: false,
                is_symlink: false,
                is_share: false,
                executable: false,
            })
            .collect();
        panel.search = Some(session(&root, entries));
        panel.replace_rows(
            paths
                .iter()
                .enumerate()
                .map(|(index, path)| FileRow {
                    name: "report.txt".into(),
                    path: path.parent().unwrap().to_string_lossy().into_owned().into(),
                    selected: index == 0,
                    ..Default::default()
                })
                .collect(),
        );
        panel.tabs.tabs[0].sort = SortState {
            column: SortColumn::Size,
            order: SortOrder::Desc,
            secondary: None,
        };
        panel.tabs.tabs[0].cursor = 0;
        panel.tabs.tabs[0].selection_anchor = 0;
        reorder_panel(&mut panel, &Config::default());
        assert_eq!(row_path(&panel.rows_model.row_data(0).unwrap()), paths[1]);
        assert!(!panel.rows_model.row_data(0).unwrap().selected);
        assert!(panel.rows_model.row_data(1).unwrap().selected);
        assert_eq!(panel.tabs.tabs[0].cursor, 1);
        assert_eq!(panel.tabs.tabs[0].selection_anchor, 1);
        apply_cut_marks(&*panel.rows_model, &HashSet::from([paths[0].clone()]));
        assert!(!panel.rows_model.row_data(0).unwrap().cut);
        assert!(panel.rows_model.row_data(1).unwrap().cut);
        let cancellation = panel.search.as_ref().unwrap().cancel.clone();
        panel.search = None;
        assert!(cancellation.load(Ordering::Relaxed));
    }
}
