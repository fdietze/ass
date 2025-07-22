#![cfg(test)]

use crate::{
    file_state::FileState,
    patch::{InsertOperation, PatchOperation, ReplaceOperation},
};
use console::style;
use std::{collections::BTreeMap, fs, path::PathBuf};
use tempfile::Builder;
fn setup_test_file(content: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp_dir = Builder::new().prefix("test-fs-").tempdir().unwrap();
    let file_path = tmp_dir.path().join("test.txt");
    fs::write(&file_path, content).unwrap();
    (tmp_dir, file_path)
}

#[test]
fn test_file_state_new() {
    let (_tmp_dir, file_path) = setup_test_file("line 1\nline 2");
    let state = FileState::new(file_path, "line 1\nline 2");

    assert_eq!(state.lines.len(), 2);
    assert_eq!(state.lines.get(&1000), Some(&"line 1".to_string()));
    assert_eq!(state.lines.get(&2000), Some(&"line 2".to_string()));

    let expected_lif_content = "LID1000: line 1\nLID2000: line 2";
    let expected_hash = FileState::calculate_hash(expected_lif_content);
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

    let expected_header = format!(
        "File: {} | Hash: {} | Lines: 1-2/2",
        relative_path.display(),
        short_hash
    );
    let expected_body = "1    LID1000: line 1\n2    LID2000: line 2";
    assert_eq!(
        representation,
        format!("{expected_header}\n{expected_body}")
    );
}

#[test]
fn test_get_lif_string_for_range() {
    let (_tmp_dir, file_path) = setup_test_file("1\n2\n3\n4\n5");
    let state = FileState::new(file_path.clone(), "1\n2\n3\n4\n5");

    let partial_representation = state.get_lif_string_for_range(Some(2), Some(4));

    let project_root = std::env::current_dir().unwrap();
    let relative_path = file_path.strip_prefix(&project_root).unwrap_or(&file_path);
    let short_hash = state.get_short_hash();

    let expected_header = format!(
        "File: {} | Hash: {} | Lines: 2-4/5",
        relative_path.display(),
        short_hash
    );
    let expected_body = "2    LID2000: 2\n3    LID3000: 3\n4    LID4000: 4";
    assert_eq!(
        partial_representation,
        format!("{expected_header}\n{expected_body}")
    );
}

#[test]
fn test_apply_and_write_patch() {
    let (_tmp_dir, file_path) = setup_test_file("line 1\nline 3");
    let mut state = FileState::new(file_path.clone(), "line 1\nline 3");

    let patch = vec![PatchOperation::Insert(InsertOperation {
        after_lid: "LID1000".to_string(),
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

    // Check diff
    assert!(diff.contains(&style("+ LID1500: line 2".to_string()).green().to_string()));
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

    assert_eq!(state.lines.len(), 2);
    assert_eq!(state.lines.get(&500), Some(&"new first line".to_string()));
    assert_ne!(state.lif_hash, original_hash);
    assert_eq!(state.get_full_content(), "new first line\nline 1");
}

#[test]
fn test_patch_insert_in_middle() {
    let (_tmp_dir, file_path) = setup_test_file("line 1\nline 2");
    let mut state = FileState::new(file_path, "line 1\nline 2");

    let patch = vec![PatchOperation::Insert(InsertOperation {
        after_lid: "LID1000".to_string(),
        content: vec!["new middle line".to_string()],
        context_before: Some("line 1".to_string()),
        context_after: Some("line 2".to_string()),
    })];
    state.apply_patch(&patch).unwrap();

    assert_eq!(state.lines.len(), 3);
    assert_eq!(state.lines.get(&1500), Some(&"new middle line".to_string()));
    assert_eq!(state.get_full_content(), "line 1\nnew middle line\nline 2");
}

#[test]
fn test_patch_delete_range() {
    let content = "line 1\nline 2\nline 3\nline 4";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: "LID2000".to_string(),
        end_lid: "LID3000".to_string(),
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

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: "LID2000".to_string(),
        end_lid: "LID3000".to_string(),
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
    let json_data = r#"
        [
            {
                "op": "r",
                "start_lid": "LID1000",
                "end_lid": "LID2000",
                "content": ["new content"],
                "context_before": "optional context",
                "context_after": null
            },
            {
                "op": "i",
                "after_lid": "LID3000",
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
            start_lid: "LID1000".to_string(),
            end_lid: "LID2000".to_string(),
            content: vec!["new content".to_string()],
            context_before: Some("optional context".to_string()),
            context_after: None
        })
    );
    assert_eq!(
        operations[1],
        PatchOperation::Insert(InsertOperation {
            after_lid: "LID3000".to_string(),
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
    let patch1 = vec![PatchOperation::Insert(InsertOperation {
        after_lid: "LID1000".to_string(),
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
    let patch2 = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: "LID2000".to_string(),
        end_lid: "LID3000".to_string(),
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
    let patch3 = vec![PatchOperation::Insert(InsertOperation {
        after_lid: "LID1500".to_string(), // This was the LID for "inserted after 1"
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

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: "LID3000".to_string(),
        end_lid: "LID1000".to_string(),
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

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: "LID999".to_string(), // Does not exist
        end_lid: "LID2000".to_string(),
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
            .contains("start_lid 'LID999' does not exist")
    );
}

#[test]
fn test_patch_replace_non_existent_end_lid() {
    let content = "line 1\nline 2";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: "LID1000".to_string(),
        end_lid: "LID9999".to_string(), // Does not exist
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
            .contains("end_lid 'LID9999' does not exist")
    );
}

#[test]
fn test_error_on_lid_space_exhaustion() {
    let mut lines = BTreeMap::new();
    lines.insert(1000, "line 1".to_string());
    lines.insert(1002, "line 2".to_string()); // Only 1 space between LIDs

    // Try to insert 2 lines, which requires 3 slots (step would be 0)
    let result = FileState::generate_new_lids(&lines, "LID1000", 2);
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err().to_string(),
        "Cannot insert 2 lines between LID1000 and LID1002. Not enough space."
    );
}

#[test]
fn test_generate_new_lids_at_start_with_no_space() {
    let mut lines = BTreeMap::new();
    lines.insert(1, "line 1".to_string()); // A very small starting LID

    // Try to insert a line at the start. The step will be 1 / (1+1) = 0.
    let result = FileState::generate_new_lids(&lines, "_START_OF_FILE_", 1);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Not enough space to insert at the beginning")
    );
}

#[test]
fn test_patch_replace_first_line() {
    let content = "line 1\nline 2\nline 3";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let mut state = FileState::new(file_path, content);

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: "LID1000".to_string(),
        end_lid: "LID1000".to_string(),
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

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: "LID3000".to_string(),
        end_lid: "LID3000".to_string(),
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

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: "LID1000".to_string(),
        end_lid: "LID3000".to_string(),
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

    let patch = vec![PatchOperation::Replace(ReplaceOperation {
        start_lid: "LID1000".to_string(),
        end_lid: "LID3000".to_string(),
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

    let patch = vec![PatchOperation::Insert(InsertOperation {
        after_lid: "LID2000".to_string(),
        content: vec!["new last line".to_string()],
        context_before: None,
        context_after: None,
    })];
    state.apply_patch(&patch).unwrap();

    assert_eq!(state.lines.len(), 3);
    // The new lid should be halfway between 2000 and the synthetic next lid (2000 + 1000).
    assert_eq!(state.lines.get(&2500), Some(&"new last line".to_string()));
    assert_eq!(state.get_full_content(), "line 1\nline 2\nnew last line");
}

#[test]
fn test_parse_lid_invalid_formats() {
    assert!(FileState::parse_lid("foo").is_err());
    assert!(FileState::parse_lid("LID").is_err());
    assert!(FileState::parse_lid("LID-123").is_err());
    assert!(FileState::parse_lid("LID123a").is_err());
}

#[test]
fn test_deserialize_malformed_patch_operation() {
    // Unknown operation code
    let json_unknown_op = r#"[{"op": "d", "afterLid": "LID1000"}]"#;
    let result: Result<Vec<PatchOperation>, _> = serde_json::from_str(json_unknown_op);
    assert!(result.is_err());

    // Missing required field for "r"
    let json_missing_field_r = r#"[{"op": "r", "startLid": "LID1000"}]"#;
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

    // A single newline creates two lines: one before, one after.
    assert_eq!(state.lines.len(), 2);
    assert_eq!(state.lines.get(&1000), Some(&"".to_string()));
    assert_eq!(state.lines.get(&2000), Some(&"".to_string()));
    assert_eq!(state.get_full_content(), "\n");
}

#[test]
fn test_file_state_new_with_trailing_newline() {
    let content = "line 1\n";
    let (_tmp_dir, file_path) = setup_test_file(content);
    let state = FileState::new(file_path, content);

    assert_eq!(state.lines.len(), 2);
    assert_eq!(state.lines.get(&1000), Some(&"line 1".to_string()));
    assert_eq!(state.lines.get(&2000), Some(&"".to_string()));
    assert_eq!(state.get_full_content(), "line 1\n");
}
