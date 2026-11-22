---
name: ante-guide
description: Cheat sheet for the running Ante version. Use for questions about Ante itself (features, CLI, TUI, slash commands, settings, permissions, skills, MCP, catalog), for changing Ante's own configuration, for diagnosing Ante problems (startup, catalog, sign-in, update), and for "what changed" or "what broke after an update". Ordinary task requests that merely use the second person ("can you fix this?") are not about Ante.
user-invocable: false
---

# Ante cheat sheet

This file ships inside the binary, so it matches the installed version. Truth order:

1. The user's files say what is *set* — read them, never guess.
2. This sheet says what things *mean* in this version.
3. The binary says the rest: `ante --help`, `ante <command> --help`, `ante catalog`, `ante doctor`.
4. Docs at `https://docs.antigma.ai/` only for depth the above lack. They may describe a different version than `ante --version`; where they disagree with this sheet, this sheet wins.

Never answer from memory, never clone the repository, never infer shipped behavior from Ante's source tree.

## Operations

| Symptom | Do |
|---|---|
| Won't start, behaves oddly, "is my setup ok?" | `ante doctor` — checks home writability, the project settings layer, the merged catalog and its auth; each warning names the file at fault |
| Provider or model missing, catalog edit not showing | `ante catalog` prints the merged catalog; stderr `warning: skipped catalog entry …` names the bad entry. The catalog is read at startup — restart after editing |
| Sign in or manage credentials | `/providers` in the TUI: select a provider, then press `m` for OAuth or API-key access. CLI: `ante auth login <name>`. Configured credential order applies unless you choose a credential for the session |
| Update | `ante update` installs the channel's newest release (or an exact version); `/update` schedules the pending one for exit. `channel` setting, default `stable` |
| Bug report | `ante rage` — redacted bundle of logs and environment |
| Which version, what changed | `ante --version`; `versions.json` in Ante home holds the latest release notes offline; full history at `https://raw.githubusercontent.com/AntigmaLabs/ante/main/CHANGELOG.md` |

## Files

Ante home is `$ANTE_HOME`, else `~/.ante`. `ANTE_PROFILE` or `--profile <name>` swaps `settings.json` for `<name>.settings.json`.

- `settings.json` — preferences. Session keys: `model`, `provider`, `permission_mode`, `permissions` (`allow`/`ask`/`deny` matchers), `mcp_servers`, `model_effort`, `tools`, `skills`, `include_skills`, `exclude_skills`, `auto_memory`, `session_save`, `short_prompt`, `system_prompt`, `append_system_prompt`. UI keys, applied live: `theme`, `render_mode`, `status_line`, `status_line_command`, `channel`, `tips`, `auto_compact`, `group_tool_activity`; `attention` (TUI bell and desktop notification, read at startup)
- `<name>.settings.json` — a profile: whole-file replacement of `settings.json`, not a layer; omitted values fall back to defaults
- `catalog.json` — custom providers (models in each provider's `preferred_models`), `oauth_profiles`, `helpers`; use the `add-provider` skill. A provider's `credential` is `env:NAME`, `secret:NAME`, `oauth:PROFILE[/label]`, `helper:NAME`, a list tried in order, or `null` for none; omitted on a custom provider it means `secret:<id>`
- `AGENTS.md` — personal instructions; `~/.claude/CLAUDE.md` is read only when it is absent
- `auth/credentials/` stored credentials, managed with `ante auth` · `sessions/` transcripts and snapshots · `logs/` · `offline-config.json` · `versions.json`
- `skills/`, `agents/` — user skills and subagents; `.system/` — bundled ones, rewritten on every version change

Project, from the cwd: `AGENTS.md` (wins outright over `CLAUDE.md`, even when empty); `.ante/settings.json` from the nearest ancestor — session keys only, may narrow but never widen permissions, `model`/`provider` ignored; skills in `.ante/skills/`, `.agents/skills/`, `.claude/skills/`; subagents in `.ante/agents/`, `.agents/agents/`, `.claude/agents/`.

## Precedence

- Settings: defaults < `settings.json` or profile < project `.ante/settings.json` < CLI flags.
- Permissions: `deny` > session grants > `ask` > `allow` > tool defaults. Unmatched calls fall to the mode — `strict` asks except for known-safe read-only Bash, `auto` runs except dangerous Bash, `yolo` runs everything and ignores even `deny`. Shift+Tab cycles modes (`strict`/`auto` persist, `yolo` is session-only); "always allow" on a prompt appends the matcher to `permissions.allow`.
- A matcher is a tool name (`Bash`) or a primary-argument glob (`Bash(cargo test *)`, `Read(~/.ssh/*)`, `Agent(explore)`).
- Session keys are read at session start; say which edits need a new session.
- `short_prompt` / `--short-prompt` selects smaller built-in tool descriptions; fresh system instructions are already compact in either mode.
- Resume keeps the saved system instructions and prompt mode; current `system_prompt`, `append_system_prompt`, and `short_prompt` settings or flags apply to fresh sessions only, so start a new session to change instructions. The environment and memory blocks stay as they were at session start (a date change is told by a later message); tools and permissions follow current settings. A session saved before its instructions were recorded resumes on current settings.

## Changing configuration

Smallest surface: a one-off constraint goes in the prompt, a repository convention in the project `AGENTS.md`, a personal one in `~/.ante/AGENTS.md`, a preference in `settings.json`, a reusable workflow in a skill. Prefer `/config`, `/mcp`, `/theme`, `/statusline` — they write the same file. Do not invent field names: for a key not listed above, read its row on `configuration/preference` first. Community profiles and skills are indexed at `https://raw.githubusercontent.com/AntigmaLabs/ante/main/curated/README.md` — fetch files, never clone.

## Slash commands

`/clear` · `/compact [instructions]` · `/context` · `/usage` · `/goal <condition> | clear` · `/rename [title]` · `/resume` · `/init` · `/exit` · `/config` · `/theme` · `/statusline` · `/providers` · `/models [model_id]` · `/offline-mode` · `/mcp` · `/update [skip | cancel]` · `/diff` · `/term [name]` · `/welcome` · `/import-claude`. User-invocable skills appear as `/<name>`. There is no /help and no /permissions command.

## After an update

Stored formats that changed and may still need a hand. Prune entries older than the last few minor series at release time.

- **v0.3.0** — credential config reshaped; old shapes are refused by name (`ante catalog` stderr, `ante doctor`). Provider `auth` → `credential` (`{"bearer":{"env_key":"X"}}` → `"env:X"`; a `header` placement adds `"credential_header": "H"`), `http_headers` → `headers`, `{session_id}` → `${session_id}`, the top-level `models` section → the provider's `preferred_models`. A custom provider that had no `auth` now fails with "No credential for <id>": add `"credential": null`. MCP `oauth` block → `ante auth login <server>` or `headers.Authorization`; the MCP login command is now `ante auth login`. Provider `antix-api-key` → `antix`. Stored sign-ins and `api_keys.json` move to `auth/credentials/` on their own.
- **v0.2.11** — `offline-config.json` gains `default_model` and drops a few unused keys; older files still load. No action.
- **v0.2.8** — session `meta.json`/`dialog.json` fields changed; older sessions still load and resume. No action.
- **Legacy keys** still tolerated silently: `permission_settings` is ignored — move its rules under `permissions`; `channel: "latest"` means `stable`.

## Docs, as last resort

Pages under `https://docs.antigma.ai/`: `usage/slash-commands`, `usage/tui`, `reference/tui-reference`, `reference/cli-reference`, `configuration/preference`, `configuration/permission`, `reference/catalog-reference`, `reference/storage-reference`, `extend/skills`, `extend/mcp`, `extend/agents-md`. For anything else, WebFetch `https://docs.antigma.ai/llms-full.txt`, grep the cache path it returns (never Read it whole), and reuse that path for the rest of the session. If the docs do not cover it, say so.

Issues: https://github.com/AntigmaLabs/ante/issues
