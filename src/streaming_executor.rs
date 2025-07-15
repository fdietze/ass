use anyhow::Result;
use futures::StreamExt;
use openrouter_api::{OpenRouterClient, Ready, models::tool::ToolCall, types::chat::*};
use std::io::{Write, stdout};

pub async fn stream_and_collect_response(
    client: &OpenRouterClient<Ready>,
    mut request: ChatCompletionRequest,
) -> Result<Message> {
    request.stream = Some(true);
    let mut stream = client.chat()?.chat_completion_stream(request);

    let mut content = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let choice = chunk.choices.first();

        if let Some(c) = choice.and_then(|c| c.delta.content.as_deref()) {
            print!("{c}");
            stdout().flush()?;
            content.push_str(c);
        }

        if let Some(tool_call_chunks) = choice.and_then(|c| c.delta.tool_calls.as_ref()) {
            // The stream can send tool calls in chunks, so we append them.
            tool_calls.extend(tool_call_chunks.iter().cloned());
        }
    }

    println!();

    Ok(Message {
        role: "assistant".to_string(),
        content,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        name: None,
        tool_call_id: None,
    })
}
