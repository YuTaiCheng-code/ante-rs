Launch a new agent to handle complex, multi-step tasks. Each agent type has specific capabilities and tools.

Available agent types and the tools they have access to are listed in <system-reminder> messages in the conversation.

Specify `subagent_type` to select the agent type, plus a short (3-5 word) `description` of the task.

Optional `model` selects a model ID advertised by the session's provider, overriding the agent definition. Otherwise the definition's model is used, then the session model. An unresolved choice falls back to the session model before the agent starts.

- Delegate substantial independent work that can run alongside useful work, or extensive exploration that materially reduces context load; keep the conclusion, not the file dumps. Keep immediate dependencies local when delegation only adds a round trip. Don't delegate solely for query count or an agent description match. Honor user requests for delegation; choose an appropriate agent type. For known targets and small investigations, use direct tools.
- The agent starts with fresh context and no memory of prior runs. Write the prompt as a self-contained task brief: objective, relevant context (including discoveries and rejected approaches), constraints, and expected output; say whether you expect research only, code changes, review, or testing.
- Once you delegate a search, don't duplicate it yourself — wait for the result.
- The agent's final message is returned to you and is not shown to the user, so relay what matters. If the agent edited code, inspect the actual changes before reporting completion.
- To run agents in parallel, send multiple Agent tool calls in one message.
