interface Message {
  role: string;
  content: string;
}

interface Choice {
  message: Message;
}

interface Resp {
  id: string;
  created: number;
  model: string;
  choices: Choice[];
}

export class GPTHistory {
  private history: string[] = [];

  public async sendSnippet(command: string): Promise<string> {
    this.history.push(command);

    const answer = await this.callGPTAPI(this.history.join("\n"));

    this.history.push(answer);

    return answer.trim();
  }

  private async callGPTAPI(prompt: string): Promise<string> {
    const apiKey = Deno.env.get("OPENAI_API_KEY");

    const url = "https://api.openai.com/v1/chat/completions";
    const headers = {
      "Content-Type": "application/json",
      "Authorization": `Bearer ${apiKey}`,
    };

    const data = {
      model: "gpt-3.5-turbo",
      messages: [{ role: "user", content: prompt }],
    };

    const httpResponse = await fetch(url, {
      method: "POST",
      headers,
      body: JSON.stringify(data),
    });
    const response: Resp = JSON.parse(await httpResponse.text());

    return response.choices[0].message.content;
  }
}
