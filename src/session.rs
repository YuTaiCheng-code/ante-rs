use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

use crate::llm::{Block, Message, Role, Usage};
use crate::util::{ante_home, now_rfc3339};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub short_prompt: bool,
    pub dir: String,
    pub first_user_message: String,
    pub message_count: usize,
    pub usage: Usage,
    pub started_time: String,
    pub last_active_time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rolled_from: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DialogFile {
    system: Vec<String>,
    messages: Vec<MessageRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MessageRecord {
    role: String,
    blocks: Vec<BlockRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BlockRecord {
    Text { text: String },
    Thinking { text: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { id: String, name: String, content: String, is_error: bool },
    Image { media_type: String, data: String },
}

pub struct Session {
    pub meta: SessionMeta,
    pub system: Vec<String>,
    pub messages: Vec<Message>,
}

fn sessions_dir() -> PathBuf {
    ante_home().join("sessions")
}

impl Session {
    pub fn new(provider: impl Into<String>, model: impl Into<String>, short_prompt: bool) -> Session {
        let id = format!("ses_{}", crate::util::ulid());
        Session {
            meta: SessionMeta {
                id,
                provider: provider.into(),
                model: model.into(),
                short_prompt,
                dir: std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                first_user_message: String::new(),
                message_count: 0,
                usage: Usage::default(),
                started_time: now_rfc3339(),
                last_active_time: now_rfc3339(),
                rolled_from: None,
            },
            system: Vec::new(),
            messages: Vec::new(),
        }
    }

    pub fn dir(&self) -> PathBuf {
        sessions_dir().join(&self.meta.id)
    }

    pub fn note_user_message(&mut self, text: &str) {
        if self.meta.first_user_message.is_empty() {
            self.meta.first_user_message = crate::util::truncate(text, 500, "…");
        }
    }

    pub fn save(&mut self) -> Result<()> {
        self.meta.message_count = self.messages.len();
        self.meta.last_active_time = now_rfc3339();
        let dir = self.dir();
        std::fs::create_dir_all(&dir)?;
        crate::util::write_file(
            &dir.join("meta.json"),
            &serde_json::to_string_pretty(&self.meta)?,
        )?;
        let dialog = DialogFile {
            system: self.system.clone(),
            messages: self.messages.iter().map(to_record).collect(),
        };
        crate::util::write_file(
            &dir.join("dialog.json"),
            &serde_json::to_string_pretty(&dialog)?,
        )?;
        Ok(())
    }

    pub fn append_event(&self, event: &serde_json::Value) {
        let dir = self.dir();
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("events.jsonl"))
        {
            let _ = writeln!(f, "{}", event);
        }
    }

    pub fn list() -> Vec<SessionMeta> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(sessions_dir()) {
            for e in rd.flatten() {
                let p = e.path().join("meta.json");
                if let Ok(s) = std::fs::read_to_string(&p) {
                    if let Ok(m) = serde_json::from_str::<SessionMeta>(&s) {
                        out.push(m);
                    }
                }
            }
        }
        out.sort_by(|a, b| b.last_active_time.cmp(&a.last_active_time));
        out
    }

    pub fn load(id_or_prefix: &str) -> Result<Session> {
        let dir = sessions_dir().join(id_or_prefix);
        let dir = if dir.exists() {
            dir
        } else {
            Self::list()
                .into_iter()
                .find(|m| m.id.starts_with(id_or_prefix))
                .map(|m| sessions_dir().join(m.id))
                .ok_or_else(|| anyhow!("no session matching '{id_or_prefix}'"))?
        };
        let meta: SessionMeta =
            serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json"))?)?;
        let dialog: DialogFile = if dir.join("dialog.json").exists() {
            serde_json::from_str(&std::fs::read_to_string(dir.join("dialog.json"))?)?
        } else {
            DialogFile {
                system: vec![],
                messages: vec![],
            }
        };
        Ok(Session {
            meta,
            system: dialog.system,
            messages: dialog.messages.into_iter().map(from_record).collect(),
        })
    }
}

fn to_record(m: &Message) -> MessageRecord {
    MessageRecord {
        role: match m.role {
            Role::User => "user".into(),
            Role::Assistant => "assistant".into(),
        },
        blocks: m
            .blocks
            .iter()
            .map(|b| match b {
                Block::Text(t) => BlockRecord::Text { text: t.clone() },
                Block::Thinking(t) => BlockRecord::Thinking { text: t.clone() },
                Block::ToolUse { id, name, input } => BlockRecord::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                },
                Block::ToolResult { id, name, content, is_error } => BlockRecord::ToolResult {
                    id: id.clone(),
                    name: name.clone(),
                    content: content.clone(),
                    is_error: *is_error,
                },
                Block::Image { media_type, data } => BlockRecord::Image {
                    media_type: media_type.clone(),
                    data: data.clone(),
                },
            })
            .collect(),
    }
}

fn from_record(r: MessageRecord) -> Message {
    Message {
        role: if r.role == "assistant" { Role::Assistant } else { Role::User },
        blocks: r
            .blocks
            .into_iter()
            .map(|b| match b {
                BlockRecord::Text { text } => Block::Text(text),
                BlockRecord::Thinking { text } => Block::Thinking(text),
                BlockRecord::ToolUse { id, name, input } => Block::ToolUse { id, name, input },
                BlockRecord::ToolResult { id, name, content, is_error } => {
                    Block::ToolResult { id, name, content, is_error }
                }
                BlockRecord::Image { media_type, data } => Block::Image { media_type, data },
            })
            .collect(),
    }
}
