//! Goal-driven sessions: after every turn, a strict judge decides whether the
//! goal condition is met. Prompt text and response shape are recovered from
//! the original binary (`src/core/session/goal.rs` and the rodata next to it).

use std::sync::Arc;

use serde_json::Value;

use crate::llm::{ChatRequest, Message};

/// Built-in iteration budget; the original stops with
/// "iteration budget exhausted; stopping".
pub const MAX_ITERATIONS: u32 = 10;

pub const JUDGE_PROMPT: &str = "Stop. For this message only, act as a strict evaluator — not as the agent.\n\nJudge whether the following goal condition is now satisfied, using ONLY evidence already present in the conversation above.\n\nCONDITION:\n";

pub const JUDGE_RULES: &str = "\n\nRespond with a single JSON object and nothing else:\n{\"ok\": <bool>, \"impossible\": <bool>, \"reason\": \"<one sentence>\"}\n\nRules:\n- Set \"ok\" to true only with concrete evidence the condition holds (e.g. command or test output showing success). If the evidence is absent or you are unsure, set it false.\n- Set \"impossible\" to true only if the condition can never be satisfied — it is self-contradictory or fundamentally unreachable in this environment. Merely incomplete or hard is NOT impossible; when in doubt, set it false.\n- Never assume success. Always include a \"reason\".";

pub const HANDOFF_PROMPT: &str = "Write a handoff for a successor agent. The conversation above is being replaced by your handoff; the successor continues the work with it, any user messages preserved verbatim alongside it, and everything newer than what you can see, also verbatim.\n\nPoint to files and other recoverable sources rather than transcribing them. Spend the words on what cannot be recovered by looking: the user's intent and corrections, the decisions and the reasoning behind them, and the findings and uncertainties that took work to reach.\n\nDistinguish confirmed findings from hypotheses. Record what was changed, what was executed, and what was verified. Preserve pending job identifiers, status/output paths, and unknown outcomes.\n\nStructure it as:\n**Goal:** what the user asked for, in their own terms: intent, constraints, every correction, and every direction they rejected.\n**Done:** what is complete and how it was verified.\n**Learned & decided:** findings and decisions with the reasoning that still matters, including what was rejected and why, so it is not proposed again.\n**Index:** where things are: exact file paths, identifiers, commands, branches, URLs, and the short error messages that matter. Pointers, not contents.\n**In progress:** what was underway at the cut, and its exact state.\n**Next:** the immediate next steps, in order.\n\nOutput only the handoff.";

/// 生成交接摘要；失败时返回可在本地降级使用的摘要。
pub async fn handoff(rt: &Arc<crate::runtime::Runtime>, messages: &[Message], instructions: &str) -> String {
    let client = match rt.client() {
        Ok(c) => c,
        Err(e) => return format!("**Goal:** {instructions}\n**Done:** (handoff unavailable: {e})"),
    };
    let mut msgs: Vec<Message> = messages.to_vec();
    msgs.push(Message::user_text(format!("{HANDOFF_PROMPT}{instructions}")));
    let req = ChatRequest {
        model: rt.model.id.clone(),
        system: vec!["You write a handoff for the agent that continues this session.".into()],
        messages: msgs,
        tools: Vec::new(),
        max_tokens: 2048,
        effort: Some("low".into()),
        include_images: false,
    };
    match client.complete(req, |_| {}).await {
        Ok(r) if !r.text.trim().is_empty() => r.text,
        Ok(_) => format!("**Goal:** {instructions}\n**Done:** (the model returned an empty handoff)"),
        Err(e) => format!("**Goal:** {instructions}\n**Done:** (handoff failed: {e})"),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Met(String),
    Unreachable(String),
    Continue(String),
}

#[derive(Debug, Clone)]
pub struct Goal {
    pub condition: String,
    pub iteration: u32,
}

impl Goal {
    pub fn new(condition: String) -> Self {
        Goal { condition, iteration: 0 }
    }

    /// `/goal`, `/goal <condition>`, `/goal clear`.
    /// Returns a user-facing line, plus whether the goal is still active.
    pub fn command(arg: &str, current: &mut Option<Goal>) -> (String, bool) {
        let arg = arg.trim();
        if arg.is_empty() {
            return match current {
                Some(g) => (format!("Goal: {} — iteration {}/{}", g.condition, g.iteration, MAX_ITERATIONS), true),
                None => ("No goal is set. Usage: /goal <condition>".into(), false),
            };
        }
        if arg == "clear" {
            *current = None;
            return ("Goal cleared.".into(), false);
        }
        *current = Some(Goal::new(arg.to_string()));
        (
            format!("🎯 Goal set: {arg}\nI'll keep working until it's met. Use /goal clear (or interrupt) to stop."),
            true,
        )
    }
}

/// 与原件一致的判定器调用：不传工具，只让模型输出一个 JSON 对象。
pub async fn evaluate(rt: &Arc<crate::runtime::Runtime>, messages: &[Message], condition: &str) -> Verdict {
    let mut judge_messages: Vec<Message> = messages.to_vec();
    judge_messages.push(Message::user_text(format!("{JUDGE_PROMPT}{condition}{JUDGE_RULES}")));

    let req = ChatRequest {
        model: rt.model.id.clone(),
        system: vec![
            "Stop. For this message only, act as a strict evaluator — not as the agent.".into(),
        ],
        messages: judge_messages,
        tools: Vec::new(),
        max_tokens: 512,
        effort: Some("low".into()),
        include_images: false,
    };

    let client = match rt.client() {
        Ok(c) => c,
        Err(_) => return Verdict::Continue("the evaluator task failed".into()),
    };
    let resp = match client.complete(req, |_| {}).await {
        Ok(r) => r,
        Err(e) => return Verdict::Continue(format!("the evaluator task failed: {e}")),
    };
    parse_verdict(&resp.text)
}

pub fn parse_verdict(text: &str) -> Verdict {
    let Some(start) = text.find('{') else {
        return Verdict::Continue("goal: evaluation failed: no JSON verdict".into());
    };
    let Some(end) = text.rfind('}') else {
        return Verdict::Continue("goal: evaluation failed: no JSON verdict".into());
    };
    let raw = &text[start..=end];
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return Verdict::Continue(format!("goal: evaluation failed: {raw}"));
    };
    let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
    let impossible = v.get("impossible").and_then(|x| x.as_bool()).unwrap_or(false);
    let reason = v
        .get("reason")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if ok {
        Verdict::Met(reason)
    } else if impossible {
        Verdict::Unreachable(reason)
    } else {
        Verdict::Continue(reason)
    }
}

/// 单轮判定结果转成展示文案；`None` 表示清空目标。
pub fn apply(goal: &mut Option<Goal>, verdict: Verdict) -> (String, bool) {
    match verdict {
        Verdict::Met(reason) => {
            *goal = None;
            (format!("✅ Goal met: {reason}"), false)
        }
        Verdict::Unreachable(reason) => {
            *goal = None;
            (
                format!("🚫 Goal abandoned — judged unreachable: {reason}"),
                false,
            )
        }
        Verdict::Continue(reason) => {
            let Some(g) = goal.as_mut() else {
                return (String::new(), false);
            };
            g.iteration += 1;
            let iteration = g.iteration;
            if iteration >= MAX_ITERATIONS {
                *goal = None;
                return (
                    format!("🛑 Goal stopped after {iteration} iterations (not yet met): {reason}"),
                    false,
                );
            }
            let cond = g.condition.clone();
            (
                format!(
                    "🔁 Goal not yet met (iteration {iteration}/{MAX_ITERATIONS}): {reason}\nContinue working toward: {cond}"
                ),
                true,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_met_and_unreachable() {
        assert_eq!(parse_verdict("{\"ok\":true,\"impossible\":false,\"reason\":\"tests pass\"}"), Verdict::Met("tests pass".into()));
        assert_eq!(
            parse_verdict("noise {\"ok\":false,\"impossible\":true,\"reason\":\"contradiction\"} tail"),
            Verdict::Unreachable("contradiction".into())
        );
        assert_eq!(
            parse_verdict("{\"ok\":false,\"impossible\":false,\"reason\":\"not yet\"}"),
            Verdict::Continue("not yet".into())
        );
    }

    #[test]
    fn iteration_budget_stops() {
        let mut g = Some(Goal::new("all tests pass".into()));
        for _ in 0..MAX_ITERATIONS - 1 {
            let (line, active) = apply(&mut g, Verdict::Continue("not yet".into()));
            assert!(active, "{line}");
        }
        let (line, active) = apply(&mut g, Verdict::Continue("still failing".into()));
        assert!(!active);
        assert!(line.starts_with("🛑 Goal stopped after 10 iterations"));
    }

    #[test]
    fn commands() {
        let mut g = None;
        let (line, _) = Goal::command("cargo test passes", &mut g);
        assert!(line.contains("Goal set"));
        let (line, active) = Goal::command("", &mut g);
        assert!(line.starts_with("Goal: cargo test passes — iteration 0/"));
        assert!(active);
        let (line, active) = Goal::command("clear", &mut g);
        assert_eq!(line, "Goal cleared.");
        assert!(!active);
    }
}
