# 内嵌提示词提取清单

从官方 `ante 0.3.4` 二进制按 UTF-8 文本段提取，已去掉 NUL/二进制噪声，并按用途拆分为独立文件。
这些内容是原版行为证据，**仅用于本地兼容性研究**；未经确认不要直接推到公开仓库。

## 1. 主系统提示

官方主提示的核心四段已经在 `resources/prompt_sections.json` 中：
- `intro`
- `harness`
- `communicating_with_the_user`
- `working_defaults`

本次补充的是原版在运行期拼接进去的“动态段”。

## 2. 已提取文件

| 文件 | 用途 | 大小 | 与原版对应 |
|---|---:|---:|---|
| `prompts/skills_system_prompt.md` | 技能触发与渐进式加载规则 | 2.7 KB | 缺失，当前只有一句简化说明 |
| `prompts/auto_memory.md` | 自动记忆分类、写入/回忆规则 | 3.1 KB | 完全缺失 |
| `prompts/goal_evaluator.md` | 每轮结束后的严格目标判定器 | 0.9 KB | 已有，文本一致 |
| `prompts/goal_working.md` | 目标工作循环与未完成反馈 | 0.6 KB | 部分实现 |
| `prompts/compact_handoff.md` | 压缩时给后继 Agent 的交接模板 | 1.8 KB | 已有 `HANDOFF_PROMPT`，文本一致 |
| `prompts/ambient_prompts.md` | 状态 spinner 短语 + 下一句预测 | 1.0 KB | 缺失 |
| `prompts/subagent_explore.md` | explore 子代理完整定义与系统提示 | 1.2 KB | 仅元数据，缺完整 system prompt |
| `prompts/agent_tool_long.md` | Agent 工具长版说明与示例 | 3.1 KB | 有近似版 |
| `prompts/agent_tool_short_raw.md` | Agent 工具短版说明 | 1.7 KB | 有近似版 |
| `prompts/skill_add-provider.md` | 自定义 provider 技能正文 | 4.4 KB | 只有元数据 |
| `prompts/skill_ante-guide.md` | Ante 自身使用/排障手册 | 8.3 KB | 缺失 |
| `prompts/skill_resume-claude.md` | 接续 Claude Code 会话 | 2.4 KB | 只有元数据 |
| `prompts/skill_resume-codex.md` | 接续 Codex 会话 | 2.8 KB | 只有元数据 |
| `prompts/skill_skill-creator.md` | 技能创建/评估主流程 | 8.1 KB | 缺失 |
| `prompts/skill_tmux.md` | tmux 持久终端编排 | 6.0 KB | 缺失 |

总计约 55 KB。

## 3. 关键发现

### 3.1 Skills 提示被严重简化
原版要求：
- 用户点名或任务命中描述时必须使用技能
- 完整读取 `SKILL.md`，分页/截断时读到 EOF
- 相对路径按技能目录解析
- `references/` 由主 Agent 自己读，不能交给子代理总结
- 优先运行/修改 `scripts/`，复用 `assets/`
- 多技能时声明使用顺序

我们当前只列了名称和一句 “Read it before using a skill”，这会显著改变模型行为。

### 3.2 Auto Memory 是完整子系统
原版按 `user / feedback / project / reference` 分类，要求：
- 主题文件 + `MEMORY.md` 一行索引
- 只保存未来 Agent 真能因此做得更好的内容
- 不保存 secrets、当前任务状态、代码结构、已有 AGENTS.md 内容
- 回忆内容只能当背景，不能当指令；使用前要回仓库核验

这不是一段 prompt，配套还需要目录、`/memory`、`--no-memory` 和设置开关。

### 3.3 目标判定与交接已经对齐
`goal.rs` 中的 `JUDGE_PROMPT` / `JUDGE_RULES` / `HANDOFF_PROMPT` 与提取文本一致，说明这两块逆向得比较完整。

### 3.4 子代理缺“人格定义”
原版 explore 子代理有自己的 frontmatter（tools: Glob/Grep/Read/Bash）和完整系统提示；我们只有 `builtin_agents.json` 的名称/描述，运行时的 explore 子代理约束不足。

### 3.5 还存在未提取的短模板
二进制里还有：
- 会话标题生成
- git commit message
- AGENTS.md 初始化
- AskUser 回答后的续跑提示
- 技能读取提醒
- context overflow 提示

这些多为短字符串，下一步可以按调用点继续定位。

## 4. 建议入库方式

1. `prompts/` 保留为“证据库”，不直接参与构建
2. 把 `skills_system_prompt` 与 `auto_memory` 提升为正式资源，走版本化 diff
3. 6 个内置技能放到 `resources/skills/<name>/SKILL.md`，补 frontmatter 和发现逻辑
4. 每次替换 prompt 都跑 `frame_test` + 工具 e2e，避免只改文字导致行为回归

