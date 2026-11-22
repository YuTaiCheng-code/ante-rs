# Ante 0.3.4 逆向分析归档

> 仅供学习与互操作研究。原版二进制及其提取文本的版权归原作者所有；不得用于绕过授权、商业再分发或其他侵权用途。使用前请自行评估当地法律与软件许可风险。

本目录保存本轮差距分析、提示词证据、实现说明和验证结果。

## 文件

- `GAP_REPORT.md` — 原版与 ante-rs 的能力/结构差距、已补齐项、剩余优先级。
- `PROMPT_INVENTORY.md` — 15 份提示词提取物的来源、用途和对应实现。
- `IMPLEMENTATION_NOTES.md` — 本轮代码改动、验证命令与明确限制。
- `prompts/` — 从原版二进制提取的提示词原文，作为分析证据保留。

## 本轮落地

- 工具 schema：补齐 `WebSearch`、`ViewImage`、`TodoWrite`，支持小写/下划线别名；默认工具表只暴露有 schema 的工具。
- Skills：完整触发与读取规范、6 个内置技能正文、磁盘技能覆盖同名内置技能。
- Auto Memory：`MEMORY.md` 索引、25 KB 读取上限、系统提示段、`/memory`。
- MCP：stdio 与 Streamable HTTP/SSE，工具以 `mcp__<server>__<tool>` 注入。
- 会话结构：快照、`/rewind`、`/fork`、`/resume`，压缩走 handoff。
- TUI：普通输入执行真实 Agent 回合；compact/goal/diff/mcp/jobs 等命令闭环，usage/context 接真实状态。
- 后台 Job：独立进程组、输出/状态文件、完成通知。
- OAuth：自动续期与 `ante auth refresh`。
- 子代理：explore 使用完整提取提示并附加环境信息。

## 验证

```text
cargo test --release          22 passed
tests/prompts_e2e_test.py     11 passed
tests/approval_e2e_test.py     4 passed
tests/goal_e2e_test.py         5 passed
tests/features_e2e_test.py     5 passed
tests/offline_e2e_test.py      6 passed
tests/frame_test.py            9 passed
```

双架构发布二进制尚未重新打包；MCP 目前仅支持 stdio。
