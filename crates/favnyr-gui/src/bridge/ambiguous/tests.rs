use super::*;

fn row(name: &str) -> FileRow {
    FileRow {
        name: name.into(),
        path: "folder01".into(),
        ext: ops::ext_of(name).into(),
        kind: rfs::classify_kind(Some(&ops::ext_of(name)), false).as_i32(),
        rendered: true,
        selected: true,
        comment: "Note01".into(),
        ..Default::default()
    }
}

fn location(panel: usize) -> ThumbLocation {
    ThumbLocation {
        panel,
        row: 0,
        row_count: 1,
    }
}

fn resolution(kind: FileKind) -> Resolution {
    Resolution {
        kind,
        stamp: Stamp {
            size: Some(16),
            modified: Some(32),
        },
        remote: false,
    }
}

fn panel(preview: bool) -> Panel {
    let panel = Panel::with_mode("folder01".into(), columns::default_columns(), 0, preview);
    let mut rows = vec![row("File01.ts")];
    layout_rows(&mut rows, THUMB_DEFAULT_ZOOM, true, 0);
    panel.replace_rows(rows);
    panel
}

fn image() -> Image {
    Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(2, 2))
}

#[test]
fn probes_are_visible_only_and_generic_results_do_not_become_thumbnail_failures() {
    let cache = RefCell::new(ThumbLru::new(4));
    let mut requests = HashMap::new();
    for name in ["File01.txt", "File01.mp4", "File01.obj", "File01.xcf"] {
        assert!(!prepare(
            &cache,
            &mut row(name),
            true,
            location(0),
            &mut requests
        ));
    }
    let mut source = row("File01.ts");
    assert!(prepare(
        &cache,
        &mut source,
        false,
        location(0),
        &mut requests
    ));
    assert!(requests.is_empty());
    assert!(prepare(
        &cache,
        &mut source,
        true,
        location(0),
        &mut requests
    ));
    assert_eq!(requests.len(), 1);
    let request = requests.remove(&row_path(&source)).unwrap();
    assert_eq!(request.job.probe, Some(Rule::TransportStream));
    assert_eq!(request.job.kind, FileKind::File);
    cache
        .borrow_mut()
        .kinds
        .insert(row_path(&source), resolution(FileKind::File));
    assert!(prepare(
        &cache,
        &mut source,
        true,
        location(0),
        &mut requests
    ));
    assert!(requests.is_empty());
    assert!(cache.borrow().failed.is_empty());
}

#[test]
fn worker_reads_recognize_streams_and_keep_text_missing_and_short_files_generic() {
    let directory = crate::test_support::TestFolder::new();
    let path = directory.0.join("File01.ts");
    std::fs::write(&path, b"export const value = 'File01';\n").unwrap();
    let text = probe(&path, Rule::TransportStream);
    assert_eq!(text.kind, FileKind::File);
    assert_eq!(
        text.stamp.size,
        Some(std::fs::metadata(&path).unwrap().len())
    );
    let mut bytes = vec![0xff; 188 * 6];
    for index in 0..6 {
        bytes[index * 188..index * 188 + 4].copy_from_slice(&[0x47, 0x01, 0, 0x10]);
    }
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(probe(&path, Rule::TransportStream).kind, FileKind::Video);
    std::fs::write(&path, [0x47]).unwrap();
    assert_eq!(probe(&path, Rule::TransportStream).kind, FileKind::File);
    assert_eq!(
        probe(&directory.0.join("File02.ts"), Rule::TransportStream).kind,
        FileKind::File
    );
    assert_eq!(
        probe(&directory.0, Rule::TransportStream).kind,
        FileKind::File
    );
}

#[test]
fn classification_in_icon_mode_and_preview_mode_shares_one_result_without_enlarging_icons() {
    let panels = [panel(false), panel(true)];
    let cache = RefCell::new(ThumbLru::new(4));
    let path = PathBuf::from("folder01").join("File01.ts");
    let requests = apply(
        &panels,
        &cache,
        &Config::default(),
        &path,
        &[location(0), location(1)],
        resolution(FileKind::Video),
    );
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].locations, [location(1)]);
    assert_eq!(requests[0].job.kind, FileKind::Video);
    assert!(requests[0].job.probe.is_none());
    for panel in &panels {
        let row = panel.rows_model.row_data(0).unwrap();
        assert_eq!(row.kind, FileKind::Video.as_i32());
        assert!(!row.preview_capable);
        assert_eq!(row.visual_h, 28.0);
        assert!(row.selected);
        assert_eq!(row.comment, "Note01");
    }
    cache
        .borrow_mut()
        .put(path.to_string_lossy().into_owned(), image());
    promote_panel_previews(&panels[1], &cache, &Config::default());
    let row = panels[1].rows_model.row_data(0).unwrap();
    assert!(row.preview_capable);
    assert_eq!(row.visual_h, 76.0);
    assert!(row.selected);
    assert_eq!(panels[0].rows_model.row_data(0).unwrap().visual_h, 28.0);
    assert!(
        apply(
            &panels,
            &cache,
            &Config::default(),
            &path,
            &[location(0), location(1)],
            resolution(FileKind::File)
        )
        .is_empty()
    );
    assert!(!cache.borrow().contains(path.to_string_lossy().as_ref()));
    assert_eq!(cache.borrow().kinds.kind(&path), Some(FileKind::File));
    assert_eq!(panels[1].rows_model.row_data(0).unwrap().visual_h, 28.0);
    assert!(panels[1].rows_model.row_data(0).unwrap().selected);
}

#[test]
fn old_generations_cannot_publish_a_probe_or_replace_a_new_job() {
    let scheduler = ThumbScheduler::new();
    let path = PathBuf::from("folder01").join("File01.ts");
    let mut requests = HashMap::new();
    push_probe(
        &mut requests,
        path.clone(),
        Rule::TransportStream,
        location(0),
    );
    scheduler.replace_pending(requests.into_values().collect());
    let old = scheduler.try_take_next().unwrap();
    scheduler.invalidate_paths(std::slice::from_ref(&path));
    assert!(!scheduler.set_probe_result(&path, old.serial, resolution(FileKind::Video)));
    assert!(scheduler.take_probe_result(&path, old.serial).is_none());
    let mut requests = HashMap::new();
    push_probe(
        &mut requests,
        path.clone(),
        Rule::TransportStream,
        location(0),
    );
    scheduler.replace_pending(requests.into_values().collect());
    let current = scheduler.try_take_next().unwrap();
    assert_ne!(old.serial, current.serial);
    assert!(scheduler.set_probe_result(&path, current.serial, resolution(FileKind::File)));
    scheduler.complete(&path, old.serial);
    assert_eq!(
        scheduler
            .take_probe_result(&path, current.serial)
            .unwrap()
            .kind,
        FileKind::File
    );
    assert_eq!(
        scheduler.in_flight_locations(&path, current.serial),
        [location(0)]
    );
    scheduler.complete(&path, current.serial);
}

#[test]
fn cache_is_bounded_and_content_invalidation_removes_every_related_result() {
    let mut cache = ThumbLru::new(2);
    for name in ["File01.ts", "File02.ts", "File03.ts"] {
        cache
            .kinds
            .insert(PathBuf::from(name), resolution(FileKind::File));
    }
    assert_eq!(cache.kinds.map.len(), 2);
    assert!(cache.kinds.kind(Path::new("File01.ts")).is_none());
    cache.put("File02.ts".into(), image());
    cache.mark_failed("File03.ts".into());
    cache.remove_path(Path::new("File02.ts"));
    cache.remove_path(Path::new("File03.ts"));
    assert!(cache.kinds.paths().next().is_none());
    assert!(cache.map.is_empty());
    assert!(cache.failed.is_empty());
}

#[test]
fn a_relisted_changed_version_cannot_reuse_the_old_kind_or_pixels() {
    let path = PathBuf::from("folder01").join("File01.ts");
    let cache = RefCell::new(ThumbLru::new(4));
    cache
        .borrow_mut()
        .kinds
        .insert(path.clone(), resolution(FileKind::Video));
    cache
        .borrow_mut()
        .put(path.to_string_lossy().into_owned(), image());
    let mut entry = Entry {
        name: "File01.ts".into(),
        path: path.clone(),
        size_bytes: Some(16),
        mtime_unix: Some(32),
        created_unix: None,
        is_dir: false,
        kind: FileKind::File,
        hidden: false,
        is_symlink: false,
        content_remote: false,
        is_share: false,
        executable: true,
    };
    assert_eq!(entry_kind(&entry, &cache), FileKind::Video);
    entry.size_bytes = Some(17);
    assert_eq!(entry_kind(&entry, &cache), FileKind::File);
    assert!(cache.borrow().kinds.kind(&path).is_none());
    assert!(!cache.borrow().contains(path.to_string_lossy().as_ref()));
    cache
        .borrow_mut()
        .kinds
        .insert(path.clone(), resolution(FileKind::Video));
    entry.size_bytes = Some(16);
    entry.mtime_unix = Some(33);
    assert_eq!(entry_kind(&entry, &cache), FileKind::File);
    assert!(cache.borrow().kinds.kind(&path).is_none());
}

#[test]
fn ordinary_listings_skip_the_probe_path_and_the_gate_follows_replacements() {
    let panel = panel(false);
    assert!(panel.has_ambiguous_files.get());
    panel.replace_rows(vec![
        row("File01.txt"),
        row("File02.mp4"),
        row("File03.obj"),
    ]);
    assert!(!panel.has_ambiguous_files.get());
    panel.replace_rows(vec![row("File01.TS")]);
    assert!(panel.has_ambiguous_files.get());
    panel.replace_rows(Vec::new());
    assert!(!panel.has_ambiguous_files.get());
}

#[test]
fn a_cached_generic_result_demotes_a_previously_previewed_row_when_it_reappears() {
    let panel = panel(true);
    let mut previous = panel.rows_model.row_data(0).unwrap();
    previous.kind = FileKind::Video.as_i32();
    previous.preview_capable = true;
    previous.thumbnail = image();
    let mut rows = vec![previous];
    layout_rows(&mut rows, THUMB_DEFAULT_ZOOM, true, 0);
    panel.replace_rows(rows);
    assert_eq!(panel.rows_model.row_data(0).unwrap().visual_h, 76.0);
    let cache = RefCell::new(ThumbLru::new(4));
    let mut row = panel.rows_model.row_data(0).unwrap();
    cache
        .borrow_mut()
        .kinds
        .insert(row_path(&row), resolution(FileKind::File));
    let mut requests = HashMap::new();
    assert!(prepare(&cache, &mut row, true, location(0), &mut requests));
    assert!(requests.is_empty());
    assert_eq!(row.kind, FileKind::File.as_i32());
    assert!(!row.preview_capable);
    assert_eq!(row.thumbnail.size().width, 0);
    panel.rows_model.set_row_data(0, row);
    relayout_changed_previews(&panel, &Config::default());
    assert_eq!(panel.rows_model.row_data(0).unwrap().visual_h, 28.0);
    assert!(panel.rows_model.row_data(0).unwrap().selected);
}

#[test]
fn classification_does_not_touch_recycled_rows_and_grid_geometry_stays_fixed() {
    let mut panel = panel(true);
    panel.tabs.tabs[0].item_arrangement = ItemArrangement::Grid;
    panel.tabs.tabs[0].cursor = 0;
    let cache = RefCell::new(ThumbLru::new(4));
    let path = PathBuf::from("folder01").join("File01.ts");
    apply(
        std::slice::from_ref(&panel),
        &cache,
        &Config::default(),
        &path,
        &[location(0)],
        resolution(FileKind::Video),
    );
    let before = panel.rows_model.row_data(0).unwrap();
    cache
        .borrow_mut()
        .put(path.to_string_lossy().into_owned(), image());
    promote_panel_previews(&panel, &cache, &Config::default());
    let after = panel.rows_model.row_data(0).unwrap();
    assert!(after.preview_capable);
    assert_eq!(
        (after.visual_y, after.visual_h),
        (before.visual_y, before.visual_h)
    );
    assert_eq!(panel.tabs.tabs[0].cursor, 0);
    assert!(after.selected);
    panel.replace_rows(vec![row("File02.ts")]);
    assert!(
        apply(
            std::slice::from_ref(&panel),
            &cache,
            &Config::default(),
            &path,
            &[location(0)],
            resolution(FileKind::Video)
        )
        .is_empty()
    );
    assert_eq!(
        panel.rows_model.row_data(0).unwrap().kind,
        FileKind::File.as_i32()
    );
}

#[cfg(windows)]
#[test]
fn offline_content_is_not_classified_as_video_and_keeps_the_remote_state() {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_OFFLINE, SetFileAttributesW,
    };
    use windows::core::PCWSTR;
    let directory = crate::test_support::TestFolder::new();
    let path = directory.0.join("File01.ts");
    std::fs::write(&path, [0x47; PROBE_BYTES]).unwrap();
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_ATTRIBUTE_OFFLINE).unwrap();
    }
    let result = probe(&path, Rule::TransportStream);
    unsafe {
        SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_ATTRIBUTE_NORMAL).unwrap();
    }
    assert!(result.remote);
    assert_eq!(result.kind, FileKind::File);
    let panels = [panel(false)];
    let cache = RefCell::new(ThumbLru::new(4));
    let target = PathBuf::from("folder01").join("File01.ts");
    assert!(
        apply(
            &panels,
            &cache,
            &Config::default(),
            &target,
            &[location(0)],
            result
        )
        .is_empty()
    );
    assert!(panels[0].rows_model.row_data(0).unwrap().content_remote);
    assert!(cache.borrow().kinds.kind(&target).is_none());
}
