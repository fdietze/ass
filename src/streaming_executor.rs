use anyhow::Result;
use console::style;
use futures::StreamExt;
use openrouter_api::{OpenRouterClient, Ready, models::tool::ToolCall, types::chat::*};
use std::io::{Write, stdout};

pub async fn stream_and_collect_response(
    client: &OpenRouterClient<Ready>,
    mut request: ChatCompletionRequest,
) -> Result<Option<Message>> {
    request.stream = Some(true);
    let mut stream = client.chat()?.chat_completion_stream(request);

    let mut content = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
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

        if let Some(tool_call_chunks) = choice.and_then(|c| c.delta.tool_calls.as_ref()) {
            // The stream can send tool calls in chunks, so we append them.
            tool_calls.extend(tool_call_chunks.iter().cloned());
        }
    }

    if header_printed {
        println!();
    }

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
