#[cfg(test)]
mod tests {
    use crate::file_editor::*;
    use crate::file_state_manager::FileStateManager;
    use crate::patch::{InsertOperation, PatchOperation, ReplaceOperation};
    use std::fs;
    use tempfile::Builder;

    fn setup_test_file(content: &str) -> (tempfile::TempDir, String) {
        let tmp_dir = Builder::new().prefix("test-patcher-").tempdir().unwrap();
        let file_path = tmp_dir.path().join("test_file.txt");
        fs::write(&file_path, content).unwrap();
        (tmp_dir, file_path.to_str().unwrap().to_string())
    }

    #[test]
    fn test_execute_single_patch_successfully() {
        let (_tmp_dir, file_path) = setup_test_file("line 1\nline 3");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_path).unwrap();
        let initial_short_hash = initial_state.get_short_hash().to_string();

        let args = FileOperationArgs {
            edits: vec![PatchArgs {
                file_path: file_path.clone(),
                lif_hash: initial_short_hash.clone(),
                patch: vec![PatchOperation::Insert(InsertOperation {
                    after_lid: "LID1000".to_string(),
                    content: vec!["line 2".to_string()],
                    context_before: Some("line 1".to_string()),
                    context_after: Some("line 3".to_string()),
                })],
            }],
            copies: vec![],
            moves: vec![],
        };

        let result = execute_file_operations(&args, &mut manager, &accessible_paths);
        assert!(result.is_ok());

        let output = result.unwrap();
        assert!(output.contains("Patch from hash"));
        assert!(output.contains(&file_path));

        let disk_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(disk_content, "line 1\nline 2\nline 3");

        let final_state = manager.open_file(&file_path).unwrap();
        let final_short_hash = final_state.get_short_hash();
        assert_ne!(final_short_hash, initial_short_hash);
        assert!(output.contains(&format!("New lif_hash: {final_short_hash}")));
    }

    #[test]
    fn test_execute_patch_hash_mismatch() {
        let (_tmp_dir, file_path) = setup_test_file("line 1");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let args = FileOperationArgs {
            edits: vec![PatchArgs {
                file_path: file_path.clone(),
                lif_hash: "wrong_hash".to_string(),
                patch: vec![],
            }],
            copies: vec![],
            moves: vec![],
        };

        let result = execute_file_operations(&args, &mut manager, &accessible_paths);
        assert!(result.is_ok()); // The function itself doesn't error, it reports errors in the string
        let output = result.unwrap();
        assert!(output.contains("Error: Hash mismatch"));
        assert!(output.contains(&file_path));
    }

    #[test]
    fn test_execute_multiple_patches_with_partial_failure() {
        // --- Setup ---
        let (_tmp_dir, file1_path) = setup_test_file("file1 line1");
        let file2_path = _tmp_dir.path().join("file2.txt");
        fs::write(&file2_path, "file2 line1").unwrap();
        let file2_path_str = file2_path.to_str().unwrap().to_string();

        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let file1_initial_state = manager.open_file(&file1_path).unwrap();
        let file1_initial_hash = file1_initial_state.get_short_hash().to_string();

        let file2_initial_state = manager.open_file(&file2_path_str).unwrap();
        let file2_initial_hash = file2_initial_state.get_short_hash().to_string();

        // --- Args ---
        // Edit for file1 is valid
        let valid_edit = PatchArgs {
            file_path: file1_path.clone(),
            lif_hash: file1_initial_hash,
            patch: vec![PatchOperation::Insert(InsertOperation {
                after_lid: "LID1000".to_string(),
                content: vec!["file1 line2".to_string()],
                context_before: Some("file1 line1".to_string()),
                context_after: None,
            })],
        };
        // Edit for file2 has a hash mismatch
        let invalid_edit = PatchArgs {
            file_path: file2_path_str.clone(),
            lif_hash: "wrong_hash".to_string(),
            patch: vec![],
        };

        let args = FileOperationArgs {
            edits: vec![valid_edit, invalid_edit],
            copies: vec![],
            moves: vec![],
        };

        // --- Act ---
        let result = execute_file_operations(&args, &mut manager, &accessible_paths).unwrap();

        // --- Assert ---
        // Check final string report
        assert!(result.contains(&format!("File: {file1_path}")));
        assert!(result.contains("Patch from hash"));
        assert!(result.contains(&format!("File: {file2_path_str}")));
        assert!(result.contains("Error: Hash mismatch"));

        // Check file1 on disk (should be changed)
        let file1_content = fs::read_to_string(&file1_path).unwrap();
        assert_eq!(file1_content, "file1 line1\nfile1 line2");

        // Check file2 on disk (should NOT be changed)
        let file2_content = fs::read_to_string(&file2_path).unwrap();
        assert_eq!(file2_content, "file2 line1");

        // Check hashes in manager
        let file1_final_hash = manager.open_file(&file1_path).unwrap().get_short_hash();
        assert!(result.contains(file1_final_hash));

        let file2_final_hash = manager.open_file(&file2_path_str).unwrap().get_short_hash();
        assert_eq!(file2_final_hash, file2_initial_hash); // Should be unchanged
    }

    #[test]
    fn test_execute_patch_with_wrong_context() {
        let (_tmp_dir, file_path) = setup_test_file("line 1\nline 2\nline 3");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_path).unwrap();
        let initial_short_hash = initial_state.get_short_hash().to_string();

        let args = FileOperationArgs {
            edits: vec![PatchArgs {
                file_path: file_path.clone(),
                lif_hash: initial_short_hash.clone(),
                patch: vec![PatchOperation::Insert(InsertOperation {
                    after_lid: "LID2000".to_string(),
                    content: vec!["new line".to_string()],
                    context_before: Some("line 1".to_string()), // This is wrong
                    context_after: Some("line 3".to_string()),
                })],
            }],
            copies: vec![],
            moves: vec![],
        };

        let result = execute_file_operations(&args, &mut manager, &accessible_paths).unwrap();
        assert!(result.contains("Error: ContextBefore mismatch"));

        // Ensure file was not changed
        let disk_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(disk_content, "line 1\nline 2\nline 3");
    }

    #[test]
    fn test_execute_patch_with_lid_in_context() {
        let (_tmp_dir, file_path) = setup_test_file("line 1\nline 3");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_path).unwrap();
        let initial_short_hash = initial_state.get_short_hash().to_string();

        // The AI provides context with the LID prefix, which should now be handled.
        let args = FileOperationArgs {
            edits: vec![PatchArgs {
                file_path: file_path.clone(),
                lif_hash: initial_short_hash.clone(),
                patch: vec![PatchOperation::Insert(InsertOperation {
                    after_lid: "LID1000".to_string(),
                    content: vec!["line 2".to_string()],
                    context_before: Some("LID1000: line 1".to_string()), // Incorrectly includes LID
                    context_after: Some("LID2000: line 3".to_string()),  // Incorrectly includes LID
                })],
            }],
            copies: vec![],
            moves: vec![],
        };

        let result = execute_file_operations(&args, &mut manager, &accessible_paths);
        assert!(result.is_ok(), "Operation should succeed");

        let output = result.unwrap();
        assert!(
            !output.contains("Error:"),
            "Result should not contain an error message, but was: {output}"
        );
        assert!(output.contains("Patch from hash"));

        let disk_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(disk_content, "line 1\nline 2\nline 3");
    }

    #[test]
    fn test_execute_patch_with_whitespace_mismatch_in_context() {
        // Setup a file with extra whitespace
        let (_tmp_dir, file_path) = setup_test_file("line 1\n\n  line 3  ");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_path).unwrap();
        let initial_short_hash = initial_state.get_short_hash().to_string();

        // The AI provides context with clean whitespace. This should succeed.
        let args = FileOperationArgs {
            edits: vec![PatchArgs {
                file_path: file_path.clone(),
                lif_hash: initial_short_hash.clone(),
                patch: vec![PatchOperation::Replace(ReplaceOperation {
                    start_lid: "LID2000".to_string(), // The empty line
                    end_lid: "LID2000".to_string(),
                    content: vec!["line 2".to_string()],
                    context_before: Some("line 1".to_string()),
                    context_after: Some("line 3".to_string()),
                })],
            }],
            copies: vec![],
            moves: vec![],
        };

        let result = execute_file_operations(&args, &mut manager, &accessible_paths);
        assert!(result.is_ok(), "Operation should succeed");

        let output = result.unwrap();
        assert!(
            !output.contains("Error:"),
            "Result should not contain an error message, but was: {output}"
        );

        let disk_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(disk_content, "line 1\nline 2\n  line 3  ");
    }

    #[test]
    fn test_copy_intra_file() {
        let (_tmp_dir, file_path) = setup_test_file("line 1\nline 2\nline 3");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_path).unwrap();
        let initial_hash = initial_state.get_short_hash().to_string();

        let args = FileOperationArgs {
            copies: vec![CopyArgs {
                source_file_path: file_path.clone(),
                source_lif_hash: initial_hash.clone(),
                source_start_lid: "LID1000".to_string(),
                source_end_lid: "LID1000".to_string(),
                dest_file_path: file_path.clone(),
                dest_lif_hash: initial_hash,
                dest_after_lid: "LID3000".to_string(),
            }],
            edits: vec![],
            moves: vec![],
        };

        let result = execute_file_operations(&args, &mut manager, &accessible_paths).unwrap();
        assert!(!result.contains("Error:"), "Operation should succeed");

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "line 1\nline 2\nline 3\nline 1");
    }

    #[test]
    fn test_move_inter_file_and_chain_edit() {
        let (_tmp_dir, source_path) = setup_test_file("... irrelevant ...\nfn my_func() {}\n...");
        let (_tmp_dir2, dest_path) = setup_test_file("mod helper;");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![
            _tmp_dir.path().to_str().unwrap().to_string(),
            _tmp_dir2.path().to_str().unwrap().to_string(),
        ];

        let source_hash = manager
            .open_file(&source_path)
            .unwrap()
            .get_short_hash()
            .to_string();
        let dest_hash = manager
            .open_file(&dest_path)
            .unwrap()
            .get_short_hash()
            .to_string();

        let args = FileOperationArgs {
            moves: vec![MoveArgs {
                source_file_path: source_path.clone(),
                source_lif_hash: source_hash,
                source_start_lid: "LID2000".to_string(),
                source_end_lid: "LID2000".to_string(),
                dest_file_path: dest_path.clone(),
                dest_lif_hash: dest_hash.clone(),
                dest_after_lid: "LID1000".to_string(),
            }],
            edits: vec![PatchArgs {
                file_path: dest_path.clone(),
                lif_hash: dest_hash, // Note: using the *original* hash
                patch: vec![PatchOperation::Replace(ReplaceOperation {
                    start_lid: "LID1500".to_string(), // LID of the moved function
                    end_lid: "LID1500".to_string(),
                    content: vec!["pub fn my_func() {}".to_string()], // Make it public
                    context_before: None,
                    context_after: None,
                })],
            }],
            copies: vec![],
        };

        let result = execute_file_operations(&args, &mut manager, &accessible_paths).unwrap();
        assert!(
            !result.contains("Error:"),
            "Operation should succeed, but got {result}"
        );

        let source_content = fs::read_to_string(&source_path).unwrap();
        assert_eq!(source_content, "... irrelevant ...\n...");

        let dest_content = fs::read_to_string(&dest_path).unwrap();
        assert_eq!(dest_content, "mod helper;\npub fn my_func() {}");
    }
}
