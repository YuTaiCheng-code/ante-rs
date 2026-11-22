# ante-rs

> **仅供学习与研究（FOR STUDY / RESEARCH ONLY）**
>
> 本项目是对 Antigma Labs 的 `ante` 0.3.4（终端 AI 编码代理）的独立逆向学习与 Rust 重实现。**不得用于商业用途，不得用于绕过授权、付费、风控或访问控制。**
> 仓库不包含原始二进制、内嵌手册或专有资源；"Ante"、"Antigma" 等名称与商标归 Antigma Labs 所有，本项目与其无隶属或背书关系。

## 预编译二进制（Linux，静态 musl，含真彩色 TUI）

| 架构 | 文件 | 大小 | SHA-256 |
| --- | --- | --- | --- |
| x86_64 | [`ante-v0.3.4-x86_64-linux-musl`](ante-v0.3.4-x86_64-linux-musl) | 4.9 MB | `d0578794d96486407f76364688beab7676febdf961d93053d3814eb0e7261dd3` |
| aarch64 | [`ante-v0.3.4-aarch64-linux-musl`](ante-v0.3.4-aarch64-linux-musl) | 4.0 MB | `757d93bef7b165ba67301b7105269ab1c31be17f6459c8d0c6650e86d8164304` |

校验：`sha256sum -c SHA256SUMS.txt`

```bash
chmod +x ante-v0.3.4-x86_64-linux-musl
./ante-v0.3.4-x86_64-linux-musl --version   # ante 0.3.4
```

## 源码包

[`ante-rs-0.3.4-reimpl.tar.gz`](ante-rs-0.3.4-reimpl.tar.gz)（含 `src/`、`resources/`、测试、黄金帧与说明文档）

```bash
tar xzf ante-rs-0.3.4-reimpl.tar.gz
cd ante-rs
cargo build --release
```

## TUI 颜色（与原件对齐）

原件为 24-bit 真彩色，本实现已补上对应色值：Logo 橙色逐列渐变（`#ffaf00` → `#d78700` → `#af8700` → `#af5f00`）、标题 `#ff875f`、目录与次要路径 `#5f5f87`、未连接提示 `#87afaf`。

![欢迎页](welcome.png)

![命令面板](palette.png)

> 截图是配色修复前的灰度版，验证色值请在终端直接运新二进制。

## 技术要点

- Rust 1.80+；四种 wire style：`/chat/completions`、`/messages`、`/responses`、`/models/{model}:streamGenerateContent?alt=sse`。
- `serve --stdio` 事件键顺序 `timestamp,id,event,parent`。
- 审批 5 选项，规则泛化如 `cargo test -p foo` → `Bash(cargo test *)`。
- 目标闭环、`/term` 持久终端、会话 rollover handoff 已实现。

## 许可证

MIT；第三方名称与商标不随本许可证授权。使用前请阅读 [LEGAL_NOTICE.md](LEGAL_NOTICE.md)。
