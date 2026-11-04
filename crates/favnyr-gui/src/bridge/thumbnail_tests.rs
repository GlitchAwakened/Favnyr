//! Preview eligibility, bounded failures and optional-provider geometry.

use super::*;

fn image() -> Image {
    image_from_thumb(&Thumbnail {
        width: 1,
        height: 1,
        rgba: vec![12, 34, 56, 255],
    })
}

fn panel() -> Panel {
    let mut panel = Panel::with_mode(PathBuf::from("folder01"), Vec::new(), 0, true);
    panel.tabs.tabs[0].zoom = THUMB_DEFAULT_ZOOM;
    let mut rows: Vec<_> = (0..100)
        .map(|index| FileRow {
            name: format!("File{index:02}.xcf").into(),
            path: "folder01".into(),
            ext: "xcf".into(),
            kind: FileKind::Image.as_i32(),
            selected: index == 50,
            cut: index == 51,
            comment: "Note01".into(),
            ..Default::default()
        })
        .collect();
    layout_rows(&mut rows, THUMB_DEFAULT_ZOOM, true, 0);
    panel.viewport_top.set(rows[50].visual_y + 7.0);
    panel.viewport_height.set(140.0);
    panel.replace_rows(rows);
    panel
}

#[test]
fn unsupported_images_remain_images_without_reserving_preview_space() {
    let kind = rfs::classify_kind(Some("xcf"), false);
    assert_eq!(kind, FileKind::Image);
    assert!(thumbnail_kind_for_row(kind.as_i32(), "xcf").is_some());
    assert!(!reserves_preview_space(kind.as_i32(), "xcf"));
    assert_eq!(effective_row_height(false, THUMB_DEFAULT_ZOOM, true), 28.0);
    assert_eq!(effective_row_height(false, THUMB_DEFAULT_ZOOM, false), 76.0);
    for extension in ["png", "JPG", "svg", "psd", "afphoto"] {
        assert!(reserves_preview_space(kind.as_i32(), extension));
    }
    for (kind, extension) in [
        (FileKind::Document, "pdf"),
        (FileKind::Audio, "mp3"),
        (FileKind::Audio, "flac"),
        (FileKind::Video, "mp4"),
        (FileKind::File, "stl"),
        (FileKind::File, "blend1"),
        (FileKind::File, "obj"),
        (FileKind::File, "glb"),
        (FileKind::File, "gltf"),
    ] {
        assert!(reserves_preview_space(kind.as_i32(), extension));
    }
}

#[test]
fn rebuilding_rows_preserves_cached_preview_geometry_and_respects_invalidation() {
    let mut entry = Entry {
        name: "File01.xcf".into(),
        path: PathBuf::from("folder01").join("File01.xcf"),
        size_bytes: Some(16),
        mtime_unix: None,
        created_unix: None,
        is_dir: false,
        kind: FileKind::Image,
        hidden: false,
        is_symlink: false,
        content_remote: false,
        is_share: false,
        executable: false,
    };
    let cache = RefCell::new(ThumbLru::new(8));
    let annotations = favnyr_core::annotations::AnnotationStore::default();
    let context = RowContext {
        lang: Lang::En,
        now_unix: 0,
        big_icon: false,
        grid: false,
        zoom: THUMB_DEFAULT_ZOOM,
        compact_icon_rows: true,
        item_spacing: 0,
        group_mode: GroupMode::Mixed,
        annotations: &annotations,
        thumb_cache: Some(&cache),
    };
    let build = |entry: &Entry| {
        entries_to_rows(std::slice::from_ref(entry), "folder01", &context).remove(0)
    };
    assert_eq!(build(&entry).visual_h, 28.0);
    cache
        .borrow_mut()
        .put(entry.path.to_string_lossy().into_owned(), image());
    assert_eq!(build(&entry).visual_h, 76.0);
    entry.content_remote = true;
    assert_eq!(build(&entry).visual_h, 28.0);
    entry.content_remote = false;
    cache.borrow_mut().remove_path(&entry.path);
    cache
        .borrow_mut()
        .mark_failed(entry.path.to_string_lossy().into_owned());
    assert_eq!(build(&entry).visual_h, 28.0);
    cache.borrow_mut().remove_path(&entry.path);
    entry.name = "File02.png".into();
    entry.path = PathBuf::from("folder01").join(&entry.name);
    let row = build(&entry);
    assert_eq!(row.visual_h, 76.0);
    assert_eq!(row.thumbnail.size().width, 0);
    assert!(cache.borrow().failed.is_empty());
}

#[test]
fn failure_cache_is_bounded_and_does_not_evict_successful_textures() {
    let mut cache = ThumbLru::new(2);
    cache.put("File01.png".into(), image());
    for key in ["File02.xcf", "File03.xcf", "File03.xcf", "File04.xcf"] {
        cache.mark_failed(key.into());
    }
    assert_eq!(cache.failed.len(), 2);
    assert_eq!(cache.failed_order.len(), 2);
    assert!(!cache.failed.contains("File02.xcf"));
    assert!(cache.failed.contains("File03.xcf"));
    assert!(cache.failed.contains("File04.xcf"));
    assert!(cache.get("File01.png").is_some());
    cache.put("File03.xcf".into(), image());
    assert!(!cache.failed.contains("File03.xcf"));
    cache.remove_path(Path::new("File04.xcf"));
    assert!(cache.failed.is_empty());
    assert!(cache.failed_order.is_empty());
    assert!(cache.contains("File03.xcf"));
}

#[test]
fn failed_and_completed_previews_share_path_invalidation_rules() {
    let mut cache = ThumbLru::new(8);
    for extension in ["xcf", "png"] {
        let source = PathBuf::from("folder01").join(format!("File01.{extension}"));
        let target = PathBuf::from("folder01").join(format!("File02.{extension}"));
        cache.mark_failed(source.to_string_lossy().into_owned());
        cache.mark_failed(target.to_string_lossy().into_owned());
        for path in [&source, &target] {
            cache.remove_path(path);
            assert!(!cache.failed.contains(path.to_string_lossy().as_ref()));
            assert!(!cache.contains(path.to_string_lossy().as_ref()));
        }
        cache.put(target.to_string_lossy().into_owned(), image());
        cache.remove_path(&target);
        assert!(!cache.contains(target.to_string_lossy().as_ref()));
    }
}

#[test]
fn optional_previews_relayout_once_and_preserve_anchor_selection_and_hit_tests() {
    let panel = panel();
    let config = Config {
        compact_icon_rows_in_preview: true,
        ..Default::default()
    };
    let cache = RefCell::new(ThumbLru::new(8));
    for index in [20, 50] {
        let row = panel.rows_model.row_data(index).unwrap();
        cache
            .borrow_mut()
            .put(row_path(&row).to_string_lossy().into_owned(), image());
    }
    let revision = panel.rows_revision.get();
    let anchor = model_geometry(&*panel.rows_model)
        .capture_anchor(panel.viewport_top.get(), 140.0, 0.0, None)
        .unwrap();
    promote_panel_previews(&panel, &cache, &config);
    assert_eq!(panel.rows_revision.get(), revision.wrapping_add(1));
    let top = panel.viewport_top.get();
    let row = panel.rows_model.row_data(anchor.index).unwrap();
    assert!((row.visual_y + row.visual_h * anchor.fraction - top).abs() < 0.001);
    assert_eq!(panel.thumbnail_restore_top.get(), Some(top));
    assert!(row.selected);
    assert!(panel.rows_model.row_data(51).unwrap().cut);
    let mut y = 0.0;
    for (index, row) in panel.rows_model.iter().enumerate() {
        assert_eq!(row.visual_y, y);
        assert_eq!(row.model_index, index as i32);
        assert_eq!(row.comment.as_str(), "Note01");
        assert_eq!(
            row.visual_h,
            if [20, 50].contains(&index) {
                76.0
            } else {
                28.0
            }
        );
        assert_eq!(
            row_index_at_content_y(&panel.rows_model, y + 0.5),
            index as i32
        );
        y += row.visual_h;
    }
    assert!(panel.rendered_rows_model.row_count() < 30);
    promote_panel_previews(&panel, &cache, &config);
    assert_eq!(panel.rows_revision.get(), revision.wrapping_add(1));
    panel.reset_rows_viewport();
    assert!(panel.thumbnail_restore_top.get().is_none());
}

#[test]
fn failures_remote_content_and_shortcuts_never_trigger_optional_relayout() {
    let panel = panel();
    let cache = RefCell::new(ThumbLru::new(8));
    let revision = panel.rows_revision.get();
    let mut row = panel.rows_model.row_data(50).unwrap();
    let key = row_path(&row).to_string_lossy().into_owned();
    cache.borrow_mut().mark_failed(key.clone());
    promote_panel_previews(&panel, &cache, &Config::default());
    assert_eq!(panel.rows_revision.get(), revision);
    cache.borrow_mut().put(key, image());
    row.content_remote = true;
    panel.rows_model.set_row_data(50, row.clone());
    promote_panel_previews(&panel, &cache, &Config::default());
    assert_eq!(panel.rows_revision.get(), revision);
    row.content_remote = false;
    row.kind = FileKind::File.as_i32();
    row.ext = "lnk".into();
    panel.rows_model.set_row_data(50, row);
    promote_panel_previews(&panel, &cache, &Config::default());
    assert_eq!(panel.rows_revision.get(), revision);
}

#[test]
fn grid_and_noncompact_modes_keep_their_existing_geometry() {
    for (grid, preview, compact) in [
        (true, true, true),
        (false, true, false),
        (false, false, true),
    ] {
        let mut panel = panel();
        panel.tabs.tabs[0].item_arrangement = if grid {
            ItemArrangement::Grid
        } else {
            ItemArrangement::List
        };
        panel.tabs.tabs[0].preview = preview;
        let before: Vec<_> = panel
            .rows_model
            .iter()
            .map(|row| (row.visual_y, row.visual_h))
            .collect();
        let revision = panel.rows_revision.get();
        let cache = RefCell::new(ThumbLru::new(8));
        let row = panel.rows_model.row_data(50).unwrap();
        cache
            .borrow_mut()
            .put(row_path(&row).to_string_lossy().into_owned(), image());
        promote_panel_previews(
            &panel,
            &cache,
            &Config {
                compact_icon_rows_in_preview: compact,
                ..Default::default()
            },
        );
        assert!(panel.rows_model.row_data(50).unwrap().preview_capable);
        assert_eq!(panel.rows_revision.get(), revision);
        assert_eq!(
            panel
                .rows_model
                .iter()
                .map(|row| (row.visual_y, row.visual_h))
                .collect::<Vec<_>>(),
            before
        );
        assert!(panel.thumbnail_restore_top.get().is_none());
    }
}

#[test]
fn optional_preview_height_changes_restore_the_live_viewport_without_panel_rebuild() {
    crate::test_support::run_with_windows(|windows| {
        let panel = Rc::new(panel());
        let ui = MainWindow::new().unwrap();
        let views = Rc::new(VecModel::from(vec![PanelView {
            rows: ModelRc::from(panel.rows_model.clone()),
            rendered_rows: panel.rendered_rows_model.clone(),
            preview_mode: true,
            content_width: 500.0,
            col_name_w: 400.0,
            ..Default::default()
        }]));
        ui.set_panels(ModelRc::from(views.clone()));
        let applied = Rc::new(Cell::new(None));
        let observed = applied.clone();
        let target = panel.clone();
        ui.on_panel_viewport_changed(move |_, top, height, _| {
            let top = if let Some(restored) = target.thumbnail_restore_top.take() {
                observed.set(Some(restored));
                restored
            } else {
                top
            };
            target.viewport_top.set(top);
            if height > 0.0 {
                target.viewport_height.set(height);
            }
            top
        });
        ui.show().unwrap();
        ui.window().set_size(slint::LogicalSize::new(900.0, 800.0));
        let adapter = windows.borrow()[0].clone();
        let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 800];
        let mut render = || {
            ui.window().request_redraw();
            adapter.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, 900);
            });
        };
        render();
        let mut view = views.row_data(0).unwrap();
        view.cursor_row = 70;
        view.scroll_gen = 1;
        views.set_row_data(0, view);
        render();
        let old_top = panel.viewport_top.get();
        assert!(old_top > 0.0);
        let cache = RefCell::new(ThumbLru::new(8));
        let row = panel.rows_model.row_data(20).unwrap();
        cache
            .borrow_mut()
            .put(row_path(&row).to_string_lossy().into_owned(), image());
        promote_panel_previews(
            &panel,
            &cache,
            &Config {
                compact_icon_rows_in_preview: true,
                ..Default::default()
            },
        );
        render();
        assert_eq!(applied.get(), Some(old_top + 48.0));
        assert_eq!(panel.viewport_top.get(), old_top + 48.0);
        assert!(panel.thumbnail_restore_top.get().is_none());
        assert_eq!(views.row_data(0).unwrap().rows_revision, 0);
        ui.hide().unwrap();
    });
}
