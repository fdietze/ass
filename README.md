# alors

## Goals

- be very close to how messaget+ased AI apis work. Try to minimize leaky abstractions. Everything can be fully understood and adjusted, like system prompts.
- No TUI, use the scrollback of the terminal to see what happened.
- the agent can be used via CLI, but also as a library to build higher-level workflows on top.
- Very robust file editing tools that allow LLMs to edit large files with precise changes over many steps without re-reading the file.
- High quality human readable codebase

## Configuration

```
~/.config/alors/config.toml
```

## Providers

OpenRouter:

```bash
export OPENROUTER_API_KEY='...'
cargo run -- "create a new file hello.txt containing 'Hello world'" --model "google/gemini-2.5-pro"
```

Ollama:

```bash
cargo run -- "create a new file hello.txt containing 'Hello world'" --backend ollama --model "qwen3:0.6b"
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.26s
     Running `target/debug/alors 'create a new file hello.txt containing '\''Hello world'\''' --backend ollama --model 'qwen3:0.6b'`
Backend: Ollama
Model: qwen3:0.6b
[user]
create a new file hello.txt containing 'Hello world'

[assistant]
<think>
Okay, the user wants to create a new file called hello.txt that contains 'Hello world'. Let me think about how to approach this.

First, I need to use the create_files function. The parameters required are files, which is an array of files to create. Each file in the array needs a file_path and content. The content should be an array of strings, with each string being the line content. Since the file is just one line, maybe I can set the content as ["Hello world"] and set the file_path to "hello.txt". That should create the file as specified.

I should make sure that the parameters are correctly formatted. The content array has one element, so the JSON should be like: {"content": ["Hello world"], "file_path": "hello.txt"}. Then, I'll output this in the tool call format.
</think>


[tool: create_files]
hello.txt:
\`\`\`

Hello world

\`\`\`
Execute this tool? [Y/n]

[assistant]
<think>
Okay, the user wanted to create a file called hello.txt with 'Hello world'. I called the create_files function with the correct parameters. The tool response says the file is created successfully. Now, I need to confirm if there's any further action needed. Since the user hasn't provided any new instructions, I should just acknowledge the creation. No more tool calls needed here.
</think>

The file 'hello.txt' has been created successfully. The content is as specified:

\`\`\`

File: hello.txt | Hash: 5b774464 | Lines: 1-1/1
1 lid-80_h812: Hello world

\`\`\`

No further actions are needed.
user>
```
