#![cfg(test)]

use crate::{
    file_state::{FileState, RangeSpec},
    patch::{InsertOperation, PatchOperation, ReplaceOperation},
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

fn get_indexes(state: &FileState) -> Vec<String> {
    state.lines.keys().map(|k| k.to_string()).collect()
}

#[test]
fn test_file_state_new() {
    let (_tmp_dir, file_path) = setup_test_file("line 1\nline 2");
    let state = FileState::new(file_path, "line 1\nline 2");

    assert_eq!(state.lines.len(), 2);
    let indexes = get_indexes(&state);
    assert_eq!(
        state
            .lines
            .get(&FractionalIndex::from_string(&indexes[0]).unwrap()),
        Some(&"line 1".to_string())
    );
    assert_eq!(
        state
            .lines
            .get(&FractionalIndex::from_string(&indexes[1]).unwrap()),
        Some(&"line 2".to_string())
    );

    let expected_lif_content = format!("{}: line 1\n{}: line 2", indexes[0], indexes[1]);
    let expected_hash = FileState::calculate_hash(&expected_lif_content);
    assert_eq!(state.lif_hash, expected_hash);
}

#[test]
fn test_get_lif_representation_new_format() {
    let (_tmp_dir, file_path) = setup_test_file("line 1\nline 2");
    let state = FileState::new(file_path.clone(), "line 1\nline 2");
    let representation = state.get_lif_representation();

    let project_root = std::env::current_dir().unwrap();
    let relative_path = file_path.strip_prefix(&project_root).unwrap_or(&file_path);
    let short_hash = state.get_short_hash();
    let indexes = get_indexes(&state);

    let expected_header = format!(
        "File: {} | Hash: {} | Lines: 1-2/2",
        relative_path.display(),
        short_hash
    );
    let expected_body = format!("1    {}: line 1\n2    {}: line 2", indexes[0], indexes[1]);
    assert_eq!(
        representation,
        format!("{expected_header}\n{expected_body}")
    );
}

#[test]
fn test_get_lif_string_for_range() {
    let (_tmp_dir, file_path) = setup_test_file("1\n2\n3\n4\n5");
    let state = FileState::new(file_path.clone(), "1\n2\n3\n4\n5");

    let partial_representation = state.get_lif_string_for_ranges(Some(&[RangeSpec {
        start_line: 2,
        end_line: 4,
    }]));

    let project_root = std::env::current_dir().unwrap();
    let relative_path = file_path.strip_prefix(&project_root).unwrap_or(&file_path);
    let short_hash = state.get_short_hash();
    let indexes = get_indexes(&state);

    let expected_header = format!(
        "File: {} | Hash: {} | Lines: 2-4/5",
        relative_path.display(),
        short_hash
    );
    let expected_body = format!(
        "2    {}: 2\n3    {}: 3\n4    {}: 4",
        indexes[1], indexes[2], indexes[3]
    );
    assert_eq!(
        partial_representation,
        format!("{expected_header}\n{expected_body}")
    );
}

#[test]
fn test_apply_and_write_patch() {
    let (_tmp_dir, file_path) = setup_test_file("line 1\nline 3");
    let mut state = FileState::new(file_path.clone(), "line 1\nline 3");
    let old_indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Insert(InsertOperation {
        after_lid: old_indexes[0].clone(),
        content: vec!["line 2".to_string()],
        context_before: None,
        context_after: None,
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
            &style(format!("+ {inserted_index}: line 2"))
                .green()
                .to_string()
        )
    );
}

#[test]
fn test_get_full_content() {
    let (_tmp_dir, file_path) = setup_test_file("one\ntwo");
    let state = FileState::new(file_path, "one\ntwo");
    assert_eq!(state.get_full_content(), "one\ntwo");
}

#[test]
fn test_patch_insert_at_start() {
    let (_tmp_dir, file_path) = setup_test_file("line 1");
    let mut state = FileState::new(file_path, "line 1");
    let original_hash = state.lif_hash.clone();

    let patch = vec![PatchOperation::Insert(InsertOperation {
        after_lid: "_START_OF_FILE_".to_string(),
        content: vec!["new first line".to_string()],
        context_before: None,
        context_after: Some("line 1".to_string()),
    })];
    state.apply_patch(&patch).unwrap();
    let new_indexes = get_indexes(&state);

    assert_eq!(state.lines.len(), 2);
    assert_eq!(
        state
            .lines
            .get(&FractionalIndex::from_string(&new_indexes[0]).unwrap()),
        Some(&"new first line".to_string())
    );
    assert_ne!(state.lif_hash, original_hash);
    assert_eq!(state.get_full_content(), "new first line\nline 1");
}

#[test]
fn test_patch_insert_in_middle() {
    let (_tmp_dir, file_path) = setup_test_file("line 1\nline 2");
    let mut state = FileState::new(file_path, "line 1\nline 2");
    let old_indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Insert(InsertOperation {
        after_lid: old_indexes[0].clone(),
        content: vec!["new middle line".to_string()],
        context_before: Some("line 1".to_string()),
        context_after: Some("line 2".to_string()),
    })];
    state.apply_patch(&patch).unwrap();
    let new_indexes = get_indexes(&state);

    assert_eq!(state.lines.len(), 3);
    assert_eq!(
        state
            .lines
            .get(&FractionalIndex::from_string(&new_indexes[1]).unwrap()),
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

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: old_indexes[1].clone(),
        end_lid: old_indexes[2].clone(),
        content: vec![],
        context_before: Some("line 1".to_string()),
        context_after: Some("line 4".to_string()),
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

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: old_indexes[1].clone(),
        end_lid: old_indexes[2].clone(),
        content: vec!["replacement".to_string()],
        context_before: Some("line 1".to_string()),
        context_after: Some("line 4".to_string()),
    })];
    state.apply_patch(&patch).unwrap();

    assert_eq!(state.lines.len(), 3);
    assert_eq!(state.get_full_content(), "line 1\nreplacement\nline 4");
}

#[test]
fn test_deserialize_patch_operation() {
    // Note: The LIDs are now just strings, so no "LID" prefix is needed.
    let json_data = r#"
        [
            {
                "op": "r",
                "start_lid": "a",
                "end_lid": "b",
                "content": ["new content"],
                "context_before": "optional context",
                "context_after": null
            },
            {
                "op": "i",
                "after_lid": "c",
                "content": ["inserted line 1", "inserted line 2"],
                "context_before": null,
                "context_after": "optional context 2"
            }
        ]
        "#;
    let operations: Vec<PatchOperation> = serde_json::from_str(json_data).unwrap();
    assert_eq!(operations.len(), 2);
    assert_eq!(
        operations[0],
        PatchOperation::Replace(ReplaceOperation {
            start_lid: "a".to_string(),
            end_lid: "b".to_string(),
            content: vec!["new content".to_string()],
            context_before: Some("optional context".to_string()),
            context_after: None
        })
    );
    assert_eq!(
        operations[1],
        PatchOperation::Insert(InsertOperation {
            after_lid: "c".to_string(),
            content: vec!["inserted line 1".to_string(), "inserted line 2".to_string()],
            context_before: None,
            context_after: Some("optional context 2".to_string())
        })
    );
}

#[test]
fn test_edit_same_file_thrice_sequentially() {
    let content = "line 1\nline 2\nline 3";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);

    // First patch
    let indexes1 = get_indexes(&state);
    let patch1 = vec![PatchOperation::Insert(InsertOperation {
        after_lid: indexes1[0].clone(),
        content: vec!["inserted after 1".to_string()],
        context_before: Some("line 1".to_string()),
        context_after: Some("line 2".to_string()),
    })];
    state.apply_patch(&patch1).unwrap();

    assert_eq!(state.lines.len(), 4);
    assert_eq!(
        state.get_full_content(),
        "line 1\ninserted after 1\nline 2\nline 3"
    );

    // Second patch
    let indexes2 = get_indexes(&state);
    let patch2 = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: indexes2[2].clone(),
        end_lid: indexes2[3].clone(),
        content: vec!["replacement".to_string()],
        context_before: Some("inserted after 1".to_string()),
        context_after: None,
    })];
    state.apply_patch(&patch2).unwrap();

    assert_eq!(state.lines.len(), 3);
    assert_eq!(
        state.get_full_content(),
        "line 1\ninserted after 1\nreplacement"
    );

    // Third patch
    let indexes3 = get_indexes(&state);
    let patch3 = vec![PatchOperation::Insert(InsertOperation {
        after_lid: indexes3[1].clone(),
        content: vec!["another insertion".to_string()],
        context_before: Some("inserted after 1".to_string()),
        context_after: Some("replacement".to_string()),
    })];
    state.apply_patch(&patch3).unwrap();

    assert_eq!(state.lines.len(), 4);
    assert_eq!(
        state.get_full_content(),
        "line 1\ninserted after 1\nanother insertion\nreplacement"
    );
}

#[test]
fn test_patch_replace_invalid_range_start_after_end() {
    let content = "line 1\nline 2\nline 3";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);
    let indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: indexes[2].clone(),
        end_lid: indexes[0].clone(),
        content: vec!["new".to_string()],
        context_before: None,
        context_after: None,
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
fn test_patch_replace_non_existent_start_lid() {
    let content = "line 1\nline 2";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);
    let indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: "non-existent-index".to_string(),
        end_lid: indexes[1].clone(),
        content: vec!["new".to_string()],
        context_before: None,
        context_after: None,
    })];

    let result = state.apply_patch(&patch);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid index format")
    );
}

#[test]
fn test_patch_replace_non_existent_end_lid() {
    let content = "line 1\nline 2";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);
    let indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: indexes[0].clone(),
        end_lid: "non-existent-index".to_string(),
        content: vec!["new".to_string()],
        context_before: None,
        context_after: None,
    })];

    let result = state.apply_patch(&patch);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid index format")
    );
}

#[test]
fn test_no_error_on_repeated_insertions() {
    let (_tmp_dir, file_path) = setup_test_file("a\nb");
    let mut state = FileState::new(file_path, "a\nb");

    for i in 0..100 {
        let indexes = get_indexes(&state);
        let patch = vec![PatchOperation::Insert(InsertOperation {
            after_lid: indexes[i].clone(),
            content: vec![format!("new line {i}")],
            context_before: None,
            context_after: None,
        })];
        // This should never fail with fractional indexing
        state.apply_patch(&patch).unwrap();
    }
    assert_eq!(state.lines.len(), 102);
}

#[test]
fn test_patch_replace_first_line() {
    let content = "line 1\nline 2\nline 3";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);
    let indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: indexes[0].clone(),
        end_lid: indexes[0].clone(),
        content: vec!["new first".to_string()],
        context_before: None,
        context_after: None,
    })];
    state.apply_patch(&patch).unwrap();

    assert_eq!(state.lines.len(), 3);
    assert_eq!(state.get_full_content(), "new first\nline 2\nline 3");
}

#[test]
fn test_patch_replace_last_line() {
    let content = "line 1\nline 2\nline 3";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);
    let indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: indexes[2].clone(),
        end_lid: indexes[2].clone(),
        content: vec!["new last".to_string()],
        context_before: None,
        context_after: None,
    })];
    state.apply_patch(&patch).unwrap();

    assert_eq!(state.lines.len(), 3);
    assert_eq!(state.get_full_content(), "line 1\nline 2\nnew last");
}

#[test]
fn test_patch_replace_entire_file() {
    let content = "line 1\nline 2\nline 3";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);
    let indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: indexes[0].clone(),
        end_lid: indexes[2].clone(),
        content: vec!["all new".to_string()],
        context_before: None,
        context_after: None,
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

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: indexes[0].clone(),
        end_lid: indexes[2].clone(),
        content: vec![],
        context_before: None,
        context_after: None,
    })];
    state.apply_patch(&patch).unwrap();

    assert_eq!(state.lines.len(), 0);
    assert_eq!(state.get_full_content(), "");
}

#[test]
fn test_patch_insert_at_end() {
    let content = "line 1\nline 2";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);
    let indexes = get_indexes(&state);

    let patch = vec![PatchOperation::Insert(InsertOperation {
        after_lid: indexes[1].clone(),
        content: vec!["new last line".to_string()],
        context_before: None,
        context_after: None,
    })];
    state.apply_patch(&patch).unwrap();
    let new_indexes = get_indexes(&state);

    assert_eq!(state.lines.len(), 3);
    assert_eq!(
        state
            .lines
            .get(&FractionalIndex::from_string(&new_indexes[2]).unwrap()),
        Some(&"new last line".to_string())
    );
    assert_eq!(state.get_full_content(), "line 1\nline 2\nnew last line");
}

#[test]
fn test_parse_index_invalid_formats() {
    assert!(FileState::parse_index("").is_err());
    assert!(FileState::parse_index("not-a-valid-index").is_err());
}

#[test]
fn test_deserialize_malformed_patch_operation() {
    // Unknown operation code
    let json_unknown_op = r#"[{"op": "d", "after_lid": "a"}]"#;
    let result: Result<Vec<PatchOperation>, _> = serde_json::from_str(json_unknown_op);
    assert!(result.is_err());

    // Missing required field for "r"
    let json_missing_field_r = r#"[{"op": "r", "start_lid": "a"}]"#;
    let result: Result<Vec<PatchOperation>, _> = serde_json::from_str(json_missing_field_r);
    assert!(result.is_err());

    // Missing required field for "i"
    let json_missing_field_i = r#"[{"op": "i"}]"#;
    let result: Result<Vec<PatchOperation>, _> = serde_json::from_str(json_missing_field_i);
    assert!(result.is_err());
}

#[test]
fn test_file_state_new_empty_file() {
    let (_tmp_dir, file_path) = setup_test_file("");
    let state = FileState::new(file_path, "");

    assert!(state.lines.is_empty());
    let expected_hash = FileState::calculate_hash("");
    assert_eq!(state.lif_hash, expected_hash);
    assert_eq!(state.get_full_content(), "");
}

#[test]
fn test_file_state_new_with_single_newline() {
    let content = "\n";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let state = FileState::new(file_path, content);

    // A single newline is one empty line.
    assert_eq!(state.lines.len(), 1);
    let indexes = get_indexes(&state);
    assert_eq!(
        state
            .lines
            .get(&FractionalIndex::from_string(&indexes[0]).unwrap()),
        Some(&"".to_string())
    );
    assert_eq!(state.get_full_content(), "\n");
    assert!(state.ends_with_newline);
}

#[test]
fn test_file_state_new_with_trailing_newline() {
    let content = "line 1\n";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let state = FileState::new(file_path, content);

    assert_eq!(state.lines.len(), 1);
    let indexes = get_indexes(&state);
    assert_eq!(
        state
            .lines
            .get(&FractionalIndex::from_string(&indexes[0]).unwrap()),
        Some(&"line 1".to_string())
    );
    assert_eq!(state.get_full_content(), "line 1\n");
    assert!(state.ends_with_newline);
}

#[test]
fn test_hash_uniqueness_for_trailing_newline() {
    // This test proves the core of the hashing problem. Two file states that
    // differ on disk (one has a trailing newline, one doesn't) should NEVER
    // produce the same hash.

    let (_tmp_dir, file_path) = setup_test_file("content");

    // State 1: No trailing newline
    let mut state1 = FileState::new(file_path.clone(), "line 1\nline 2");
    state1.ends_with_newline = false;
    let hash1 = FileState::calculate_hash(&state1.get_lif_content_for_hashing());

    // State 2: Trailing newline
    let mut state2 = FileState::new(file_path, "line 1\nline 2");
    state2.ends_with_newline = true; // Manually set for the test
    let hash2 = FileState::calculate_hash(&state2.get_lif_content_for_hashing());

    // The lines collection is identical for both.
    assert_eq!(state1.lines, state2.lines);
    // But the newline flag is different.
    assert_ne!(state1.ends_with_newline, state2.ends_with_newline);

    // This assertion will FAIL until the hash calculation is fixed to include
    // the `ends_with_newline` flag.
    assert_ne!(
        hash1, hash2,
        "BUG: Hashes are identical for files that differ only by a trailing newline."
    );
}

#[test]
fn test_get_lif_string_for_range_past_eof() {
    let (_tmp_dir, file_path) = setup_test_file("1\n2\n3");
    let state = FileState::new(file_path.clone(), "1\n2\n3");

    // Request a range where end_line is past the end of the file. This should not panic.
    let representation = state.get_lif_string_for_ranges(Some(&[RangeSpec {
        start_line: 2,
        end_line: 5, // There are only 3 lines
    }]));

    let project_root = std::env::current_dir().unwrap();
    let relative_path = file_path.strip_prefix(&project_root).unwrap_or(&file_path);
    let short_hash = state.get_short_hash();
    let indexes = get_indexes(&state);

    let expected_header = format!(
        "File: {} | Hash: {} | Lines: 2-5/3",
        relative_path.display(),
        short_hash
    );

    // Note: The body should only contain the lines that actually exist in the range.
    let expected_body = format!("2    {}: 2\n3    {}: 3", indexes[1], indexes[2]);

    assert_eq!(
        representation,
        format!("{expected_header}\n{expected_body}")
    );
}
