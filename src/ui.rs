use colored::Colorize;
use openrouter_api::types::chat::Message;

/// Prints a message to the console with nice formatting.
pub fn pretty_print_message(message: &Message) {
    let role_text = message.role.to_string();
    let role_colored = match role_text.as_str() {
        "tool" => role_text.purple(),
        _ => role_text.blue(),
    };
    println!("\n[{role_colored}]");

    if !message.content.is_empty() {
        let is_user = role_text == "user";
        for line in message.content.lines() {
            if line.starts_with('#') {
                println!("{}", line.dimmed());
            } else if is_user {
                println!("{}", line.cyan());
            } else {
                println!("{line}");
            }
        }
    }

    if let Some(tool_calls) = &message.tool_calls {
        println!("{}", "# tool calls:".dimmed());
        for tool_call in tool_calls {
            let text = format!(
                "# - Calling `{}`: {}",
                tool_call.function_call.name, tool_call.function_call.arguments
            );
            println!("{}", text.dimmed());
        }
    }
}
