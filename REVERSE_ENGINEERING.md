# ante 0.3.4 逆向分析与 Rust 等效实现报告

原始样本：`ante-v0.3.4-linux-x86_64-musl.tar.gz`，解包后为单个 38,347,632 字节的
`ante`（ELF 64-bit LSB pie，static-pie，stripped）。自述 “Ghost in the shell”，
是 Antigma Labs 的终端 AI 编码代理（定位与 Claude Code 类似）。

## 1. 分析方法

1. **黑盒命令面**：`--help` 与各子命令帮助，得到完整 CLI。
2. **静态字符串**：`strings` 约 8 万行，复原源码模块路径与日志事件名，确定内部模块划分。
3. **内嵌手册**：二进制内嵌约 350KB 的 `ante-guide` 使用手册，给出配置语义、权限模型、slash 命令、目录布局。
4. **协议探针**：`serve --stdio` 逐条发送 JSONL 操作（op），观察返回事件，确定 op/event 的 serde 形状。
5. **抓包对拍**：将 `HOME`/`ANTE_HOME` 指向临时目录，写自定义 `catalog.json` 把 `base_url` 指向本地 mock SSE 服务，捕获四种 wire style 的真实请求体；再让 mock 返回 tool_call，捕获工具循环的第二轮请求。
6. **落盘核对**：对比原件与替代实现产生的 catalog、doctor、auth、sessions 文件。

## 2. 内部架构（由字符串与行为推断）

```
src/
  main.rs / cli.rs          命令解析与分发
  runtime.rs                运行参数、provider/model 解析、系统提示词组装、工具选择
  catalog.rs                provider/model 目录（内嵌 + 用户 catalog.json 叠加）
  settings.rs               全局/档案/项目三层 settings
  credentials.rs            auth/credentials 凭据存储与解析
  llm.rs                    统一 Message / StreamEvent
  wire/                     openai_chat、openai_responses、anthropic、gemini
  client.rs                 HTTP/SSE、重试、错误分类
  agent.rs                  turn 循环、权限判定、子代理、压缩
  tools/                    bash、fs、web、todo、permission、mcp、scheduler
  session.rs / protocol.rs 会话持久化、JSONL 协议
  headless.rs / repl.rs     非交互与交互前端
  commands/                 auth/doctor/usage/rage/offline/update/serve
```

## 3. 配置与目录

- Ante home：`$ANTE_HOME`，否则 `~/.ante`。
- `settings.json`、`<profile>.settings.json`、`catalog.json`、`AGENTS.md`。
- 项目层 `.ante/settings.json` 只接受 session 相关键，不能放宽权限。
- 权限优先级：`deny > 会话授权 > ask > allow > 模式默认`；matcher 语法如 `Bash(cargo test *)`。
- 凭据：`auth/credentials/secret/<name>.json`（`{"value": "..."}`，0600）、`oauth/<name>.json`；
  旧 `api_keys.json` 会被读取兼容。
- 会话：`sessions/ses_<ULID>/meta.json`、`dialog.json`、`events.jsonl`。
- 日志：`logs/<date>/ante.<pid>.log`。

## 4. 四种 wire style（实测请求）

| wire_style | 路径 | 关键字段 |
|---|---|---|
| `OpenAiCompatible` | `POST {base}/chat/completions` | `model/messages/reasoning_effort/max_tokens/stream/stream_options{include_usage}/tools` |
| `AnthropicMessage` | `POST {base}/messages` | `model/max_tokens/messages/system[]/tools[{name,description,input_schema}]/stream/thinking{type,budget_tokens}/cache_control`，头含 `anthropic-version` |
| `OpenAiResponse` | `POST {base}/responses` | `model/instructions/input/tools[{type:function,...}]/tool_choice/parallel_tool_calls/max_output_tokens/reasoning{effort}/stream/include["reasoning.encrypted_content"]/store:false` |
| `Gemini` | `POST {base}/models/{model}:streamGenerateContent?alt=sse` | `contents/systemInstruction/generationConfig{maxOutputTokens,thinkingConfig}/tools[{functionDeclarations}]/toolConfig` |

认证默认 `Authorization: Bearer <cred>`；`credential_header` 可改发送头（如 `x-api-key`）。

## 5. 工具循环与事件

模型返回 tool_call 后，回传消息形如：

```json
{"role":"tool","content":"status: exited\nexit_code: 0\n\nstdout:\nhi\n","tool_call_id":"call_1"}
```

headless JSONL 事件序列：

```
SessionStart → UserInput → TurnStart → AgentMessage → UsageUpdate
→ ToolStart{id,name,args} → ToolEnd{tool_use_id,tool_name,status,result_json}
→ AgentMessage → UsageUpdate → TurnEnd{status,steps}
```

`serve --stdio` 的 op 外层为 `{"id":"op_<ULID>","op":{"Variant":...}}`；
`StartSession`/`Shutdown` 为 unit 变体，`UserInput` 为新类型字符串（`{"UserInput":"..."}`）。
`Shutdown` 先发 `SessionEnd`，再回 `"Goodbye"`。

## 6. 等效性验证结果

| 项目 | 结果 |
|---|---|
| `--help` / 子命令面 | 与原件一致 |
| `catalog` 输出 | 21 providers / 93 models，逐字段、逐顺序完全一致 |
| 四种 wire style 请求 | 路径、字段、流式解析均一致；均能对 mock 端到端出事件 |
| 工具调用循环 | ToolStart/ToolEnd、tool 消息内容与 `tool_call_id` 逐字一致 |
| `serve --stdio` | 事件种类与顺序一致（SessionStart/ExtensionRefreshed/UserInput/TurnStart/AgentMessageDelta/AgentMessage/UsageUpdate/TurnEnd/SessionEnd/Goodbye），JSON 键顺序一致 |
| `doctor` | 行项、列宽、警告文本一致 |
| `auth add/list/rm` | 消息文本、表格列、凭据路径 `secret/<name>.json` 与 `{"value"}` 内容一致 |
| 会话落盘 | 目录与会话文件名格式一致 |

## 7. 已知差异与限制

- `auth add` 的远程校验使用 `GET {base_url}/models` 近似原件行为；对不暴露该端点的 provider 只做本地存储，不保证与原件的私有校验接口完全一致。
- TUI（交互式全屏界面）未逐帧复刻，`repl.rs` 提供等价的流式 REPL；`--output-format` 的三种模式已实现。
- MCP、离线 llama.cpp 推理、自动更新下载仅保留命令面与基本流程，未做与原件的字节级对拍。
- 会话 `dialog.json` 的 system 分段数量取决于提示词裁剪策略，可能与原件在长上下文下存在差异。
