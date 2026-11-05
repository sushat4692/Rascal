use rascal_core::{DiffRow, DiffRowKind, FileDiff, FileStatus};

use super::{DiffDisplayRow, DiffInfo, DiffLayout, SplitPair, build_rows};

fn row(kind: DiffRowKind, old: Option<u32>, new: Option<u32>, text: &str) -> DiffRow {
    DiffRow {
        kind,
        old_lineno: old,
        new_lineno: new,
        text: text.to_string(),
    }
}

fn diff(rows: Vec<DiffRow>) -> FileDiff {
    FileDiff {
        path: "a.txt".to_string(),
        old_path: None,
        status: FileStatus::Modified,
        is_binary: false,
        is_submodule: false,
        base_label: "第1親".to_string(),
        additions: 0,
        deletions: 0,
        rows,
        truncated: false,
    }
}

#[test]
fn split_pairs_removed_and_added_lines() {
    let rows = build_rows(
        &diff(vec![
            row(DiffRowKind::HunkHeader, None, None, "@@"),
            row(DiffRowKind::Context, Some(1), Some(1), "same"),
            row(DiffRowKind::Removed, Some(2), None, "old1"),
            row(DiffRowKind::Removed, Some(3), None, "old2"),
            row(DiffRowKind::Added, None, Some(2), "new1"),
            row(DiffRowKind::Context, Some(4), Some(3), "tail"),
        ]),
        DiffLayout::Split,
    );

    assert!(matches!(rows[0], DiffDisplayRow::Full(_)));
    assert!(matches!(
        rows[1],
        DiffDisplayRow::Split(SplitPair {
            old: Some(_),
            new: Some(_)
        })
    ));
    let DiffDisplayRow::Split(pair) = &rows[2] else {
        panic!("expected split pair");
    };
    assert_eq!(pair.old.as_ref().unwrap().text, "old1");
    assert_eq!(pair.new.as_ref().unwrap().text, "new1");
    let DiffDisplayRow::Split(pair) = &rows[3] else {
        panic!("expected split pair");
    };
    assert_eq!(pair.old.as_ref().unwrap().text, "old2");
    assert!(pair.new.is_none());
}

#[test]
fn split_handles_added_only_blocks() {
    let rows = build_rows(
        &diff(vec![row(DiffRowKind::Added, None, Some(1), "new")]),
        DiffLayout::Split,
    );
    let DiffDisplayRow::Split(pair) = &rows[0] else {
        panic!("expected split pair");
    };
    assert!(pair.old.is_none());
    assert_eq!(pair.new.as_ref().unwrap().text, "new");
}

#[test]
fn unified_keeps_one_row_per_line() {
    let rows = build_rows(
        &diff(vec![
            row(DiffRowKind::Removed, Some(1), None, "old"),
            row(DiffRowKind::Added, None, Some(1), "new"),
        ]),
        DiffLayout::Unified,
    );
    assert_eq!(rows.len(), 2);
    assert!(matches!(rows[0], DiffDisplayRow::Unified(_)));
    assert!(matches!(rows[1], DiffDisplayRow::Unified(_)));
}

#[test]
fn info_rows_short_circuit() {
    let mut file = diff(Vec::new());
    file.is_binary = true;
    let rows = build_rows(&file, DiffLayout::Split);
    assert!(matches!(
        rows.as_slice(),
        [DiffDisplayRow::Info(DiffInfo::Binary)]
    ));
}

#[test]
fn layout_keys_roundtrip() {
    for layout in [DiffLayout::Unified, DiffLayout::Split] {
        assert_eq!(DiffLayout::from_key(layout.key()), Some(layout));
    }
}
