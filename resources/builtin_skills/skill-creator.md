---
name: skill-creator
description: Create, improve, and evaluate skills. Use when users want to build a new skill, revise an existing one, run quality evals, compare against a baseline, or optimize a skill description for better triggering.
user-invocable: false
---

# Skill Creator

Create new skills and improve existing ones through a repeatable loop.

## Workflow at a glance

1. Define the skill intent and trigger boundaries.
2. Draft or update SKILL.md and bundled resources.
3. Write a small eval set and run with-skill + baseline.
4. Review qualitative outputs + quantitative assertions with the user.
5. Improve the skill and repeat until feedback is clean.
6. Optionally optimize the SKILL.md description for triggering, then package.

## Communication

- Adjust jargon to the user's level.
- Explain terms like "assertion", "benchmark", and "JSON" when needed.
- Prefer concrete examples over abstract language.

---

## Create or update a skill

### 1) Capture intent

Collect or confirm:

1. What outcome should this skill enable?
2. When should it trigger?
3. What output format is expected?
4. Should this skill have objective assertions, subjective review, or both?

### 2) Draft SKILL.md

Frontmatter:

- `name` (required)
- `description` (required; include both "what it does" and "when to use it")
- `compatibility` (optional)

Body:

- Use imperative instructions.
- Keep core workflow in SKILL.md.
- Put large details in `references/`; link to them clearly.
- Put deterministic helpers in `scripts/`.
- Put reusable output templates/files in `assets/`.

Suggested structure:

```text
skill-name/
|- SKILL.md
|- scripts/      # optional
|- references/   # optional
`- assets/       # optional
```

Guidelines:

- Keep SKILL.md lean (target: under ~500 lines).
- Avoid duplicated content between SKILL.md and references.
- If a skill supports multiple variants, keep selection logic in SKILL.md and split variant details into separate files.

### 3) Create initial test prompts

Write 2-3 realistic prompts and save to `evals/evals.json` (prompts first, assertions can be empty initially):

```json
{
  "skill_name": "example-skill",
  "evals": [
    {
      "id": 1,
      "prompt": "User task prompt",
      "expected_output": "What success looks like",
      "files": [],
      "assertions": []
    }
  ]
}
```

See `references/schemas.md` for full schemas.

---

## Run and evaluate test cases

Do this as one continuous flow.

Workspace layout:

```text
<skill-name>-workspace/
`- iteration-N/
   `- <eval-name>/
      |- eval_metadata.json
      |- with_skill/
      |  |- outputs/
      |  |- grading.json
      |  `- timing.json
      `- without_skill/ or old_skill/
         |- outputs/
         |- grading.json
         `- timing.json
```

### Step 1: Launch runs in parallel

For each eval, launch both runs in the same turn:

- `with_skill`: run using the current skill.
- Baseline:
  - New skill work: `without_skill`.
  - Existing skill improvement: `old_skill` using a snapshot (`cp -r <skill-path> <workspace>/skill-snapshot/`).

Create `eval_metadata.json` per eval:

```json
{
  "eval_id": 0,
  "eval_name": "descriptive-eval-name",
  "prompt": "The user task prompt",
  "assertions": []
}
```

### Step 2: Draft assertions while runs execute

- Write objective assertions when possible.
- Use qualitative review for subjective outputs.
- Update both `eval_metadata.json` and `evals/evals.json`.

### Step 3: Capture timing per run

When each run finishes, save timing from task notifications to `timing.json`:

```json
{
  "total_tokens": 84852,
  "duration_ms": 23332,
  "total_duration_seconds": 23.3
}
```

### Step 4: Grade, aggregate, review

1. Grade each run (inline or via `agents/grader.md`) and save `grading.json`.
2. Aggregate:
   ```bash
   python -m scripts.aggregate_benchmark <workspace>/iteration-N --skill-name <name>
   ```
3. Analyze with `agents/analyzer.md` for variance, non-discriminating assertions, and cost/perf tradeoffs.
4. Generate viewer:
   ```bash
   nohup python <skill-creator-path>/eval-viewer/generate_review.py \
     <workspace>/iteration-N \
     --skill-name "<name>" \
     --benchmark <workspace>/iteration-N/benchmark.json \
     > /dev/null 2>&1 &
   VIEWER_PID=$!
   ```
   For iteration 2+, also pass `--previous-workspace <workspace>/iteration-(N-1)`.

Headless mode: use `--static <output.html>` instead of serving a live page.

### Step 5: Collect feedback

Read `feedback.json` after user review and focus edits on non-empty feedback items.

Stop viewer when done:

```bash
kill $VIEWER_PID 2>/dev/null
```

---

## Improve the skill

When iterating:

1. Generalize from feedback; do not overfit to a few prompts.
2. Remove instructions that add cost without improving results.
3. Explain "why", not only "what".
4. Productize repeated work into scripts/assets/references.

Iteration loop:

1. Update skill files.
2. Rerun evals into `iteration-(N+1)` with baseline.
3. Regenerate review viewer (`--previous-workspace`).
4. Collect feedback and repeat.

Stop when:

- User says it is good.
- Feedback is effectively empty.
- Additional iterations are not improving outcomes.

---

## Optional: Blind comparison

For stricter A/B judgment, use:

- `agents/comparator.md`
- `agents/analyzer.md`

Use this only when the user wants rigorous version comparison.

---

## Optional: Description optimization

Use this after the skill behavior is stable.

Prerequisites for fully automated mode:

- `claude` CLI (`claude -p`) for trigger evals
- `anthropic` Python package + API key for iterative description improvement

If those are unavailable, skip this section or run eval-only mode.

### 1) Build trigger eval set

Create ~20 realistic queries:

- 8-10 `should_trigger: true`
- 8-10 `should_trigger: false` (prefer near-misses, not trivial negatives)

Format:

```json
[
  {"query": "user prompt", "should_trigger": true},
  {"query": "another prompt", "should_trigger": false}
]
```

### 2) Review eval set with user

Use `assets/eval_review.html`:

1. Replace placeholders:
   - `__EVAL_DATA_PLACEHOLDER__`
   - `__SKILL_NAME_PLACEHOLDER__`
   - `__SKILL_DESCRIPTION_PLACEHOLDER__`
2. Open temp html for user edits.
3. Import exported eval JSON from `~/Downloads`.

### 3) Run optimization loop

Full loop (eval + auto-improve):

```bash
python -m scripts.run_loop \
  --eval-set <path-to-trigger-eval.json> \
  --skill-path <path-to-skill> \
  --model <session-model-id> \
  --max-iterations 5 \
  --improvement-mode auto \
  --verbose
```

Eval-only (no Anthropic dependency for improvement):

```bash
python -m scripts.run_loop \
  --eval-set <path-to-trigger-eval.json> \
  --skill-path <path-to-skill> \
  --improvement-mode disabled \
  --max-iterations 1 \
  --verbose
```

This runs train/test evaluation and proposes improved descriptions. Use the returned `best_description`.

### 4) Apply result

Update SKILL.md frontmatter `description`, then show before/after and scores.

---

## Packaging

If `present_files` is available, package and share:

```bash
python -m scripts.package_skill <path/to/skill-folder>
```

---

## Environment notes

### Claude Code

Use the full workflow above (parallel runs, baseline, benchmark, viewer, optimization).

### Claude.ai

- No subagents: run test cases serially.
- Skip baseline and quantitative benchmark unless explicitly needed.
- If browser viewing is unavailable, present outputs directly in chat.
- `run_loop.py` optimization may be unavailable without `claude -p` and Anthropic credentials.
- If unavailable, use manual review or `--improvement-mode disabled`.

### Cowork

- Subagents are available.
- Use `generate_review.py --static` for review output.
- User feedback may come back as downloaded `feedback.json`.

---

## Reference files

- `agents/grader.md`: assertion grading format and process
- `agents/analyzer.md`: benchmark analysis patterns
- `agents/comparator.md`: blind A/B comparison flow
- `references/schemas.md`: JSON schemas for eval and grading artifacts

Keep this loop in mind: draft/update -> evaluate -> review with user -> improve -> repeat -> package.
