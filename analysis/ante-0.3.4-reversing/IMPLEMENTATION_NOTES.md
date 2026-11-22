# 本轮实现说明

## 1. 工具表 bug

问题：`WebSearch`、`ViewImage`、`TodoWrite` 的执行器早已存在，但 `resources/tools_*.json` 没有 schema，模型看不到；`--include-tools` 用原版小写名称时也匹配不到。

修复：
- 两种 prompt 都补齐三个 schema。
- 默认工具集加入 `WebSearch`。
- `TodoWrite` / `ViewImage` 保持默认关闭，但可通过 `--include-tools todowrite,viewimage` 打开。
- `canonical_tool_name()` 统一处理 `todowrite`、`view_image`、`web-search` 等写法。
- 移除没有 schema 的 `Terminal` 默认位置，避免默认工具表里有工具却不可见。

测试：
- `lowercase_tool_aliases_are_canonicalized`
- `all_default_tools_have_schemas_in_both_prompts`

## 2. Skills

修复：
- `resources/skills_prompt.md`：原版技能触发、完整读取、相对路径、references 路由、脚本复用规则。
- `resources/builtin_skills/*.md`：6 个内置技能正文（add-provider、ante-guide、resume-claude、resume-codex、skill-creator、tmux）。
- `skills::discover()` 现在始终附加内置技能；同名用户技能优先，内置版被 shadow。
- `Skill` 增加 `body_text`，内置技能不依赖磁盘路径也能读取正文。

测试：`builtin_skills_are_always_available_with_bodies`。

## 3. Auto Memory

修复：
- `resources/auto_memory_prompt.md`：原版记忆分类、保存/不保存规则、索引与回忆规则。
- `src/memory.rs`：`ANTE_HOME/memory`、`MEMORY.md` 索引（25 KB 上限）、系统段、`/memory [show|path|clear]`。
- `settings.auto_memory` / `--enable-auto-memory` / `--disable-auto-memory` 已有开关现在真正影响 prompt。
- `/import-claude` 读取 `./CLAUDE.md`、`./.claude/CLAUDE.md`、`~/.claude/CLAUDE.md`，写入 `memory/claude-code.md` 并更新索引。

测试：`prompt_has_directory_and_index`。

## 4. MCP

实现：
- `src/mcp.rs`：stdio JSON-RPC 2.0 与 Streamable HTTP/SSE。
- 支持 `initialize`、`notifications/initialized`、`tools/list`、`tools/call`。
- 工具名注入为 `mcp__<server>__<tool>`。
- 配置支持对象 `{command,args,env,cwd,transport}` 和字符串命令。
- MCP 工具动态出现在 `tool_schemas()`，调用走 `tools::execute()`。

HTTP 细节：
- POST JSON-RPC，Accept 同时声明 JSON 与 SSE。
- 解析 SSE `data:` 响应，保存 `Mcp-Session-Id` 并回传。
- 支持 config 的 `headers`，其中 `${ENV}` 会展开。
- 单 server 失败不再中断其他 server；401 给出配置 Authorization 的明确错误。

明确限制：
- MCP 专属 OAuth 授权、prompts/resources、tools/list 分页、重连与管理动作仍未实现。

测试：
- `stdio_discovery_and_call`（真实拉起 mock Python server）
- `http_sse_discovery_and_call`（本地 TCP mock 返回 SSE）
- `rejects_unknown_transport`
- `expands_env_headers`

## 5. 会话结构

实现：
- `Session::save_snapshot()` / `snapshot_numbers()` / `load_snapshot()` / `fork_from()`。
- `/rewind [n]`：回到某个用户输入边界。
- `/fork [n]`：从快照创建新会话并切换过去。
- `/resume [id]`：REPL/TUI 内恢复历史会话。
- 自动压缩 `maybe_compact` 改为调用 `goal::handoff()`，不再直接丢弃中间历史。
- `/compact` 也走 handoff 摘要 + 最近消息。

测试：`snapshot_roundtrip_and_fork`、`estimate_is_positive_and_counts_wide_text`。

## 6. TUI

保留现有首屏、命令面板、overlay 和逐帧测试，同时把非命令输入从占位文本改为真实 Agent 回合：
- 输入后暂时离开 alternate screen / raw mode，执行真实 `agent::run_turn()`，显示流式文本和工具开始事件。
- 回合结束保存会话、保存快照，再回到 TUI。
- `/rewind`、`/fork`、`/resume`、`/memory` 已接到真实状态。

TUI 命令闭环：
- `/compact` 走 handoff；`/goal` 接入目标循环。
- `/diff`、`/mcp`、`/init`、`/rename`、`/import-claude`、`/jobs` 均有真实实现。
- `/usage` 显示会话累计用量；`/context` 使用 `llm::estimate_tokens` 估算宽字符与工具 JSON。

明确限制：
- 还没有把 68 个原版 TUI 组件全部拆开；diff approval、resume picker、agent panel、job 输出面板、markdown/LaTeX 渲染仍未补齐。
- 当前 TUI 回合会临时退出全屏，不是理想的同屏流式渲染。
- `/statusline`、`/update`、`/offline-mode`、`/add-provider` 仍是提示。

## 7. 后台 Job

实现：
- `src/jobs.rs`：`ANTE_HOME/jobs/<id>/` 保存 `meta.json`、`status.json`、`stdout`、`stderr`。
- Bash 超过 `max_wait_ms` 返回后台句柄，不再杀进程。
- 子进程通过 `setpgid(0,0)` 进入独立进程组，`kill -- -<pid>` 可整组停止。
- waiter 线程写最终状态，并在下一回合开始注入 `<notification from="job">`。
- REPL/TUI 增加 `/jobs` 列表。

## 8. OAuth 续期

实现：
- `credentials::refresh_stored()` 用 catalog 的 `token_url`、`client_id`、`client_secret` 刷新 refresh token。
- 过期前 5 分钟自动刷新；失败只告警，不阻断请求。
- `ante auth refresh <name>` 强制刷新。

## 9. explore 子代理

实现：
- `resources/subagent_explore.md` 直接嵌入构建。
- explore 子代理的系统提示使用完整提取文本，并在末尾追加运行时环境段。
- 工具集仍限制为只读探索需要的 `Glob`、`Grep`、`Read`、`Bash`。

## 10. 验证

```text
cargo test --release          22 passed
tests/frame_test.py            9 passed
tests/approval_e2e_test.py     4 passed
tests/goal_e2e_test.py         5 passed
tests/features_e2e_test.py     5 passed
tests/offline_e2e_test.py      6 passed
tests/prompts_e2e_test.py     11 passed
```

最终 `cargo build --release` 已通过；双架构发布二进制尚未重新打包。
