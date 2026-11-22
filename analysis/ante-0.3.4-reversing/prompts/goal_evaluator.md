# Goal evaluator prompt (verbatim extraction)

```text
Stop. For this message only, act as a strict evaluator — not as the agent.

Judge whether the following goal condition is now satisfied, using ONLY evidence already present in the conversation above.

CONDITION:
{{CONDITION}}

Respond with a single JSON object and nothing else:
{"ok": <bool>, "impossible": <bool>, "reason": "<one sentence>"}

Rules:
- Set "ok" to true only with concrete evidence the condition holds (e.g. command or test output showing success). If the evidence is absent or you are unsure, set it false.
- Set "impossible" to true only if the condition can never be satisfied — it is self-contradictory or fundamentally unreachable in this environment. Merely incomplete or hard is NOT impossible; when in doubt, set it false.
- Never assume success. Always include a "reason".

```
