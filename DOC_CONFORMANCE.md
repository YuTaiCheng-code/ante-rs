# 文档对齐清单（5 篇 usage 文档）

抓取方式：`curl -L --compressed -A "<Chrome UA>"`，五篇均返回 200，正文从 `<article>` 提取后通读。
二进制核验：在 `work/ante/ante` 中检索到对应实现痕迹，例如

- catalog 字段：`oauth_profiles`、`helpers`、`token_request_encoding`、`derived_headers`、`required_scopes`、`max_request_images`
- 审批：`Tool call denied by user:`、`Tool call denied by user and was not executed.`
- 目标循环：`src/core/session/goal.rs`、`🔁 Goal not yet met (iteration `、`🛑 Goal stopped after `、`🚫 Goal abandoned — judged unreachable: `、`✅ Goal met: `、`Goal cleared — could not evaluate the condition: `
- 版本提示：`release_notes`、`dismissed_version`、`last_checked_unix_secs`

环境里没有 Ghidra/radare2，已安装 capstone 5.0.7；本轮以字符串/结构定位为主，指令级反编译尚未展开。

## 1. providing-context（上下文提供）

| 要求 | 状态 |
| --- | --- |
| `@file` 引用整文件、`@dir/` 引用目录树，Tab 补全、Enter 确认 | 未实现 |
| 补全覆盖完整项目树、复用忽略规则、支持 `~/`、绝对路径、`../` | 未实现 |
| 粘贴多个路径自动加 `@` 前缀 | 未实现 |
| `Ctrl+V` / `Ctrl+Shift+V` 粘贴图片，PDF 附件 | 未实现 |
| 图片受 `support_vision` 约束；`max_request_images` 保留最新图片 | 字段已解析，请求裁剪未实现 |
| OpenAI Responses / Chat Completions 保留工具图片关联 | 未实现 |
| `WebFetch` / `WebSearch` 工具、管道输入 `curl … \| ante -p` | `tools/web.rs` 存在，需逐项核对工具名与开关 |

## 2. approvals（授权与审批）

| 要求 | 状态 |
| --- | --- |
| 危险工具调用弹出审批，数字选择 1–5，`y`/Enter 确认，Esc 拒绝 | 已实现：TTY 下打印 5 选项并读取数字；输入 `y`/空行=本次允许，`4`=拒绝，`5`=拒绝并附反馈。方向键与 Esc 仍待接 crossterm 事件 |
| Tab 展开 diff 预览 | 未实现 |
| 5 个选项：本次 / 本会话 / 永久写入 settings / 拒绝 / 拒绝并反馈 | 未实现（当前只有 yes/always/no） |
| 会话级授权智能泛化（`cargo test -p foo` → `cargo test *`） | 未实现 |
| 拒绝理由作为工具失败结果回传（2000 字符上限、Readline 快捷键） | 未实现 |
| `Shift+Tab` 循环 strict → auto → yolo，即时生效 | 未实现（TUI 未处理 BackTab，底栏文案固定） |
| `ante --yolo` | 已实现 |
| Auto 模式下危险 Bash 仍询问 | 依赖 PermissionManager，需补规则核对 |

## 3. goal-sessions（目标导向会话）

| 要求 | 状态 |
| --- | --- |
| `/goal <condition>` 设定/替换，`/goal` 查询，`/goal clear` 清除 | 已实现（repl；TUI 仍只做状态展示） |
| 每轮结束后评估，未达成自动带反馈继续 | 已实现：判定器提示词与 `{ok, impossible, reason}` 从原件 rodata 还原 |
| 停止条件：Met / Unreachable / Cleared / Interrupted / 最大迭代 | Met/Unreachable/Cleared + 上限 10 已实现；Interrupted 依赖信号处理，未实现 |
| `Op::Goal` 编程接口 | 未实现（协议层仍返回 unsupported） |
| 会话持久化 + `/resume` 接管 | `/resume` 未实现 |

## 4. custom-sign-ins（自定义登录与 token 命令）

| 要求 | 状态 |
| --- | --- |
| `catalog.json` 的 `oauth_profiles`（client_id / redirect_uri / authorize_url / token_url / scopes / account_claim 等） | 结构已解析 + 启动校验，`doctor` 逐字段报告 |
| `redirect_uri` 必须 localhost/127.0.0.1、URL 必须绝对、`token_request_encoding` 只允许 json/form | 已实现并测试 |
| `${NAME}` / `${NAME:-default}` 环境变量展开 | 已实现（helper 命令与校验路径） |
| `credential` 支持 `oauth:<profile>` | 读取已存 token，浏览器授权流程未实现 |
| `credential` 支持 `helper:<name>`：数组命令、不经 shell、15 秒超时、取首行、`ttl_secs` 缓存、失败不回落 | 已实现并测试（旧实现按 `sh -c` 字符串执行且无超时/缓存，已修） |
| `derived_headers`（JWT claim 派生 header） | 仅校验，未应用 |
| `ante auth login/logout/list` 的 OAuth 分支、`--no-browser`、`profile/label` 多账号 | 未实现 |
| `/providers` 中按 `m` 进入 Sign in | 未实现 |

## 5. models-and-thinking（模型与思考深度）

| 要求 | 状态 |
| --- | --- |
| `/models` 面板：当前 provider 的模型列表、`Switch Model — <provider>` 标题 | 已实现（浮层帧 1.000） |
| `/models <model_id>` 直接切换 | 已实现（仅当次会话内存生效） |
| `/providers` 切换 provider、`Enter` 选模型、`←/→` 选账号 | 面板已实现，账号切换未实现 |
| 思考深度滑块、只显示模型支持的档位、不支持时隐藏 | 滑块已绘制；档位过滤/隐藏未实现 |
| 每个模型记住自己的 effort；`--effort min/low/medium/high/xhigh/max` | `--effort` 已实现；按模型记忆未实现 |
| 切换模型时清理旧模型推理数据/工具签名 | 未实现 |
| `Ctrl+O` 查看推理内容 | 未实现 |

## 反编译定位方法

`tools/re_find.py`：解析 ELF、取 `.text`/`.rodata`，capstone 全量扫描（28MB `.text`，约 11 秒），把 `lea reg,[rip+disp]` 命中的字符串地址映射回代码地址。用它在 `0x1d37c0e` 找到完整判定器提示词，在 `0x827139` 找到 `iteration` 与上限的比较（`[r12+0x288]` vs `[r12+0x28c]`），并确认 `src/core/session/goal.rs` 的 8 条状态文案。

## 本轮实际改动

1. `src/credentials.rs`
   - helper 改为按文档执行：命令数组直启（不经过 shell）、`${ENV}` 展开、15 秒超时、只取 stdout 首行、`ttl_secs` 缓存到 `auth/helper_cache/<name>.json`、失败时输出 `helper \`name\` failed: …` 且不回落。
2. `src/catalog.rs`
   - 启动时校验 `oauth_profiles` / `helpers`，错误按字段并入 `doctor` 的 warnings。
3. `tests/docs_conformance_test.py`：4 项（坏 profile 3/3 警告、合法 profile 无警告、helper 首行+缓存、helper 字段校验）全部通过。

## 未完成（按文档优先级建议）

1. `/goal` 闭环（含 `Op::Goal`、评估器、最大迭代）。
2. 审批 UI 五选项（已实现数字选择/会话授权/永久写入 settings/拒绝/拒绝+反馈）；Tab diff 预览与 `Shift+Tab` 模式循环待接 crossterm 事件。
3. `@` 文件/目录补全与请求内联。
4. OAuth 浏览器登录（PKCE + localhost 回调）与 `auth login/logout/list`。
5. 图片/PDF 附件与 `max_request_images` 裁剪。

6. `src/goal.rs` 新增目标闭环：
   - 判定器提示词与规则逐字取自原件 rodata；
   - `parse_verdict` 容错解析 `{"ok","impossible","reason"}`，无 JSON 时回退为继续；
   - `apply` 处理 Met / Unreachable / 继续 / 迭代上限（10）；
   - `tests/goal_e2e_test.py` 用可识别判定器请求的 SSE mock 跑通：第 1 次 not met → 自动续轮 → 第 2 次 met。
7. `ANTE_FORCE_REPL=1`：允许非 TTY stdin 进入 repl，便于脚本化测试（此前任何管道输入都会被当成 headless prompt）。
8. `tests/approval_e2e_test.py` 覆盖审批四场景：选项 2 会话级放行、选项 3 写入 `settings.permissions.allow`、再次运行免审批、选项 4 拒绝且工具未执行。
   修复要点：repl 不再长期持有 `StdinLock`，每轮重新 `read_line`；`Bash(cargo test *)` 这类前缀模式可匹配不带参数的命令。
