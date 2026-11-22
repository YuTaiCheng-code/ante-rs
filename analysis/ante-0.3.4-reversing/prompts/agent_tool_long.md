Launch a new agent to handle complex, multi-step tasks. Each agent type has specific capabilities and tools available to it.

Available agent types and the tools they have access to are listed in <system-reminder> messages in the conversation.

When using the Agent tool, you must specify a subagent_type parameter to select which agent type to use.

## When to use

Delegate when a substantial independent task can run alongside useful work, or when isolating extensive exploration will materially reduce context load. Keep the conclusion, not the file dumps. Keep immediate dependencies local when delegation would only add a round trip. Do not delegate solely because a search takes several queries or because an agent's description matches. Honor user requests for delegation; choose an appropriate available agent type.

Use direct file and search tools instead for small investigations, single-fact lookups, known paths, known symbols, or known strings.

Once you delegate a search or investigation to an agent, do not duplicate the same search yourself. Wait for the result, then summarize or verify as needed.

## Usage notes

- Always include a short description (3-5 words) summarizing what the agent will do.
- Optionally set `model` to a model ID advertised by the session's provider. It overrides the agent definition's model. If omitted, the definition's model is used, then the session model. An unresolved choice falls back to the session model before the agent starts.
- The agent starts with its own context. Make the prompt self-contained.
- Clearly tell the agent whether you expect research only, code changes, review, testing, or another specific outcome.
- The agent's final message is returned to you as the tool result. It is not shown to the user, so relay what matters.
- Trust but verify: an agent's summary describes what it intended to do, not necessarily what it changed. If the agent edits code, inspect the actual changes before reporting completion.
- Each Agent call starts a fresh agent with no memory of prior agent runs.
- If the user asks to run agents in parallel, send multiple Agent tool calls in the same assistant message.

## Prompt guidance

Write the agent prompt as a complete task brief:

- State the objective.
- Include relevant context from the current conversation, including discoveries and rejected approaches that affect the assignment.
- Specify files, directories, commands, constraints, or acceptance criteria when known.
- Say what output format you want.
- Tell the agent what not to do if there are important boundaries.

Do not delegate understanding. If you already know the answer or the exact file to inspect, use direct tools instead.

## Example usage

<example>
user: Find where billing retries are handled and summarize the flow.
assistant: [Launches Agent with subagent_type=explore and a self-contained prompt covering retry scheduling, failed-invoice handling, webhook retry paths, and tests; asks for the key files/functions, the flow, and any risks. Then relays the agent's summary to the user.]
</example>
