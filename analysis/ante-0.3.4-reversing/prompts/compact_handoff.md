# Compaction handoff prompt (verbatim extraction)

```text
Write a handoff for a successor agent. The conversation above is being replaced by your handoff; the successor continues the work with it, any user messages preserved verbatim alongside it, and everything newer than what you can see, also verbatim.

Point to files and other recoverable sources rather than transcribing them. Spend the words on what cannot be recovered by looking: the user's intent and corrections, the decisions and the reasoning behind them, and the findings and uncertainties that took work to reach.

Distinguish confirmed findings from hypotheses. Record what was changed, what was executed, and what was verified. Preserve pending job identifiers, status/output paths, and unknown outcomes.

Structure it as:

**Goal:** what the user asked for, in their own terms: intent, constraints, every correction, and every direction they rejected.
**Done:** what is complete and how it was verified.
**Learned & decided:** findings and decisions with the reasoning that still matters, including what was rejected and why, so it is not proposed again.
**Index:** where things are: exact file paths, identifiers, commands, branches, URLs, and the short error messages that matter. Pointers, not contents.
**In progress:** what was underway at the cut, and its exact state.
**Next:** the immediate next steps, in order.

Some tool results above were replaced by a "[result cleared to save context …]" marker. A cleared result means its output was removed from context; it does not establish success or failure. Preserve available evidence from surrounding messages and identify what remains unknown. Direct the successor to recorded evidence or current state; do not replay a state-changing action merely to recover its output. Output only the handoff.
```
