#![cfg(test)]

use crate::{
    file_state::FileState,
    patch::{InsertOp, PatchOperation, ReplaceOp},
};
use console::style;
use fractional_index::FractionalIndex;
use std::{fs, path::PathBuf};
use tempfile::Builder;

fn setup_test_file(content: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp_dir = Builder::new().prefix("test-fs-").tempdir().unwrap();
    let file_path = tmp_dir.path().join("test.txt");
    fs::write(&file_path, content).unwrap();
    (tmp_dir, file_path)
}

fn get_indexes(state: &FileState) -> Vec<FractionalIndex> {
    state.lines.keys().cloned().collect()
}

#[test]
fn test_file_state_new() {
    let (_tmp_dir, file_path) = setup_test_file("line 1\nline 2");
    let state = FileState::new(file_path, "line 1\nline 2");

    assert_eq!(state.lines.len(), 2);
    let indexes = get_indexes(&state);
    assert_eq!(state.lines.get(&indexes[0]), Some(&"line 1".to_string()));
    assert_eq!(state.lines.get(&indexes[1]), Some(&"line 2".to_string()));

    let expected_lif_content = format!("{:?}: line 1\n{:?}: line 2", indexes[0], indexes[1]);
    let expected_hash = FileState::calculate_hash(&expected_lif_content);
    assert_eq!(state.lif_hash, expected_hash);
}

#[test]
fn test_apply_and_write_patch() {
    let (_tmp_dir, file_path) = setup_test_file("line 1\nline 3");
    let mut state = FileState::new(file_path.clone(), "line 1\nline 3");
    let old_indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Insert(InsertOp {
        after_lid: Some(old_indexes[0].clone()),
        content: vec!["line 2".to_string()],
    })];

    let diff = state.apply_and_write_patch(&patch).unwrap();

    // Check file on disk
    let disk_content = fs::read_to_string(&file_path).unwrap();
    assert_eq!(disk_content, "line 1\nline 2\nline 3");

    // Check in-memory state
    assert_eq!(state.get_full_content(), "line 1\nline 2\nline 3");
    let new_indexes = get_indexes(&state);
    let inserted_index = &new_indexes[1];

    // Check diff
    assert!(
        diff.contains(
            &style(format!("+ {}: line 2", inserted_index.to_string()))
                .green()
                .to_string()
        )
    );
}

#[test]
fn test_patch_insert_at_start() {
    let (_tmp_dir, file_path) = setup_test_file("line 1");
    let mut state = FileState::new(file_path, "line 1");

    let patch = vec![PatchOperation::Insert(InsertOp {
        after_lid: None, // This signifies start-of-file
        content: vec!["new first line".to_string()],
    })];
    state.apply_patch(&patch).unwrap();
    let new_indexes = get_indexes(&state);

    assert_eq!(state.lines.len(), 2);
    assert_eq!(
        state.lines.get(&new_indexes[0]),
        Some(&"new first line".to_string())
    );
    assert_eq!(state.get_full_content(), "new first line\nline 1");
}

#[test]
fn test_patch_insert_in_middle() {
    let (_tmp_dir, file_path) = setup_test_file("line 1\nline 2");
    let mut state = FileState::new(file_path, "line 1\nline 2");
    let old_indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Insert(InsertOp {
        after_lid: Some(old_indexes[0].clone()),
        content: vec!["new middle line".to_string()],
    })];
    state.apply_patch(&patch).unwrap();
    let new_indexes = get_indexes(&state);

    assert_eq!(state.lines.len(), 3);
    assert_eq!(
        state.lines.get(&new_indexes[1]),
        Some(&"new middle line".to_string())
    );
    assert_eq!(state.get_full_content(), "line 1\nnew middle line\nline 2");
}

#[test]
fn test_patch_delete_range() {
    let content = "line 1\nline 2\nline 3\nline 4";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);
    let old_indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Replace(ReplaceOp {
        start_lid: old_indexes[1].clone(),
        end_lid: old_indexes[2].clone(),
        content: vec![], // Empty content means delete
    })];
    state.apply_patch(&patch).unwrap();

    assert_eq!(state.lines.len(), 2);
    assert_eq!(state.get_full_content(), "line 1\nline 4");
}

#[test]
fn test_patch_replace_range() {
    let content = "line 1\nline 2\nline 3\nline 4";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);
    let old_indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Replace(ReplaceOp {
        start_lid: old_indexes[1].clone(),
        end_lid: old_indexes[2].clone(),
        content: vec!["replacement".to_string()],
    })];
    state.apply_patch(&patch).unwrap();

    assert_eq!(state.lines.len(), 3);
    assert_eq!(state.get_full_content(), "line 1\nreplacement\nline 4");
}

#[test]
fn test_patch_replace_entire_file() {
    let content = "line 1\nline 2\nline 3";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);
    let indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Replace(ReplaceOp {
        start_lid: indexes[0].clone(),
        end_lid: indexes[2].clone(),
        content: vec!["all new".to_string()],
    })];
    state.apply_patch(&patch).unwrap();

    assert_eq!(state.lines.len(), 1);
    assert_eq!(state.get_full_content(), "all new");
}

#[test]
fn test_patch_delete_entire_file() {
    let content = "line 1\nline 2\nline 3";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);
    let indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Replace(ReplaceOp {
        start_lid: indexes[0].clone(),
        end_lid: indexes[2].clone(),
        content: vec![],
    })];
    state.apply_patch(&patch).unwrap();

    assert_eq!(state.lines.len(), 0);
    assert_eq!(state.get_full_content(), "");
}

#[test]
fn test_patch_replace_invalid_range_start_after_end() {
    let content = "line 1\nline 2\nline 3";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);
    let indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Replace(ReplaceOp {
        start_lid: indexes[2].clone(),
        end_lid: indexes[0].clone(),
        content: vec!["new".to_string()],
    })];

    let result = state.apply_patch(&patch);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("cannot be numerically greater than")
    );
}

#[test]
fn test_no_error_on_repeated_insertions() {
    let (_tmp_dir, file_path) = setup_test_file("a\nb");
    let mut state = FileState::new(file_path, "a\nb");

    for i in 0..100 {
        let indexes = get_indexes(&state);
        let patch = vec![PatchOperation::Insert(InsertOp {
            after_lid: Some(indexes[i].clone()),
            content: vec![format!("new line {i}")],
        })];
        // This should never fail with fractional indexing
        state.apply_patch(&patch).unwrap();
    }
    assert_eq!(state.lines.len(), 102);
}
