use openrouter_api::types::chat::Message;

/// Prints a message to the console with nice formatting.
pub fn pretty_print_message(message: &Message) {
    println!("\n[{}]", message.role);

    if !message.content.is_empty() {
        println!("{}", message.content);
    }

    if let Some(tool_calls) = &message.tool_calls {
        println!("# tool calls:");
        for tool_call in tool_calls {
            println!(
                "# - Calling `{}`: {}",
                tool_call.function_call.name, tool_call.function_call.arguments
            );
        }
    }
}
