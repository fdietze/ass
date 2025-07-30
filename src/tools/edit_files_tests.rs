//! # File Editor Tests

use super::{Anchor, FileEditorTool};
use crate::{
    config::Config, file_state::FileState, file_state_manager::FileStateManager, tools::Tool,
};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tempfile::Builder;

// Helper to create an anchor from a FileState and a line index
fn get_anchor(state: &FileState, line_index: usize) -> Anchor {
    let (lid_key, (content, suffix)) = state.lines.iter().nth(line_index).unwrap();
    Anchor {
        lid: FileState::display_lid(lid_key, suffix),
        line_content: content.clone(),
    }
}

// Helper to set up the FileStateManager and Config for a test
fn setup_fsm(
    content: &str,
) -> (
    tempfile::TempDir,
    PathBuf,
    Arc<Mutex<FileStateManager>>,
    Config,
) {
    let tmp_dir = Builder::new().prefix("test-fsm-").tempdir().unwrap();
    let file_path = tmp_dir.path().join("test.txt");
    fs::write(&file_path, content).unwrap();

    let accessible_paths = vec![tmp_dir.path().to_str().unwrap().to_string()];
    let config = Config {
        accessible_paths,
        ..Default::default()
    };
    let fsm = Arc::new(Mutex::new(FileStateManager::new()));

    (tmp_dir, file_path, fsm, config)
}

#[tokio::test]
async fn test_execute_replace_successfully() {
    let (_tmp_dir, file_path, fsm, config) = setup_fsm("line 1\nline 2\nline 3\nline 4");
    let file_path_str = file_path.to_str().unwrap().to_string();

    let tool = FileEditorTool;

    // Prime the FSM
    fsm.lock().unwrap().open_file(&file_path_str).unwrap();
    let state = fsm.lock().unwrap().open_files[&file_path_str].clone();
    let anchor1 = get_anchor(&state, 1); // line 2
    let anchor2 = get_anchor(&state, 2); // line 3

    let args = serde_json::json!({
        "replaces": [{
            "file_path": file_path_str,
            "anchor_range_begin": anchor1,
            "anchor_range_end": anchor2,
            "new_content": ["new middle"]
        }],
        "inserts": [],
        "moves": []
    });

    let result = tool.execute(&args, &config, fsm).await.unwrap();

    assert!(result.contains("Patch from hash"));
    let final_content = fs::read_to_string(&file_path).unwrap();
    assert_eq!(final_content, "line 1\nnew middle\nline 4");
}

#[tokio::test]
async fn test_execute_delete_successfully() {
    let (_tmp_dir, file_path, fsm, config) = setup_fsm("line 1\nline 2\nline 3");
    let file_path_str = file_path.to_str().unwrap().to_string();

    let tool = FileEditorTool;
    fsm.lock().unwrap().open_file(&file_path_str).unwrap();
    let state = fsm.lock().unwrap().open_files[&file_path_str].clone();
    let anchor_before = get_anchor(&state, 1); // line 2
    let anchor_after = get_anchor(&state, 1); // line 2

    let args = serde_json::json!({
        "replaces": [{
            "file_path": file_path_str,
            "anchor_range_begin": anchor_before,
            "anchor_range_end": anchor_after,
            "new_content": []
        }],
        "inserts": [],
        "moves": []
    });

    tool.execute(&args, &config, fsm).await.unwrap();
    let final_content = fs::read_to_string(&file_path).unwrap();
    assert_eq!(final_content, "line 1\nline 3");
}

#[tokio::test]
async fn test_execute_insert_before_anchor() {
    let (_tmp_dir, file_path, fsm, config) = setup_fsm("line 1\nline 3");
    let file_path_str = file_path.to_str().unwrap().to_string();

    let tool = FileEditorTool;
    fsm.lock().unwrap().open_file(&file_path_str).unwrap();
    let state = fsm.lock().unwrap().open_files[&file_path_str].clone();
    let anchor = get_anchor(&state, 1); // line 3

    let args = serde_json::json!({
        "replaces": [],
        "inserts": [{
            "file_path": file_path_str,
            "at_position": "before_anchor",
            "context_anchor": anchor,
            "new_content": ["line 2"]
        }],
        "moves": []
    });

    tool.execute(&args, &config, fsm).await.unwrap();
    let final_content = fs::read_to_string(&file_path).unwrap();
    assert_eq!(final_content, "line 1\nline 2\nline 3");
}

#[tokio::test]
async fn test_execute_move_successfully() {
    let (tmp_dir, _file_path, fsm, config) =
        setup_fsm("source line 1\nsource line 2\nsource line 3");
    let source_path_str = tmp_dir
        .path()
        .join("test.txt")
        .to_str()
        .unwrap()
        .to_string();
    let dest_path = tmp_dir.path().join("dest.txt");
    fs::write(&dest_path, "dest line 1\ndest line 2").unwrap();
    let dest_path_str = dest_path.to_str().unwrap().to_string();

    let tool = FileEditorTool;

    // Prime FSM
    fsm.lock().unwrap().open_file(&source_path_str).unwrap();
    fsm.lock().unwrap().open_file(&dest_path_str).unwrap();
    let source_state = fsm.lock().unwrap().open_files[&source_path_str].clone();
    let dest_state = fsm.lock().unwrap().open_files[&dest_path_str].clone();

    let source_start = get_anchor(&source_state, 1); // "source line 2"
    let source_end = get_anchor(&source_state, 1); // "source line 2"
    let dest_anchor = get_anchor(&dest_state, 0); // "dest line 1"

    let args = serde_json::json!({
        "moves": [{
            "source_file_path": source_path_str,
            "source_range_start_anchor": source_start,
            "source_range_end_anchor": source_end,
            "dest_file_path": dest_path_str,
            "dest_at_position": "after_anchor",
            "dest_context_anchor": dest_anchor
        }],
        "replaces": [],
        "inserts": []
    });

    tool.execute(&args, &config, fsm).await.unwrap();

    let source_content = fs::read_to_string(tmp_dir.path().join("test.txt")).unwrap();
    let dest_content = fs::read_to_string(dest_path).unwrap();

    assert_eq!(source_content, "source line 1\nsource line 3");
    assert_eq!(dest_content, "dest line 1\nsource line 2\ndest line 2");
}

#[tokio::test]
async fn test_succeed_replace_with_no_anchors() {
    let (_tmp_dir, file_path, fsm, config) = setup_fsm("line 1");
    let file_path_str = file_path.to_str().unwrap().to_string();
    let tool = FileEditorTool;

    let args = serde_json::json!({
        "replaces": [{
            "file_path": file_path_str,
            "anchor_range_begin": null,
            "anchor_range_end": null,
            "new_content": ["new content"]
        }],
        "inserts": [],
        "moves": []
    });

    let result = tool.execute(&args, &config, fsm).await;
    assert!(result.is_ok());

    let final_content = fs::read_to_string(&file_path).unwrap();
    assert_eq!(final_content, "new content");
}

#[tokio::test]
async fn test_execute_fails_with_invalid_suffix() {
    let (_tmp_dir, file_path, fsm, config) = setup_fsm("line 1");
    let file_path_str = file_path.to_str().unwrap().to_string();
    let tool = FileEditorTool;

    fsm.lock().unwrap().open_file(&file_path_str).unwrap();
    let state = fsm.lock().unwrap().open_files[&file_path_str].clone();
    let mut anchor = get_anchor(&state, 0);
    anchor.lid = "lid-0_bad".to_string(); // Invalid suffix

    let args = serde_json::json!({
        "replaces": [{
            "file_path": file_path_str,
            "anchor_range_begin": anchor,
            "anchor_range_end": null,
            "new_content": ["..."]
        }],
        "inserts": [],
        "moves": []
    });

    let result = tool.execute(&args, &config, fsm).await;
    assert!(result.is_err());
    let error_string = result.unwrap_err().to_string();
    assert!(error_string.contains("Invalid FractionalIndex format in LID: '0'"));
}
