const { GPTHistory } = await import("./GPTHistory.ts");

const gptHistory = new GPTHistory();
// response example: [{action: "message", content: "Nice to meet you, Felix!"}, {action: "save", content: "{name: 'Felix'}"}]
interface Response {
  action: string;
  content: string;
}

async function optimizeMemory(memory: string): Promise<string> {
  const prompt = `
  Extract and distill the the current state of conversation from the following content. Only answer with json and meaningful fields. Simplify the structure of the json. Don't give explanations.

  ${memory}
  `;
  const response = (await gptHistory.sendSnippet(prompt)).trim();
  return response;
}

async function readMemory(): Promise<string> {
  try {
    return await Deno.readTextFile("memory.json");
  } catch {
    return "{history:[]}";
  }
}

async function assist(message: string, memory: string): Promise<string> {
  const prompt = `
You are my personal assistant. You have a json-based memory to enrich your replies.

This is your current memory:
${memory}

Here is my message to you:
${message}
`;
  const response = await gptHistory.sendSnippet(prompt);
  return response;
}

const argMessage = Deno.args.join(" ");
const memory = await readMemory();
const response = await assist(argMessage, memory);
console.log(response);

const newMemory = {
  memory: memory,
  lastUserMessage: argMessage,
  lastAssistantResponse: response,
};

const optimizedMemory = await optimizeMemory(JSON.stringify(newMemory));
// gray
console.log("\x1b[34m", optimizedMemory);
await Deno.writeTextFile("memory.json", optimizedMemory);

// console.log(actions);

// for (const action of actions) {
//   if (action.action === "message") {
//     console.log(action.content);
//   } else if (action.action === "save") {
//     let jsonMemory = "";
//     if (typeof action.content === "string") {
//       jsonMemory = action.content;
//     } else {
//       jsonMemory = JSON.stringify(action.content);
//     }
//     const optimizedMemory = await optimizeMemory(jsonMemory);
//     // print in gray
//     console.log(
//       "\x1b[90m%s\x1b[0m",
//       JSON.stringify(JSON.parse(optimizedMemory), null, 2),
//     );
//     await Deno.writeTextFile("memory.json", optimizedMemory);
//   }
// }
