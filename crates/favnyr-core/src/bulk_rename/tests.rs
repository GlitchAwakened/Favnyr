use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct Tree(PathBuf);
impl Tree {
    fn new() -> Self {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/bulk-rename-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .canonicalize()
            .unwrap();
        assert!(self.0.starts_with(root));
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn numbering() -> Options {
    Options {
        common: Some("File".into()),
        numbering: Some(Numbering {
            start: 1,
            step: 1,
            digits: 2,
            before: false,
            separator: "".into(),
        }),
        ..Default::default()
    }
}

#[test]
fn transformations_preserve_extensions_and_unicode_boundaries() {
    let options = Options {
        replacement: Some(Replacement {
            find: "é".into(),
            with: "x".into(),
            case_sensitive: false,
            all: true,
        }),
        affixes: Some(("Folder_".into(), "_01".into())),
        case: Case::Upper,
        ..Default::default()
    };
    assert_eq!(
        transform::name("été.tar.gz", false, 0, &options).unwrap(),
        "Folder_XTX_01.tar.gz"
    );
    assert_eq!(
        transform::name("été.tar.gz", true, 0, &options).unwrap(),
        "Folder_XTX.TAR.GZ_01"
    );
    assert_eq!(
        transform::name(".hidden", false, 0, &Options::default()).unwrap(),
        ".hidden"
    );
    let mut options = numbering();
    options.extension = Some(".png".into());
    assert_eq!(
        transform::name("File01.jpg", false, 8, &options).unwrap(),
        "File09.png"
    );
    assert_eq!(
        transform::name("Folder01", true, 8, &options).unwrap(),
        "File09"
    );
    options.numbering.as_mut().unwrap().start = u64::MAX;
    assert_eq!(
        transform::name("File01", false, 1, &options),
        Err(Issue::InvalidOptions)
    );
}

#[cfg(not(windows))]
#[test]
fn an_empty_extension_does_not_silently_strip_a_trailing_dot() {
    assert_eq!(
        transform::name("File01.", false, 0, &Options::default()).unwrap(),
        "File01."
    );
}

#[test]
fn preview_rejects_conflicts_nested_selection_and_invalid_names() {
    let tree = Tree::new();
    let a = tree.file("Folder01/File01.txt", b"a");
    let b = tree.file("Folder01/File02.txt", b"b");
    let common = Options {
        common: Some("File03".into()),
        ..Default::default()
    };
    let plan = preview(&[a.clone(), b.clone()], &common, || false).unwrap();
    assert!(
        plan.entries
            .iter()
            .all(|e| e.issue == Some(Issue::Duplicate))
    );
    let plan = preview(&[tree.0.join("Folder01"), a.clone()], &numbering(), || {
        false
    })
    .unwrap();
    assert!(
        plan.entries
            .iter()
            .all(|e| e.issue == Some(Issue::NestedSelection))
    );
    tree.file("Folder01/File03.txt", b"c");
    let plan = preview(std::slice::from_ref(&a), &common, || false).unwrap();
    assert_eq!(plan.entries[0].issue, Some(Issue::Occupied));
    let bad = Options {
        common: Some("Folder02/File01".into()),
        ..Default::default()
    };
    assert!(!preview(&[a, b], &bad, || false).unwrap().valid());
}

#[test]
fn cycles_and_case_changes_keep_contents() {
    let tree = Tree::new();
    let a = tree.file("File01.txt", b"a");
    let b = tree.file("File02.txt", b"b");
    let plan = preview(&[b.clone(), a.clone()], &numbering(), || false).unwrap();
    assert!(plan.valid());
    let outcome = execute(&plan, || false, |_, _| {});
    assert!(outcome.failure.is_none());
    assert_eq!(std::fs::read(&a).unwrap(), b"b");
    assert_eq!(std::fs::read(&b).unwrap(), b"a");
    let plan = preview(
        &[a],
        &Options {
            case: Case::Lower,
            ..Default::default()
        },
        || false,
    )
    .unwrap();
    assert!(execute(&plan, || false, |_, _| {}).failure.is_none());
    assert_eq!(std::fs::read(tree.0.join("file01.txt")).unwrap(), b"b");
}

#[test]
fn failed_commit_restores_cycles_and_reports_unrestored_paths() {
    for fail_recovery in [false, true] {
        let tree = Tree::new();
        let a = tree.file("File01.txt", b"a");
        // Distinct creation times exercise name tunneling during a permutation.
        std::thread::sleep(std::time::Duration::from_millis(25));
        let b = tree.file("File02.txt", b"b");
        let plan = preview(&[b.clone(), a.clone()], &numbering(), || false).unwrap();
        let outcome = execute::fail_commit(&plan, fail_recovery);
        assert!(outcome.failure.is_some());
        if fail_recovery {
            assert_eq!(outcome.locations.len(), 2);
            for (source, current) in outcome.locations {
                assert_eq!(
                    std::fs::read(current).unwrap(),
                    if source == a { b"a" } else { b"b" }
                );
            }
        } else {
            assert!(outcome.locations.is_empty(), "{outcome:?}");
            assert_eq!(std::fs::read(a).unwrap(), b"a");
            assert_eq!(std::fs::read(b).unwrap(), b"b");
        }
    }
}

#[test]
fn cancellation_and_destination_races_never_overwrite() {
    let tree = Tree::new();
    let a = tree.file("File03.txt", b"a");
    let b = tree.file("File04.txt", b"b");
    let plan = preview(&[a.clone(), b.clone()], &numbering(), || false).unwrap();
    let cancel = std::cell::Cell::new(false);
    let outcome = execute(&plan, || cancel.get(), |_, _| cancel.set(true));
    assert!(outcome.cancelled);
    assert!(outcome.locations.is_empty());
    let target = tree.0.join("File01.txt");
    let outcome = execute(
        &plan,
        || false,
        |done, total| {
            if done == total / 2 {
                std::fs::write(&target, b"c").unwrap();
            }
        },
    );
    assert!(outcome.failure.is_some());
    assert!(outcome.locations.is_empty());
    assert_eq!(std::fs::read(target).unwrap(), b"c");
    assert_eq!(std::fs::read(a).unwrap(), b"a");
    assert_eq!(std::fs::read(b).unwrap(), b"b");
}

#[test]
fn a_source_changed_since_preview_is_not_renamed() {
    let tree = Tree::new();
    let a = tree.file("File03.txt", b"a");
    let plan = preview(std::slice::from_ref(&a), &numbering(), || false).unwrap();
    std::fs::write(&a, b"changed").unwrap();
    let outcome = execute(&plan, || false, |_, _| {});
    assert!(outcome.failure.is_some());
    assert!(outcome.steps.is_empty());
    assert_eq!(std::fs::read(a).unwrap(), b"changed");
}

#[test]
fn directory_cycles_and_multiple_parents_keep_content_and_global_numbering() {
    let tree = Tree::new();
    tree.file("Folder01/File01.txt", b"a");
    tree.file("Folder02/File01.txt", b"b");
    let options = Options {
        common: Some("Folder".into()),
        ..numbering()
    };
    let a = tree.0.join("Folder01");
    let b = tree.0.join("Folder02");
    let plan = preview(&[b.clone(), a.clone()], &options, || false).unwrap();
    let outcome = execute(&plan, || false, |_, _| {});
    assert!(outcome.failure.is_none(), "{outcome:?}");
    assert_eq!(std::fs::read(a.join("File01.txt")).unwrap(), b"b");
    assert_eq!(std::fs::read(b.join("File01.txt")).unwrap(), b"a");
    let first = tree.file("Folder01/File03.txt", b"c");
    let second = tree.file("Folder02/File04.txt", b"d");
    let plan = preview(
        &[first, second],
        &Options {
            numbering: Some(Numbering {
                start: 10,
                step: 2,
                digits: 3,
                before: true,
                separator: "_".into(),
            }),
            ..Default::default()
        },
        || false,
    )
    .unwrap();
    assert!(execute(&plan, || false, |_, _| {}).failure.is_none());
    assert_eq!(std::fs::read(a.join("010_File03.txt")).unwrap(), b"c");
    assert_eq!(std::fs::read(b.join("012_File04.txt")).unwrap(), b"d");
}

#[test]
fn replacement_keeps_unicode_characters_whole_and_can_target_first_match() {
    let options = |find: &str, replacement: &str, all: bool| Options {
        replacement: Some(Replacement {
            find: find.into(),
            with: replacement.into(),
            case_sensitive: false,
            all,
        }),
        ..Default::default()
    };
    assert_eq!(
        transform::name("ΟΣ01.txt", false, 0, &options("ΟΣ", "File", true)).unwrap(),
        "File01.txt"
    );
    assert_eq!(
        transform::name(
            "File01_File01.TXT",
            false,
            0,
            &options("file", "Item", false)
        )
        .unwrap(),
        "Item01_File01.TXT"
    );
    assert_eq!(
        transform::name(
            "File01_File01.TXT",
            false,
            0,
            &options("file", "Item", true)
        )
        .unwrap(),
        "Item01_Item01.TXT"
    );
    assert_eq!(
        transform::name("İ01.txt", false, 0, &options("i", "x", true)).unwrap(),
        "İ01.txt"
    );
    assert_eq!(
        transform::name("İ01.txt", false, 0, &options("İ", "x", true)).unwrap(),
        "x01.txt"
    );
}
