# ante-rs

> **仅供学习与研究（FOR STUDY / RESEARCH ONLY）**
>
> 本项目是对 Antigma Labs 的 `ante` 0.3.4（终端 AI 编码代理）的独立逆向学习与 Rust 重实现。**不得用于商业用途，不得用于绕过授权、付费、风控或访问控制。**
> 仓库不包含原始二进制、内嵌手册或专有资源；"Ante"、"Antigma" 等名称与商标归 Antigma Labs 所有，本项目与其无隶属或背书关系。

## 附件

- 源码包：[`ante-rs-0.3.4-reimpl.tar.gz`](ante-rs-0.3.4-reimpl.tar.gz)（含 `src/`、`resources/`、测试、黄金帧与说明文档；不含原始二进制）

```bash
tar xzf ante-rs-0.3.4-reimpl.tar.gz
cd ante-rs
cargo build --release
```

## TUI 视觉验证（真实 GUI 终端 xterm 120×40）

![欢迎页](welcome.png)

![命令面板](palette.png)

欢迎页、composer、命令面板与原件黄金帧逐字符比对为 9/9，相似度 1.000。

## 技术要点

- Rust 1.80+，`cargo build --release`，产物约 4.9 MB，无系统库依赖。
- 协议：`serve --stdio` 的 op 外层为 `{"id":"op_<ULID>","op":{"Variant":...}}`；事件键顺序 `timestamp,id,event,parent`。
- 四种 wire style：`OpenAiCompatible` → `/chat/completions`；`AnthropicMessage` → `/messages`；`OpenAiResponse` → `/responses`；`Gemini` → `/models/{model}:streamGenerateContent?alt=sse`。
- TUI：crossterm raw mode + alternate screen + 手写 ANSI 定位。
- 审批：5 选项（本次 / 会话 / 永久写入 settings / 拒绝 / 拒绝并反馈），规则泛化如 `cargo test -p foo` → `Bash(cargo test *)`。
- 目标闭环、`/term` 持久终端分屏、会话 rollover handoff 已实现，本地 mock SSE 端到端测试通过。

## 测试

```bash
python3 tests/frame_test.py            # 9/9
python3 tests/parity_test.py --orig ./ante   # 6/6
python3 tests/docs_conformance_test.py # 4/4
python3 tests/goal_e2e_test.py         # 5/5
python3 tests/features_e2e_test.py     # 5/5
python3 tests/approval_e2e_test.py     # 4/4
```

## 许可证

源码以 MIT 发布；第三方名称与商标不随本许可证授权。使用前请阅读 [LEGAL_NOTICE.md](LEGAL_NOTICE.md)。
