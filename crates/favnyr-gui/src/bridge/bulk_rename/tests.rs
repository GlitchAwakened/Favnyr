use super::*;

slint::slint! {
    import { BulkRenameDialog, BulkRenameInputs, BulkRenamePreviewRow, BulkRenameRow, BulkRenamePreview } from "src/ui/bulk_rename.slint";
    export { BulkRenameInputs }
    export component RenameHarness inherits Window {
        width: 840px; height: 760px;
        in-out property <bool> open: true;
        in-out property <bool> finished;
        in-out property <bool> running;
        in-out property <bool> blocked;
        in-out property <bool> ready;
        out property <int> focused: dialog.focused-index;
        in-out property <int> edits;
        in-out property <int> submissions;
        in-out property <int> cancellations;
        dialog := BulkRenameDialog {
            open: root.open; visible: root.open; ready: root.ready;
            running: root.running; finished: root.finished; blocked: root.blocked;
            edited => { root.edits += 1; }
            submit => { root.submissions += 1; }
            cancel => { root.cancellations += 1; }
        }
    }
    export component RenamePreviewHarness inherits Window {
        width: 800px; height: 600px;
        in property <BulkRenameRow> entry;
        in property <[BulkRenameRow]> rows;
        in property <length> row-width: 760px;
        out property <length> row-height: row.height;
        out property <length> content-height: preview.content-height;
        in-out property <length> scroll-y <=> preview.content-y;
        public function seek(fraction: float) {
            preview.scroll-to(-(preview.content-height - preview.height) * fraction);
        }
        row := BulkRenamePreviewRow {
            x: 0px; y: 0px; width: root.row-width; entry: root.entry;
        }
        preview := BulkRenamePreview {
            x: 0px; y: 300px; width: root.row-width; height: 280px; rows: root.rows;
        }
    }
}

fn press(ui: &RenameHarness, key: slint::platform::Key) {
    let text: SharedString = key.into();
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
}

fn settle() {
    slint::platform::update_timers_and_animations();
    for _ in 0..2 {
        std::thread::sleep(Duration::from_millis(10));
        slint::platform::update_timers_and_animations();
    }
}

struct ObservedPreviewRows {
    rows: Vec<BulkRenameRow>,
    reads: RefCell<HashSet<usize>>,
    notify: slint::ModelNotify,
}

impl slint::Model for ObservedPreviewRows {
    type Data = BulkRenameRow;

    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn row_data(&self, index: usize) -> Option<Self::Data> {
        self.reads.borrow_mut().insert(index);
        self.rows.get(index).cloned()
    }

    fn model_tracker(&self) -> &dyn slint::ModelTracker {
        &self.notify
    }
}

#[test]
fn batch_dialog_keeps_keyboard_focus_and_skips_inactive_options() {
    crate::test_support::run_with_windows(|windows| {
        use slint::platform::Key;
        let ui = RenameHarness::new().unwrap();
        let inputs = ui.global::<BulkRenameInputs>();
        inputs.invoke_reset();
        ui.window().set_size(slint::LogicalSize::new(840., 760.));
        ui.show().unwrap();
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
        settle();
        assert_eq!(ui.get_focused(), -1);
        press(&ui, Key::Tab);
        assert_eq!(ui.get_focused(), 0);
        press(&ui, Key::Tab);
        assert_eq!(ui.get_focused(), 2);
        press(&ui, Key::Tab);
        assert_eq!(ui.get_focused(), 7);
        press(&ui, Key::Tab);
        assert_eq!(ui.get_focused(), 10);
        press(&ui, Key::Tab);
        assert_eq!(ui.get_focused(), 16);
        press(&ui, Key::Tab);
        assert_eq!(ui.get_focused(), 17);
        press(&ui, Key::Tab);
        assert_eq!(ui.get_focused(), 19);
        press(&ui, Key::Tab);
        assert_eq!(ui.get_focused(), 0);
        press(&ui, Key::Space);
        assert!(inputs.get_common_enabled());
        assert_eq!(ui.get_edits(), 1);
        press(&ui, Key::Tab);
        assert_eq!(ui.get_focused(), 1);
        press(&ui, Key::Backtab);
        assert_eq!(ui.get_focused(), 0);
        inputs.set_replace_enabled(true);
        inputs.set_affixes_enabled(true);
        inputs.set_numbering_enabled(true);
        inputs.set_extension_enabled(true);
        settle();
        for expected in 1..=19 {
            press(&ui, Key::Tab);
            assert_eq!(ui.get_focused(), expected);
        }
        press(&ui, Key::Tab);
        assert_eq!(ui.get_focused(), 0);
        ui.set_running(true);
        let adapter = windows.borrow()[0].clone();
        let mut pixels = vec![slint::Rgb8Pixel::default(); 840 * 760];
        adapter.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 840);
        });
        settle();
        press(&ui, Key::Return);
        assert_eq!(
            ui.get_cancellations(),
            1,
            "focus index: {}",
            ui.get_focused()
        );
        assert_eq!(ui.get_focused(), 19);
        press(&ui, Key::Tab);
        assert_eq!(ui.get_focused(), 19);
        press(&ui, Key::Return);
        assert_eq!(ui.get_cancellations(), 2);
        ui.set_running(false);
        ui.set_finished(true);
        settle();
        press(&ui, Key::Backtab);
        assert_eq!(ui.get_focused(), 19);
        assert_eq!(ui.get_submissions(), 0);
        ui.set_open(false);
        settle();
        inputs.invoke_reset();
        assert!(!inputs.get_common_enabled());
        assert!(!inputs.get_replace_enabled());
        assert!(!inputs.get_affixes_enabled());
        assert!(!inputs.get_numbering_enabled());
        assert!(!inputs.get_extension_enabled());
        assert!(inputs.get_case_sensitive());
        assert!(inputs.get_replace_all());
        assert_eq!(inputs.get_digits(), 2);
        assert_eq!(inputs.get_position(), 1);
        assert_eq!(inputs.get_separator(), "_");
        ui.set_finished(false);
        ui.set_open(true);
        settle();
        assert_eq!(ui.get_focused(), -1);
        press(&ui, Key::Tab);
        assert_eq!(ui.get_focused(), 0);
    });
}

#[test]
fn batch_preview_wraps_names_paths_and_errors_and_scrolls_large_batches() {
    crate::test_support::run_with_windows(|windows| {
        let ui = RenamePreviewHarness::new().unwrap();
        ui.window().set_size(slint::LogicalSize::new(800., 600.));
        let short = BulkRenameRow {
            original: "File01.txt".into(),
            proposed: "File02.txt".into(),
            parent_path: "Folder01".into(),
            ..Default::default()
        };
        ui.set_entry(short.clone());
        ui.show().unwrap();
        let adapter = windows.borrow()[0].clone();
        let mut pixels = vec![slint::Rgb8Pixel::default(); 800 * 600];
        let mut draw = || {
            for _ in 0..3 {
                ui.window().request_redraw();
                adapter.draw_if_needed(|renderer| {
                    renderer.render(&mut pixels, 800);
                });
                settle();
            }
        };
        draw();
        let short_height = ui.get_row_height();
        let long = BulkRenameRow {
            original: format!("{}.txt", "File01".repeat(25)).into(),
            proposed: format!("{}.txt", "File02".repeat(25)).into(),
            parent_path: "Folder01/".repeat(40).into(),
            status: "Message01 ".repeat(25).into(),
            error: true,
        };
        ui.set_entry(long.clone());
        draw();
        let wide_height = ui.get_row_height();
        assert!(wide_height > short_height + 30.);
        ui.set_row_width(360.);
        draw();
        assert!(ui.get_row_height() > wide_height + 30.);

        let rows = (0..2000)
            .map(|index| {
                if index % 3 == 0 {
                    long.clone()
                } else {
                    short.clone()
                }
            })
            .collect::<Vec<_>>();
        let model = Rc::new(ObservedPreviewRows {
            rows,
            reads: RefCell::default(),
            notify: slint::ModelNotify::default(),
        });
        ui.set_rows(model.clone().into());
        draw();
        assert!(ui.get_content_height() > 2000. * short_height);
        assert!(model.reads.borrow().len() < 100);
        for fraction in [0.25, 0.75, 1., 0.] {
            model.reads.borrow_mut().clear();
            ui.invoke_seek(fraction);
            draw();
            assert!(ui.get_scroll_y().is_finite());
            assert!(ui.get_scroll_y() <= 0.);
            assert!(model.reads.borrow().len() < 100);
            if fraction == 1. {
                assert!(
                    model.reads.borrow().contains(&1999),
                    "read rows: {:?}, offset: {}, extent: {}",
                    model.reads.borrow(),
                    ui.get_scroll_y(),
                    ui.get_content_height()
                );
            }
        }
        ui.set_rows(Rc::new(VecModel::from(vec![short])).into());
        draw();
        assert_eq!(ui.get_scroll_y(), 0.);
    });
}

#[test]
fn batch_paths_keep_identity_through_cycles_without_temporary_shortcuts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("Folder01");
    let a = root.join("Folder02");
    let b = root.join("Folder03");
    let ta = root.join("Folder04");
    let tb = root.join("Folder05");
    let steps = vec![
        (a.clone(), ta.clone()),
        (b.clone(), tb.clone()),
        (ta, b.clone()),
        (tb, a.clone()),
    ];
    let mut shortcuts = std::collections::BTreeMap::new();
    redirect_shortcuts(&mut shortcuts, &[a.clone(), b.clone()], &steps);
    assert_eq!(shortcuts.len(), 2);
    assert_eq!(
        shortcuts[&a.to_string_lossy().into_owned()],
        b.to_string_lossy()
    );
    assert_eq!(
        shortcuts[&b.to_string_lossy().into_owned()],
        a.to_string_lossy()
    );
    let locations = HashMap::from([
        (reservation_key(&a), b.clone()),
        (reservation_key(&b), a.clone()),
    ]);
    assert_eq!(
        final_location(&a.join("File01.txt"), &locations),
        b.join("File01.txt")
    );
    assert_eq!(final_location(&b, &locations), a);
    let mut annotations = favnyr_core::annotations::AnnotationStore::default();
    annotations.set_note(&a, "Note01");
    annotations.set_note(&b, "Note02");
    let mut folders = FolderStateStore::default();
    folders.enable(&root, vec!["Folder02".into(), "Folder03".into()]);
    for (from, to) in steps {
        annotations.rename(&from, &to);
        folders.relocate(&from, &to);
    }
    assert_eq!(annotations.note_of(&a), "Note02");
    assert_eq!(annotations.note_of(&b), "Note01");
    assert_eq!(folders.order(&root).unwrap(), ["Folder03", "Folder02"]);
}

#[test]
fn disabled_batch_controls_do_not_change_names() {
    let options = options(crate::BulkRenameOptions {
        common_name: "File01".into(),
        prefix: "File02".into(),
        find: "File03".into(),
        extension: "png".into(),
        start: 1,
        increment: 1,
        digits: 3,
        ..Default::default()
    });
    assert!(options.common.is_none());
    assert!(options.replacement.is_none());
    assert!(options.affixes.is_none());
    assert!(options.numbering.is_none());
    assert!(options.extension.is_none());
}
