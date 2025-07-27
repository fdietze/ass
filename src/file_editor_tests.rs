//! Integration tests for the `file_editor` module, focusing on the end-to-end
//! `execute_file_operations` function.

#[cfg(test)]
mod tests {
    use crate::file_editor::*;
    use crate::file_state::FileState;
    use crate::file_state_manager::FileStateManager;

    use std::fs;
    use tempfile::Builder;

    fn setup_test_file(content: &str) -> (tempfile::TempDir, String) {
        let tmp_dir = Builder::new().prefix("test-patcher-").tempdir().unwrap();
        let file_path = tmp_dir.path().join("test_file.txt");
        fs::write(&file_path, content).unwrap();
        (tmp_dir, file_path.to_str().unwrap().to_string())
    }

    // A helper to get a valid LID string (including a random suffix) for a given line index
    fn get_lid_for_line(state: &FileState, line_idx: usize) -> String {
        let (index, (_, suffix)) = state.lines.iter().nth(line_idx).unwrap();
        FileState::display_lid(index, suffix)
    }

    #[test]
    fn test_execute_insert_successfully() {
        let (_tmp_dir, file_path) = setup_test_file("line 1\nline 3");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_path).unwrap();
        let lid = get_lid_for_line(initial_state, 0);

        let request_json = format!(
            r#"{{
                "inserts": [
                    {{
                        "file_path": "{file_path}",
                        "new_content": ["line 2"],
                        "at_position": "after_anchor",
                        "anchor": {{
                            "lid": "{lid}",
                            "line_content": "line 1"
                        }}
                    }}
                ]
            }}"#
        );

        let args: TopLevelRequest = serde_json::from_str(&request_json).unwrap();
        let result = execute_file_operations(&args, &mut manager, &accessible_paths).unwrap();

        assert!(result.contains("Patch from hash"));
        let disk_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(disk_content, "line 1\nline 2\nline 3");
    }

    #[test]
    fn test_execute_insert_with_invalid_anchor_content_fails() {
        let (_tmp_dir, file_path) = setup_test_file("line 1\nline 3");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_path).unwrap();
        let lid = get_lid_for_line(initial_state, 0);

        let request_json = format!(
            r#"{{
                "inserts": [
                    {{
                        "file_path": "{file_path}",
                        "new_content": ["line 2"],
                        "at_position": "after_anchor",
                        "anchor": {{
                            "lid": "{lid}",
                            "line_content": "WRONG content"
                        }}
                    }}
                ]
            }}"#
        );

        let args: TopLevelRequest = serde_json::from_str(&request_json).unwrap();
        let result = execute_file_operations(&args, &mut manager, &accessible_paths);

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.to_string().contains("anchor content mismatch"));
        assert!(
            error
                .to_string()
                .contains("Expected 'WRONG content', found 'line 1'")
        );

        let disk_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(disk_content, "line 1\nline 3");
    }

    #[test]
    fn test_execute_replace_successfully() {
        let (_tmp_dir, file_path) = setup_test_file("one\ntwo\nthree");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_path).unwrap();
        let lid = get_lid_for_line(initial_state, 1);

        let request_json = format!(
            r#"{{
                "replaces": [
                    {{
                        "file_path": "{file_path}",
                        "range_start_anchor": {{
                            "lid": "{lid}",
                            "line_content": "two"
                        }},
                        "range_end_anchor": {{
                            "lid": "{lid}",
                            "line_content": "two"
                        }},
                        "new_content": ["2"]
                    }}
                ]
            }}"#
        );

        let args: TopLevelRequest = serde_json::from_str(&request_json).unwrap();
        let result = execute_file_operations(&args, &mut manager, &accessible_paths).unwrap();

        assert!(result.contains("Patch from hash"));
        let disk_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(disk_content, "one\n2\nthree");
    }

    #[test]
    fn test_execute_replace_with_messy_whitespace_anchor_succeeds() {
        let (_tmp_dir, file_path) = setup_test_file("one\n  let x = 1;\nthree");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_path).unwrap();
        let lid = get_lid_for_line(initial_state, 1);

        // Note the messy whitespace in the anchor content, which should be ignored
        let request_json = format!(
            r#"{{
                "replaces": [
                    {{
                        "file_path": "{file_path}",
                        "range_start_anchor": {{
                            "lid": "{lid}",
                            "line_content": "  let   x    =   1;  "
                        }},
                        "range_end_anchor": {{
                            "lid": "{lid}",
                            "line_content": "let x =    1;"
                        }},
                        "new_content": ["let y = 2;"]
                    }}
                ]
            }}"#
        );

        let args: TopLevelRequest = serde_json::from_str(&request_json).unwrap();
        let result = execute_file_operations(&args, &mut manager, &accessible_paths).unwrap();

        assert!(result.contains("Patch from hash"));
        let disk_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(disk_content, "one\nlet y = 2;\nthree");
    }

    #[test]
    fn test_execute_replace_with_collapsed_whitespace_still_fails_on_content_mismatch() {
        let (_tmp_dir, file_path) = setup_test_file("one\nlet x = 1;\nthree");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_path).unwrap();
        let lid = get_lid_for_line(initial_state, 1);

        // The content is genuinely different, not just whitespace.
        let request_json = format!(
            r#"{{
                "replaces": [
                    {{
                        "file_path": "{file_path}",
                        "range_start_anchor": {{
                            "lid": "{lid}",
                            "line_content": "let y = 2;"
                        }},
                        "range_end_anchor": {{
                            "lid": "{lid}",
                            "line_content": "let y = 2;"
                        }},
                        "new_content": ["..."]
                    }}
                ]
            }}"#
        );

        let args: TopLevelRequest = serde_json::from_str(&request_json).unwrap();
        let result = execute_file_operations(&args, &mut manager, &accessible_paths);

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.to_string().contains("anchor content mismatch"));
        // Check that the error message contains the original, un-collapsed content
        assert!(
            error
                .to_string()
                .contains("Expected 'let y = 2;', found 'let x = 1;'")
        );
    }

    #[test]
    fn test_execute_delete_successfully() {
        let (_tmp_dir, file_path) = setup_test_file("one\ntwo\nthree");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_path).unwrap();
        let lid = get_lid_for_line(initial_state, 1);

        let request_json = format!(
            r#"{{
                "replaces": [
                    {{
                        "file_path": "{file_path}",
                        "range_start_anchor": {{
                            "lid": "{lid}",
                            "line_content": "two"
                        }},
                        "range_end_anchor": {{
                            "lid": "{lid}",
                            "line_content": "two"
                        }},
                        "new_content": []
                    }}
                ]
            }}"#
        );

        let args: TopLevelRequest = serde_json::from_str(&request_json).unwrap();
        execute_file_operations(&args, &mut manager, &accessible_paths).unwrap();

        let disk_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(disk_content, "one\nthree");
    }

    #[test]
    fn test_execute_move_successfully() {
        let (_tmp_dir, source_path) = setup_test_file("source line 1\nline to move\nsource line 3");
        let (_tmp_dir2, dest_path) = setup_test_file("dest line 1");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![
            _tmp_dir.path().to_str().unwrap().to_string(),
            _tmp_dir2.path().to_str().unwrap().to_string(),
        ];

        let source_state = manager.open_file(&source_path).unwrap();
        let source_lid_to_move = get_lid_for_line(source_state, 1);
        let original_suffix = source_lid_to_move
            .split('_')
            .next_back()
            .unwrap()
            .to_string();

        let dest_state = manager.open_file(&dest_path).unwrap();
        let dest_lid = get_lid_for_line(dest_state, 0);

        let request_json = format!(
            r#"{{
                "moves": [
                    {{
                        "source_file_path": "{source_path}",
                        "source_range_start_anchor": {{
                            "lid": "{source_lid_to_move}",
                            "line_content": "line to move"
                        }},
                        "source_range_end_anchor": {{
                            "lid": "{source_lid_to_move}",
                            "line_content": "line to move"
                        }},
                        "dest_file_path": "{dest_path}",
                        "dest_at_position": "after_anchor",
                        "dest_anchor": {{
                            "lid": "{dest_lid}",
                            "line_content": "dest line 1"
                        }}
                    }}
                ]
            }}"#
        );

        let args: TopLevelRequest = serde_json::from_str(&request_json).unwrap();
        execute_file_operations(&args, &mut manager, &accessible_paths).unwrap();

        let source_content = fs::read_to_string(&source_path).unwrap();
        assert_eq!(source_content, "source line 1\nsource line 3");

        let dest_content = fs::read_to_string(&dest_path).unwrap();
        assert_eq!(dest_content, "dest line 1\nline to move");

        // Verify that the suffix of the moved line was preserved.
        let final_dest_state = manager.open_file(&dest_path).unwrap();
        let moved_line_lid = get_lid_for_line(final_dest_state, 1);
        assert!(moved_line_lid.ends_with(&original_suffix));
    }

    #[test]
    fn test_execute_insert_at_start_and_end() {
        let (_tmp_dir, file_path) = setup_test_file("middle");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let request_json = format!(
            r#"{{
                "inserts": [
                    {{
                        "file_path": "{file_path}",
                        "new_content": ["start"],
                        "at_position": "start_of_file"
                    }},
                    {{
                        "file_path": "{file_path}",
                        "new_content": ["end"],
                        "at_position": "end_of_file"
                    }}
                ]
            }}"#
        );

        let args: TopLevelRequest = serde_json::from_str(&request_json).unwrap();
        execute_file_operations(&args, &mut manager, &accessible_paths).unwrap();

        let disk_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(disk_content, "start\nmiddle\nend");
    }

    #[test]
    fn test_execute_mixed_batch_with_one_failure_aborts_all() {
        let (_tmp_dir, file1_path) = setup_test_file("file1 line1");
        let (_tmp_dir2, file2_path) = setup_test_file("file2 line1");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![
            _tmp_dir.path().to_str().unwrap().to_string(),
            _tmp_dir2.path().to_str().unwrap().to_string(),
        ];

        let file1_state = manager.open_file(&file1_path).unwrap();
        let file1_lid = get_lid_for_line(file1_state, 0);

        // This request includes a valid insert and an invalid replace.
        // The entire batch should fail.
        let request = TopLevelRequest {
            inserts: vec![InsertRequest {
                file_path: file1_path.clone(),
                new_content: vec!["file1 line2".to_string()],
                at_position: Position::AfterAnchor,
                anchor: Some(Anchor {
                    lid: file1_lid,
                    line_content: "file1 line1".to_string(),
                }),
            }],
            replaces: vec![ReplaceRequest {
                file_path: file2_path.clone(),
                range_start_anchor: Anchor {
                    lid: "lid-bad1_xxx".to_string(), // Invalid LID
                    line_content: "THIS IS WRONG".to_string(),
                },
                range_end_anchor: Anchor {
                    lid: "lid-bad2_yyy".to_string(), // Invalid LID
                    line_content: "THIS IS WRONG".to_string(),
                },
                new_content: vec!["...".to_string()],
            }],
            moves: vec![],
            copies: vec![],
        };

        let result = execute_file_operations(&request, &mut manager, &accessible_paths);

        assert!(result.is_err());

        // Check that the error message contains the expected failure.
        let error = result.unwrap_err();
        assert!(error.to_string().contains("Replace request #0"));
        assert!(
            error
                .to_string()
                .contains("Invalid FractionalIndex format in LID: 'bad1'")
        );

        // Verify no changes were made to either file
        let file1_content = fs::read_to_string(&file1_path).unwrap();
        assert_eq!(file1_content, "file1 line1");
        let file2_content = fs::read_to_string(&file2_path).unwrap();
        assert_eq!(file2_content, "file2 line1");
    }

    #[test]
    fn test_execute_fails_with_unprefixed_lid() {
        let (_tmp_dir, file_path) = setup_test_file("line 1");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_path).unwrap();
        let lid_line_1_index = initial_state.lines.keys().next().unwrap().to_string();

        let request_json = format!(
            r#"{{
                "inserts": [
                    {{
                        "file_path": "{file_path}",
                        "new_content": ["line 2"],
                        "at_position": "after_anchor",
                        "anchor": {{
                            "lid": "{lid_line_1_index}",
                            "line_content": "line 1"
                        }}
                    }}
                ]
            }}"#
        );

        let args: TopLevelRequest = serde_json::from_str(&request_json).unwrap();
        let result = execute_file_operations(&args, &mut manager, &accessible_paths);

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Invalid LID format: must start with 'lid-'")
        );
    }

    #[test]
    fn test_execute_fails_with_invalid_suffix() {
        let (_tmp_dir, file_path) = setup_test_file("line 1");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_path).unwrap();
        let (index, _) = initial_state.lines.iter().next().unwrap();
        // Construct a LID with the correct index but a deliberately wrong suffix
        let invalid_lid = format!("lid-{}_xxxx", index.to_string());

        let request_json = format!(
            r#"{{
                "inserts": [
                    {{
                        "file_path": "{file_path}",
                        "new_content": ["line 2"],
                        "at_position": "after_anchor",
                        "anchor": {{
                            "lid": "{invalid_lid}",
                            "line_content": "line 1"
                        }}
                    }}
                ]
            }}"#
        );

        let args: TopLevelRequest = serde_json::from_str(&request_json).unwrap();
        let result = execute_file_operations(&args, &mut manager, &accessible_paths);

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.to_string().contains("anchor suffix mismatch"));
    }
}

use crate::file_editor::{
    Anchor, CopyRequest, InsertRequest, MoveRequest, Position, ReplaceRequest, TopLevelRequest,
    execute_file_operations,
};
use crate::file_state::FileState;
use crate::file_state_manager::FileStateManager;
use anyhow::Result;
use std::fs;
use tempfile::tempdir;

fn get_lid(file_state: &FileState, line_index: usize) -> String {
    let (lid, (_, suffix)) = file_state.lines.iter().nth(line_index).unwrap();
    format!("lid-{}_{}", lid.to_string(), suffix)
}

#[test]
fn execute_operations_with_multiple_failures_reports_all_errors() -> Result<()> {
    // 1. Setup
    let tmp_dir = tempdir()?;
    let mut file_state_manager = FileStateManager::new();

    let file_path1 = tmp_dir.path().join("test1.txt");
    fs::write(&file_path1, "line one\nline two")?;
    let accessible_paths = vec![
        file_path1.to_str().unwrap().to_string(),
        tmp_dir
            .path()
            .join("test2.txt")
            .to_str()
            .unwrap()
            .to_string(),
    ];
    let file_path1_str = file_path1.to_str().unwrap();

    // Create file 1
    let file_state1_ro = file_state_manager.open_file(file_path1_str)?;
    let lid1_1 = get_lid(file_state1_ro, 0); // lid for "line one"
    let lid1_2 = get_lid(file_state1_ro, 1); // lid for "line two"

    // Create file 2
    let file_path2 = tmp_dir.path().join("test2.txt");
    fs::write(&file_path2, "another line")?;
    let file_path2_str = file_path2.to_str().unwrap();
    let _ = file_state_manager.open_file(file_path2_str)?;

    // 2. Define a request with multiple invalid operations
    let args = TopLevelRequest {
        // --- VALID --- a simple insert to ensure valid ops are ignored when others fail
        inserts: vec![
            // --- FAILURE 2 ---
            InsertRequest {
                file_path: file_path1_str.to_string(),
                new_content: vec!["a new line".to_string()],
                at_position: Position::AfterAnchor,
                anchor: Some(Anchor {
                    lid: "lid-9999-xyz".to_string(), // Invalid LID
                    line_content: "line two".to_string(),
                }),
            },
        ],
        replaces: vec![
            // --- FAILURE 1 ---
            ReplaceRequest {
                file_path: file_path1_str.to_string(),
                range_start_anchor: Anchor {
                    lid: lid1_1.clone(),
                    line_content: "WRONG content".to_string(), // Invalid content
                },
                range_end_anchor: Anchor {
                    lid: lid1_1.clone(),
                    line_content: "line one".to_string(),
                },
                new_content: vec![],
            },
        ],
        moves: vec![
            // --- FAILURE 4 ---
            MoveRequest {
                source_file_path: file_path1_str.to_string(),
                source_range_start_anchor: Anchor {
                    lid: lid1_2.clone(),
                    line_content: "line two".to_string(),
                },
                source_range_end_anchor: Anchor {
                    lid: lid1_2.clone(),
                    line_content: "WRONG content for move".to_string(), // Invalid content
                },
                dest_file_path: file_path2_str.to_string(),
                dest_at_position: Position::EndOfFile,
                dest_anchor: None,
            },
        ],
        copies: vec![
            // --- FAILURE 3 ---
            CopyRequest {
                source_file_path: "non_existent_file.txt".to_string(), // Invalid file
                source_range_start_anchor: Anchor {
                    lid: "lid-0000_xxx".to_string(),
                    line_content: "any".to_string(),
                },
                source_range_end_anchor: Anchor {
                    lid: "lid-0000_yyy".to_string(),
                    line_content: "any".to_string(),
                },
                dest_file_path: file_path2_str.to_string(),
                dest_at_position: Position::StartOfFile,
                dest_anchor: None,
            },
        ],
    };

    // 3. Execution & Assertions
    let result = execute_file_operations(&args, &mut file_state_manager, &accessible_paths);

    // Assert that the function returns an Err
    assert!(result.is_err());

    if let Err(e) = result {
        let error_string = e.to_string();
        println!("Error string: {error_string}");

        // Check that the error message contains the expected number of errors
        assert!(error_string.starts_with("Validation failed with 4 error(s):"));

        // Check for specific failure messages
        assert!(error_string.contains("Copy request #0 (source: 'non_existent_file.txt')"));
        assert!(error_string.contains("Operation on path 'non_existent_file.txt' is not allowed."));

        assert!(error_string.contains(&format!(
            "Move request #0 (source: '{file_path1_str}', dest: '{file_path2_str}')"
        ),));
        assert!(error_string.contains("source_range_end_anchor content mismatch"));

        assert!(error_string.contains(&format!("Replace request #0 (file: '{file_path1_str}')")));
        assert!(error_string.contains("range_start_anchor content mismatch"));

        assert!(error_string.contains(&format!("Insert request #0 (file: '{file_path1_str}')")));
        assert!(
            error_string
                .contains("Invalid LID format: must be 'lid-index_suffix'. Got: 'lid-9999-xyz'")
        );
    } else {
        panic!("Expected an error, but got Ok");
    }

    // Assert that no changes were written to the files
    let file1_content = file_state_manager
        .open_file(file_path1_str)?
        .get_full_content();
    assert_eq!(file1_content, "line one\nline two");

    let file2_content = file_state_manager
        .open_file(file_path2_str)?
        .get_full_content();
    assert_eq!(file2_content, "another line");

    Ok(())
}
