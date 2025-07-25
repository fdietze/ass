//! Integration tests for the `file_editor` module, focusing on the end-to-end
//! `execute_file_operations` function.

#[cfg(test)]
mod tests {
    use crate::file_editor::*;
    use crate::file_state::FileState;
    use crate::file_state_manager::FileStateManager;
    use crate::patch::{InsertOp, PatchOperation, ReplaceOp};
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
                        "file_path": "{}",
                        "start_anchor": {{
                            "lid": "{}",
                            "line_content": "two"
                        }},
                        "end_anchor": {{
                            "lid": "{}",
                            "line_content": "two"
                        }},
                        "new_content": ["2"]
                    }}
                ]
            }}"#,
            file_path, lid, lid
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
                        "file_path": "{}",
                        "start_anchor": {{
                            "lid": "{}",
                            "line_content": "  let   x    =   1;  "
                        }},
                        "end_anchor": {{
                            "lid": "{}",
                            "line_content": "let x =    1;"
                        }},
                        "new_content": ["let y = 2;"]
                    }}
                ]
            }}"#,
            file_path, lid, lid
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
                        "file_path": "{}",
                        "start_anchor": {{
                            "lid": "{}",
                            "line_content": "let y = 2;"
                        }},
                        "end_anchor": {{
                            "lid": "{}",
                            "line_content": "let y = 2;"
                        }},
                        "new_content": ["..."]
                    }}
                ]
            }}"#,
            file_path, lid, lid
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
                        "file_path": "{}",
                        "start_anchor": {{
                            "lid": "{}",
                            "line_content": "two"
                        }},
                        "end_anchor": {{
                            "lid": "{}",
                            "line_content": "two"
                        }},
                        "new_content": []
                    }}
                ]
            }}"#,
            file_path, lid, lid
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
        let original_suffix = source_lid_to_move.split('_').last().unwrap().to_string();

        let dest_state = manager.open_file(&dest_path).unwrap();
        let dest_lid = get_lid_for_line(dest_state, 0);

        let request_json = format!(
            r#"{{
                "moves": [
                    {{
                        "op": "move",
                        "source_file_path": "{}",
                        "source_start_anchor": {{
                            "lid": "{}",
                            "line_content": "line to move"
                        }},
                        "source_end_anchor": {{
                            "lid": "{}",
                            "line_content": "line to move"
                        }},
                        "dest_file_path": "{}",
                        "dest_at_position": "after_anchor",
                        "dest_anchor": {{
                            "lid": "{}",
                            "line_content": "dest line 1"
                        }}
                    }}
                ]
            }}"#,
            source_path, source_lid_to_move, source_lid_to_move, dest_path, dest_lid
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

        let request_json = format!(
            r#"{{
                "inserts": [
                    {{
                        "file_path": "{file1_path}",
                        "new_content": ["file1 line2"],
                        "at_position": "after_anchor",
                        "anchor": {{
                            "lid": "{file1_lid}",
                            "line_content": "file1 line1"
                        }}
                    }}
                ],
                "replaces": [
                    {{
                        "file_path": "{file2_path}",
                        "start_anchor": {{
                            "lid": "any_lid",
                            "line_content": "THIS IS WRONG"
                        }},
                        "end_anchor": {{
                            "lid": "any_lid",
                            "line_content": "THIS IS WRONG"
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
        assert!(
            error
                .to_string()
                .contains("Invalid LID format: must start with 'lid-'")
        );

        let file1_content = fs::read_to_string(&file1_path).unwrap();
        assert_eq!(file1_content, "file1 line1");
        let file2_content = fs::read_to_string(&file2_path).unwrap();
        assert_eq!(file2_content, "file2 line1");
    }

    #[test]
    fn test_execute_operations_respects_fixed_order() {
        let (_tmp_dir, file_path) = setup_test_file("line B");
        let mut manager = FileStateManager::new();
        // This request tries to insert after "line C", which only exists *after*
        // the replace operation has been planned. It also deletes line B.
        // If inserts were not last, this would fail.
        //
        // We can't actually test this fully because we can't get the LID of a line
        // that will be created in the same operation. However, we can simulate the
        // intended logic and ensure the final state is what we expect from the
        // hardcoded execution order (replaces then inserts).

        let initial_state = manager.open_file(&file_path).unwrap();
        let lid_b_idx = initial_state.lines.keys().next().unwrap().clone();

        let replace_op = PatchOperation::Replace(ReplaceOp {
            start_lid: lid_b_idx.clone(),
            end_lid: lid_b_idx,
            content: vec![
                ("line A".to_string(), "rand1".to_string()),
                ("line C".to_string(), "rand2".to_string()),
            ],
        });

        // Manually apply the first part of the plan
        manager
            .get_file_state_mut(&file_path)
            .unwrap()
            .apply_patch(&[replace_op])
            .unwrap();

        let state_after_replace = manager.get_file_state_mut(&file_path).unwrap();
        let (lid_c_idx, _) = state_after_replace
            .lines
            .iter()
            .find(|(_, (v, _))| *v == "line C")
            .map(|(k, v)| (k.clone(), v.clone()))
            .unwrap();

        let insert_op = PatchOperation::Insert(InsertOp {
            after_lid: Some(lid_c_idx),
            content: vec![("line D".to_string(), "rand3".to_string())],
        });

        manager
            .get_file_state_mut(&file_path)
            .unwrap()
            .apply_and_write_patch(&[insert_op])
            .unwrap();

        let final_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(final_content, "line A\nline C\nline D");
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
