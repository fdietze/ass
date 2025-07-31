use anyhow::Result;
use console::style;
use futures::StreamExt;
use openrouter_api::{
    OpenRouterClient, Ready,
    models::tool::{FunctionCall, ToolCall},
};
use openrouter_api::{models::tool::ToolCallChunk, types::chat::*};
use std::collections::HashMap;
use std::io::{Write, stdout};

pub async fn collect_response_non_streaming(
    client: &OpenRouterClient<Ready>,
    mut request: ChatCompletionRequest,
) -> Result<Option<Message>> {
    request.stream = Some(false);

    let response = client.chat()?.chat_completion(request).await?;

    if let Some(choice) = response.choices.first() {
        Ok(Some(choice.message.clone()))
    } else {
        Ok(None)
    }
}

/// Streams a chat completion request to the console and collects the full response.
pub async fn stream_and_collect_response(
    client: &OpenRouterClient<Ready>,
    mut request: ChatCompletionRequest,
) -> Result<Option<Message>> {
    request.stream = Some(true);
    let mut stream = client.chat()?.chat_completion_stream(request);

    let mut content = String::new();
    let mut tool_call_chunks: HashMap<u32, ToolCallChunk> = HashMap::new();
    let mut header_printed = false;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let choice = chunk.choices.first();

        if let Some(c) = choice.and_then(|c| c.delta.content.as_deref()) {
            if !header_printed {
                print!("\n[{}]\n", style("assistant").blue());
                stdout().flush()?;
                header_printed = true;
            }
            print!("{c}");
            stdout().flush()?;
            content.push_str(c);
        }

        if let Some(delta_tool_calls) = choice.and_then(|c| c.delta.tool_calls.as_deref()) {
            for tool_call_chunk in delta_tool_calls {
                let entry = tool_call_chunks
                    .entry(tool_call_chunk.index)
                    .or_insert_with(|| ToolCallChunk {
                        index: tool_call_chunk.index,
                        id: None,
                        kind: None,
                        function: None,
                    });

                if let Some(id) = &tool_call_chunk.id {
                    entry.id = Some(id.clone());
                }
                if let Some(kind) = &tool_call_chunk.kind {
                    entry.kind = Some(kind.clone());
                }
                if let Some(function_chunk) = &tool_call_chunk.function {
                    let function_entry = entry.function.get_or_insert(
                        openrouter_api::models::tool::FunctionCallChunk {
                            name: None,
                            arguments: None,
                        },
                    );

                    if let Some(name) = &function_chunk.name {
                        function_entry.name = Some(name.clone());
                    }
                    if let Some(arguments) = &function_chunk.arguments {
                        let current_args = function_entry.arguments.get_or_insert_with(String::new);
                        current_args.push_str(arguments);
                    }
                }
            }
        }
    }

    if header_printed {
        println!();
    }

    let mut tool_calls = tool_call_chunks
        .into_values()
        .map(|chunk| {
            let function_chunk = chunk.function.unwrap_or_default();
            ToolCall {
                id: chunk.id.unwrap_or_default(),
                kind: chunk.kind.unwrap_or("function".to_string()),
                function_call: FunctionCall {
                    name: function_chunk.name.unwrap_or_default(),
                    arguments: function_chunk.arguments.unwrap_or_default(),
                },
            }
        })
        .collect::<Vec<ToolCall>>();

    tool_calls.sort_by_key(|tc| tc.id.clone());

    if content.is_empty() && tool_calls.is_empty() {
        return Ok(None);
    }

    Ok(Some(Message {
        role: "assistant".to_string(),
        content,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        name: None,
        tool_call_id: None,
    }))
}
