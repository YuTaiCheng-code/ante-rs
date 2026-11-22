---
name: tmux
description: Drive an interactive terminal, long-running server, REPL, or another CLI agent in a named tmux session. Use when a task needs a persistent shell, an interactive program (vim, ssh, password prompts), watching a long-running process, or orchestrating another terminal-based agent such as Claude Code or Codex.
user-invocable: false
---

# Terminal sessions via tmux

Drive interactive programs in named tmux sessions using ordinary `tmux`
commands. Sessions are durable: they survive your turns, Ante restarts, and
the end of this conversation — anything long-running belongs in one, and a
later conversation can pick it up where it was left. The user can watch or
type in any session with `/term <name>`; repeated `/term` calls stack their
viewer splits in one column, so several sessions show at once.

## Sessions

Ante's sessions live in the `ante-` namespace. Only ever create or drive
sessions named `ante-*`; anything else in `tmux ls` belongs to the user —
never send keys to it, never kill it.

Reuse before creating — list Ante's sessions (no output means none yet):

```bash
tmux ls 2>/dev/null | grep '^ante-' || true
```

Create a session detached, wide enough that output isn't wrapped at 80
columns before anyone attaches:

```bash
tmux new-session -d -s ante-<purpose> -x 200 -y 50
```

Name sessions after their purpose (`ante-dev-server`, `ante-repl`,
`ante-codex`). Never kill an `ante-*` session you did not create in this
conversation; the user owns session lifetimes.

**Always write `-t` targets as `=<name>:`** (e.g. `-t =ante-dev-server:`).
Bare names are prefix-matched: with only `ante-dev-server-2` running,
`-t ante-dev-server` silently drives the wrong session. The `=` prefix
forces an exact match, and the trailing `:` marks the end of the session
name — pane-target commands (`send-keys`, `capture-pane`) reject a bare
`=<name>` with "can't find pane".

## Sending input

Send literal text and the submitting Enter as two separate calls:

```bash
tmux send-keys -t =ante-repl: -l 'cargo run --release'
tmux send-keys -t =ante-repl: Enter
```

`-l` sends text literally (no key-name interpretation). `Enter` sends CR —
the same byte a human's Enter key sends. Never embed `\n` in the literal
text to submit: multi-line input widgets (Claude Code, Codex) treat a
linefeed as "insert newline", not "submit".

Special keys go by tmux key name, without `-l`: `Enter`, `Escape`, `Tab`,
`BSpace`, `Up`, `Down`, `Left`, `Right`, `C-c` (Ctrl+C), `C-d`, `C-z`.

## Reading output

```bash
tmux capture-pane -p -t =ante-repl:             # visible screen (TUI apps)
tmux capture-pane -p -t =ante-repl: -S -2000    # + scrollback (shell commands)
```

For long output prefer `head`/`tail` inside the session over capturing a
huge scrollback.

## Waiting for completion

Two patterns, chosen per command:

- **Marker echo** — for shell commands. Append a marker and poll for it:

  ```bash
  tmux send-keys -t =ante-build: -l 'make test; echo DONE_47'
  tmux send-keys -t =ante-build: Enter
  # poll: capture the pane every few seconds until DONE_47 appears
  ```

- **Sleep then capture** — for TUI apps, REPLs, password prompts, and
  anything an injected `echo` would corrupt. Send the keys, `sleep` long
  enough for the program to render (typically 3–10s), then capture the
  visible screen.

## Driving other CLI agents

Another agent (Claude Code, Codex, ...) is just an interactive program in
its own session. Launch it with its initial prompt as an argument — typing
into a booting TUI races its input widget:

```bash
tmux new-session -d -s ante-claude -x 200 -y 50
tmux send-keys -t =ante-claude: -l 'claude "review the failing CI run and fix it"'
tmux send-keys -t =ante-claude: Enter
```

- One agent per session, named after the agent (`ante-claude`,
  `ante-codex`); never share a session between two programs.
- The user's own `/term claude` creates `ante-claude` *and* launches
  `claude` in it — reuse that session instead of creating another.
- Submit follow-up prompts as literal text then a separate Enter, exactly
  as above — agent input widgets treat an embedded linefeed as a newline,
  not submit.
- Agents are TUIs: use sleep-then-capture, and poll until the screen
  settles back at the agent's input prompt (agent turns can take minutes —
  poll patiently rather than assuming a fixed delay).
- To supervise several agents or compare them on the same task, send each
  session the same prompt, poll until both are idle, capture each pane,
  and report or diff the results. Offer `/term claude` + `/term codex` so
  the user can watch both — the viewer splits stack in one column.

### Launch recipes

- **Claude Code** (`claude`): `claude "<prompt>"` opens interactive with
  the prompt submitted; `--continue` resumes the last conversation in this
  directory. When nobody needs to watch, prefer headless
  `claude -p "<prompt>"` as a plain background Bash command — no tmux
  needed. Never pass `--dangerously-skip-permissions` unless the user
  asked for it.
- **Codex** (`codex`): `codex "<prompt>"` opens interactive; headless is
  `codex exec "<prompt>"`; `codex resume --last` continues the previous
  session.
- **Other agents**: check `--help` for an initial-prompt argument, a
  resume flag, and a headless mode before falling back to typing into the
  TUI.

## Handing control to the user

When the session needs input only the user can provide (browser auth codes,
2FA, passphrases) or they'd want to watch:

1. Capture the current screen and explain what the terminal is waiting for.
2. Ask the user to run `/term <name>` — it opens a real terminal window on
   the session where they can see everything and type. `/term` applies the
   `ante-` prefix itself, so `/term dev-server` and `/term ante-dev-server`
   are the same session.
3. After they confirm, capture the pane again and continue.

Never send secrets through `send-keys` — they land in shell history and
pane scrollback, and your own command line is logged.
