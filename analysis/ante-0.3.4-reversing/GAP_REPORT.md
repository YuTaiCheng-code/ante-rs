# Ante 0.3.4 原版 vs 重实现：功能缺口盘点

> 对象：`work/ante/ante`（官方 0.3.4，37 MB） vs `ante-rs`（当前重实现，约 5 MB / 39 个 Rust 文件 / 8,997 行）。
> 方法：CLI/子命令帮助、二进制中的模块路径与 panic 位置、工具 schema、内嵌提示词、现有源码模块对照。
> 说明：模块数来自二进制里可见的 `src/**/*.rs` 路径，只能作为“结构复杂度”的下界，不是官方完整文件清单。


## 本轮已补齐

- `WebSearch` / `ViewImage` / `TodoWrite` schema，以及 `--include-tools` 小写别名。
- 原版 Skills 渐进式加载提示；6 个内置技能正文常驻可发现。
- Auto Memory 提示、`MEMORY.md` 索引、`/memory` 命令。
- MCP：stdio 与 Streamable HTTP/SSE、会话头、动态工具注入。
- 会话快照、`/rewind`、`/fork`、`/resume`；自动压缩改为 LLM handoff。
- TUI 输入不再只是占位：会跑真实 Agent 回合，并保留首屏/命令面板帧。
- TUI 命令闭环：compact/goal/diff/mcp/init/rename/import-claude/jobs；usage/context 接真实会话与 token 估算。
- 后台 Job：真实后台句柄、独立进程组、输出/状态文件、完成通知。
- OAuth：自动续期、`ante auth refresh`、usage 查询。
- Claude Code memory 导入。

## 1. 量化差距

| 维度 | 官方原版 | 重实现 | 差距 |
|---|---:|---:|---|
| 二进制大小 | 37 MB | 5.0 MB | 约 7.4× |
| `.text` | 27.9 MB | 3.0 MB | 约 9.3× |
| `.rodata` | 4.6 MB | 0.94 MB | 约 4.9× |
| 可见 Ante 专属模块 | 174 个 | 39 个 Rust 文件 | 约 4.5× |
| TUI 模块 | 68 个 | 1 个 `tui.rs`，已接真实回合 | 组件化仍缺，行为不再是占位 |
| tools 模块 | 24 个 | 7 个（合并实现） | MCP 有 stdio/HTTP，prompts/resources 仍缺 |
| core 模块 | 21 个 | `session.rs` + `goal.rs` 等 | 压缩/快照/回退/分支已补，token 账本/ambient/event log 仍缺 |
| credentials 模块 | 9 个 | 1 个 `credentials.rs` | OAuth PKCE/usage/refresh 已有，专属流程仍缺 |
| offline 模块 | 9 个 | 3 个 | 引擎/router 已有，模型注册/服务/运行文件仍简化 |
| 默认 TUI 斜杠命令 | 30+ | 27 个注册 | 仅 statusline/update/offline-mode/add-provider 仍是提示；`/skills`、`/agents` 等仍缺 |

体积差不是单纯“我们写得紧”：官方代码段是我们的 9 倍多，说明大量分支、协议适配和 TUI 状态机还没有对应实现。

## 2. 当前主要缺口

已补齐：MCP stdio + HTTP/SSE、Skills、Auto Memory、压缩/快照、后台 Job、OAuth 续期、TUI 命令闭环。以下按剩余影响排序。

### 2.1 TUI 仍是“行为已通、组件未拆”
已有：真实 Agent 回合、命令闭环、真实 usage/context、首屏、命令面板、主题/模型/Provider overlay。
仍缺：
- 同屏流式渲染；当前每回合临时退出 alternate screen
- diff 审批、工具调用展开/折叠
- resume / rewind 选择器
- Markdown、代码块、表格、LaTeX
- 可配置状态栏、完整主题系统
- 后台 job 输出面板
- 剪贴板、图片粘贴、输入历史与补全
- 终端分屏 / tmux 集成
- 通知、终端标题、声音提醒
- `/statusline`、`/update`、`/offline-mode`、`/add-provider` 仍是提示

影响：交互密度和原版差距最大；68 个组件尚未拆分。

### 2.2 MCP 还缺 prompts/resources 与完整 OAuth
已有：stdio、Streamable HTTP/SSE、`Mcp-Session-Id`、工具发现/调用、按 server 汇总失败。
仍缺：
- MCP prompts / resources 的发现与读取
- MCP 专属 OAuth 授权流程（现在要求配置 Authorization header）
- tools/list 分页 cursor
- 连接健康检查、崩溃重连、超时隔离
- 运行时增删/重启 server、按 server 过滤工具
- `/mcp` 管理动作

### 2.3 上下文状态机仍缺精确账本与 ambient
已有：handoff 压缩、快照、rewind、fork、resume、宽字符 token 估算、后台通知注入。
仍缺：
- 与 provider tokenizer 对齐的精确 token 账本
- 工具结果分层降级、图片/附件预算
- 溢出后的二次压缩与失败重试
- ambient 状态、下一步建议、回合前 preamble
- 事件日志、完整 replay、会话 GC

### 2.4 后台 Job 已有执行模型，缺管理体验
已有：后台句柄、独立进程组、stdout/stderr/status 文件、完成通知、`/jobs` 列表。
仍缺：
- `/jobs kill <id>`、增量跟随输出
- 输出文件大小上限与 GC
- 子代理/多会话的 job 归属展示

### 2.5 账号与协议层
已有：通用 OAuth PKCE、自动 refresh、Anthropic/OpenAI usage。
仍缺：
- provider 专属登录差异、账号切换 UI
- Responses WebSocket 流式；`serve --websocket` 直接报未实现
- Provider catalog 的完整 resolve、effort ladder、自定义模型能力
- update 下载校验、诊断包脱敏等发布链路

### 2.6 多模态与子代理
- PDF 仅 `pdftotext` 抽文本；没有按页渲染、OCR、版面/图表理解。
- `ViewImage` 已注入图片，但缺少尺寸压缩、detail 预算和失败降级。
- explore 子代理已用完整提示；general 及其他角色模板、按角色工具权限仍未细化。

## 3. P1：功能面缺口

| 功能 | 官方行为 | 我们现状 | 备注 |
|---|---|---|---|
| WebSearch | 默认工具，带年份提示、重试、来源引用 | 已补 schema 并默认启用 | 重试/来源引用细节仍可对齐 |
| TodoWrite / ViewImage | 默认关闭，可用 `--include-tools` 打开 | 已补 schema 与小写别名 | 已可用 |
| PDF / 图片读取 | PDF 页范围/视觉读取；ViewImage detail | PDF 仅文本抽取；图片缺尺寸预算 | 多模态理解弱于原版 |
| 后台 Job | `job/` 模块、完成通知、stdout/stderr 文件 | 已有句柄/进程组/通知/列表 | kill 与 GC 仍缺 |
| OAuth / 订阅账号 | 登录、callback、profile、usage 限额 | PKCE + usage + 自动 refresh | 专属登录/账号 UI 仍缺 |
| WebSocket 流式 | OpenAI Responses WebSocket、增量事件 | 仅 HTTP SSE/JSON | 需要协议层扩展 |
| Provider 目录 | `catalog/{resolve,provider}.rs`、effort ladder | 单文件 `catalog.rs` | 自定义 provider 能力较弱 |
| 会话存储/事件日志 | `storage/{event_log,snapshot,gc}.rs` | `settings.rs` 为主 | 无法完整 replay/诊断 |
| 更新/诊断 | 下载校验、更新脚本、rage 脱敏 | `update.rs` 很薄 | 发布链路不完整 |
| 订阅用量 | 周限额、按 scope 统计 | `/usage` 只统计本地 token | 账号页不可用 |

## 4. P2：体验细节

- 主题选择与 30+ 命令面板
- 剪贴板、图片粘贴缓存、文件链接
- 通知（终端/iTerm2/tmux）
- 状态栏自定义
- 声音/注意力提示
- 输入历史与补全
- 多语言/数学公式渲染
- 会话标题自动命名
- 用量图表与 context grid

## 5. 已经对齐得不错的部分

- CLI 顶层与大部分子命令形状
- 主系统提示的 `intro/harness/communicating/working defaults`
- goal evaluator、handoff 压缩提示（与提取文本一致）
- 离线引擎安装器、产物矩阵、router、停止/清理流程
- 基础工具：Bash/Read/Write/Edit/Glob/Grep/WebFetch/Agent/AskUser
- 4 种 wire：OpenAI Chat/Responses、Anthropic、Gemini
- 权限模式、审批、基础 TUI 布局

## 6. 剩余优先级

已完成：工具表、Skills、Auto Memory、MCP stdio、Compact/Rewind/Fork、TUI 真实回合、explore 子代理完整提示。

仍需补：
1. **TUI 组件化**：diff approval、resume picker、agent panel、job 输出、markdown/LaTeX、状态栏。
2. **MCP 完整面**：prompts/resources、MCP OAuth、分页、重连、`/mcp` 管理。
3. **上下文账本与 ambient**：精确 tokenizer、工具结果降级、溢出重试、事件日志。
4. **Job 管理**：kill、跟随输出、GC。
5. **WebSocket 与多模态**：Responses WebSocket、PDF 页面渲染/OCR、图片预算。
6. **发布链路**：双架构打包、SHA256、更新校验、诊断脱敏。

> 结论：当前版本是“核心路径可用、P0/P1 大部分已补齐的精简重实现”，但仍不是完整等价实现。剩余最大差距是 **TUI 组件化、MCP prompts/resources 与专属 OAuth、精确上下文账本/ambient、WebSocket 与多模态**。
