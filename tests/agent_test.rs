use anyhow::Result;
use alors::{agent::Agent, config::Config, tool_collection::ToolCollection, tools::FileEditorTool};
use openrouter_api::models::tool::{FunctionCall, ToolCall};
use serde_json::json;
use std::{fs, sync::Arc};
use tempfile::tempdir;

#[tokio::test]
async fn test_agent_uses_file_editor_tool() -> Result<()> {
    // 1. Setup our test environment
    let temp_dir = tempdir()?;
    let file_path = temp_dir.path().join("test_file.txt");
    fs::write(&file_path, "Initial content.\n")?;

    // Ensure all file operations in the test are relative to the temp directory
    std::env::set_current_dir(temp_dir.path())?;

    // 2. Initialize the agent and its components
    let config = Config::default();
    let mut tool_collection = ToolCollection::new();
    tool_collection.register(Box::new(FileEditorTool));
    let tool_collection = Arc::new(tool_collection);

    let mut agent = Agent::new(config, None, tool_collection);

    // 3. Manually create the tool call we want to test
    let tool_call = ToolCall {
        id: "call_123".to_string(),
        kind: "function".to_string(),
        function_call: FunctionCall {
            name: "edit_files".to_string(),
            arguments: json!({
                "inserts": [{
                    "file_path": "test_file.txt",
                    "new_content": "This is new content.\n",
                    "at_position": "end_of_file",
                    "context_anchor": null,
                }],
                "replaces": [],
                "moves": [],
            })
            .to_string(),
        },
    };

    // 4. Execute the tool call
    let result = agent.execute_tool_calls(vec![tool_call]).await;
    assert!(result.is_ok());

    // 5. Verify the result
    let final_content = fs::read_to_string(&file_path)?;
    let expected_content = "Initial content.\nThis is new content.\n";
    assert_eq!(final_content.trim_end(), expected_content.trim_end());

    Ok(())
}
