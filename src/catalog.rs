use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use indexmap::IndexMap;
use std::collections::BTreeMap;

use crate::util::ante_home;

pub const BUILTIN_CATALOG: &str = include_str!("../resources/builtin_catalog.json");
pub const DEFAULT_PROVIDER: &str = "antix";
pub const DEFAULT_MODEL: &str = "gemini-4-argon";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModelSpec {
    pub id: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    pub context_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_efforts: Vec<String>,
    pub support_vision: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight_class: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProviderSpec {
    #[serde(default)]
    pub display_name: String,
    pub base_url: String,
    #[serde(default)]
    pub credential: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_header: Option<String>,
    pub wire_style: String,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub headers: IndexMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_display: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred_models: Vec<ModelSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_request_images: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_web_search: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CatalogFile {
    pub providers: IndexMap<String, ProviderSpec>,
    pub oauth_profiles: IndexMap<String, serde_json::Value>,
    pub helpers: IndexMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderView<'a> {
    pub id: &'a str,
    pub display_name: &'a str,
    pub base_url: &'a str,
    pub credential: &'a serde_json::Value,
    pub wire_style: &'a str,
    pub preferred_models: &'a Vec<ModelSpec>,
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub providers: IndexMap<String, ProviderSpec>,
    pub oauth_profiles: IndexMap<String, serde_json::Value>,
    pub helpers: IndexMap<String, serde_json::Value>,
    pub warnings: Vec<String>,
}

impl Catalog {
    pub fn load() -> Catalog {
        let mut warnings = Vec::new();
        let mut providers = IndexMap::new();
        let mut oauth_profiles = IndexMap::new();
        let mut helpers = IndexMap::new();

        match serde_json::from_str::<CatalogFile>(BUILTIN_CATALOG) {
            Ok(c) => {
                providers = c.providers;
                oauth_profiles = c.oauth_profiles;
                helpers = c.helpers;
            }
            Err(e) => warnings.push(format!("built-in catalog unparseable: {e}")),
        }

        let user_path = ante_home().join("catalog.json");
        if user_path.exists() {
            match std::fs::read_to_string(&user_path) {
                Ok(s) => match serde_json::from_str::<CatalogFile>(&s) {
                    Ok(c) => {
                        let mut entries: Vec<(String, ProviderSpec)> = c.providers.into_iter().collect();
                        entries.sort_by(|a, b| a.0.cmp(&b.0));
                        for (id, mut p) in entries {
                            if p.display_name.is_empty() {
                                p.display_name = id.clone();
                            }
                            if let Some(existing) = providers.get_mut(&id) {
                                // overlay: scalar fields replace when present, models merge by id
                                if !p.base_url.is_empty() {
                                    existing.base_url = p.base_url;
                                }
                                if !p.wire_style.is_empty() {
                                    existing.wire_style = p.wire_style;
                                }
                                if !p.credential.is_null() {
                                    existing.credential = p.credential;
                                }
                                if p.credential_header.is_some() {
                                    existing.credential_header = p.credential_header;
                                }
                                existing.headers.extend(p.headers);
                                for m in p.preferred_models {
                                    match existing.preferred_models.iter_mut().find(|x| x.id == m.id) {
                                        Some(old) => *old = m,
                                        None => existing.preferred_models.push(m),
                                    }
                                }
                            } else {
                                if p.wire_style.is_empty() {
                                    warnings.push(format!(
                                        "warning: skipped catalog entry {id}: missing `wire_style`"
                                    ));
                                    continue;
                                }
                                if p.base_url.is_empty() {
                                    warnings.push(format!(
                                        "warning: skipped catalog entry {id}: missing `base_url`"
                                    ));
                                    continue;
                                }
                                if p.credential.is_null() {
                                    p.credential = serde_json::Value::String(format!("secret:{id}"));
                                }
                                providers.insert(id, p);
                            }
                        }
                        oauth_profiles.extend(c.oauth_profiles);
                        helpers.extend(c.helpers);
                    }
                    Err(e) => warnings.push(format!(
                        "warning: skipped catalog entry: {}: {e}",
                        user_path.display()
                    )),
                },
                Err(e) => warnings.push(format!("{}: {e}", user_path.display())),
            }
        }

        // Drop models whose max_tokens >= context_limit.
        for (id, p) in providers.iter_mut() {
            let before = p.preferred_models.len();
            p.preferred_models.retain(|m| match (m.max_tokens, m.context_limit) {
                (Some(mt), Some(cl)) if mt >= cl => false,
                _ => {
                    let _ = id;
                    true
                }
            });
            if p.preferred_models.len() != before {
                warnings.push(format!(
                    "warning: skipped {} model(s) on {id}: max_tokens must be below context_limit",
                    before - p.preferred_models.len()
                ));
            }
        }

        for (name, profile) in &oauth_profiles {
            for err in validate_oauth_profile(profile) {
                warnings.push(format!("oauth profile `{name}`: {err}"));
            }
        }
        for (name, helper) in &helpers {
            for err in validate_helper(helper) {
                warnings.push(format!("helper `{name}`: {err}"));
            }
        }

        // A running offline router exposes an OpenAI-compatible endpoint. Main
        // publishes it through these variables before building the runtime.
        if let (Ok(base), Ok(model)) = (
            std::env::var("ANTE_OFFLINE_ENDPOINT"),
            std::env::var("ANTE_OFFLINE_MODEL"),
        ) {
            providers.insert(
                "offline".into(),
                ProviderSpec {
                    display_name: "Ante Offline (llama.cpp)".into(),
                    base_url: base,
                    credential: serde_json::json!([]),
                    credential_header: None,
                    wire_style: "OpenAiCompatible".into(),
                    headers: IndexMap::new(),
                    thinking_display: None,
                    preferred_models: vec![ModelSpec {
                        id: model.clone(),
                        description: "local GGUF served by the pinned llama.cpp router".into(),
                        max_tokens: Some(8192),
                        context_limit: Some(32768),
                        ..ModelSpec::default()
                    }],
                    max_request_images: None,
                    supports_web_search: Some(false),
                },
            );
        }

        Catalog {
            providers,
            oauth_profiles,
            helpers,
            warnings,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        let mut providers = Vec::new();
        for (id, p) in &self.providers {
            let v = serde_json::to_value(p).unwrap_or_default();
            let mut obj = serde_json::Map::new();
            obj.insert("id".into(), serde_json::Value::String(id.clone()));
            if let Some(o) = v.as_object() {
                for (k, val) in o {
                    obj.insert(k.clone(), val.clone());
                }
            }
            let v = serde_json::Value::Object(obj);
            providers.push(v);
        }
        serde_json::json!({ "providers": providers })
    }

    pub fn provider(&self, id: &str) -> Option<&ProviderSpec> {
        self.providers.get(id)
    }

    pub fn default_provider(&self) -> String {
        if self.providers.contains_key(DEFAULT_PROVIDER) {
            DEFAULT_PROVIDER.to_string()
        } else {
            self.providers.keys().next().cloned().unwrap_or_default()
        }
    }

    pub fn model(&self, provider: &str, model: &str) -> Option<ModelSpec> {
        self.providers
            .get(provider)
            .and_then(|p| p.preferred_models.iter().find(|m| m.id == model).cloned())
    }

    pub fn default_model(&self, provider: &str) -> Option<ModelSpec> {
        let p = self.providers.get(provider)?;
        p.preferred_models
            .iter()
            .find(|m| m.id == DEFAULT_MODEL)
            .or_else(|| p.preferred_models.first())
            .cloned()
    }

    /// Find the provider that lists a model id, preferring the given provider.
    pub fn resolve_model(
        &self,
        provider_hint: Option<&str>,
        model_hint: Option<&str>,
    ) -> (String, ModelSpec) {
        let provider = provider_hint
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.default_provider());
        let model_id = model_hint.map(|s| s.to_string());
        if let Some(p) = self.providers.get(&provider) {
            if let Some(id) = &model_id {
                if let Some(m) = p.preferred_models.iter().find(|m| m.id == *id) {
                    return (provider, m.clone());
                }
            }
            if let Some(m) = self.default_model(&provider) {
                return (provider, m);
            }
        }
        // fall back to any provider that has the model, else first model anywhere
        if let Some(id) = model_id.clone() {
            for (pid, p) in &self.providers {
                if let Some(m) = p.preferred_models.iter().find(|m| m.id == id) {
                    return (pid.clone(), m.clone());
                }
            }
        }
        for (pid, p) in &self.providers {
            if let Some(m) = p.preferred_models.first() {
                return (pid.clone(), m.clone());
            }
        }
        (
            provider,
            ModelSpec {
                id: model_id.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
                description: String::new(),
                temperature: None,
                top_p: None,
                max_tokens: Some(8192),
                context_limit: Some(200000),
                effort: None,
                supported_efforts: vec![],
                support_vision: false,
                weight_class: None,
            },
        )
    }

    pub fn require_provider(&self, id: &str) -> Result<&ProviderSpec> {
        self.providers
            .get(id)
            .ok_or_else(|| anyhow!("unknown provider '{id}'"))
    }
}

pub fn wire_style_known(style: &str) -> bool {
    matches!(
        style,
        "OpenAiCompatible" | "OpenAiResponse" | "AnthropicMessage" | "Gemini"
    )
}

fn is_absolute_http(s: &str) -> bool {
    (s.starts_with("http://") || s.starts_with("https://")) && s.len() > 8
}

fn validate_oauth_profile(v: &serde_json::Value) -> Vec<String> {
    let mut errs = Vec::new();
    let Some(obj) = v.as_object() else {
        return vec!["must be an object".into()];
    };
    for field in ["client_id", "redirect_uri", "authorize_url", "token_url"] {
        match obj.get(field).and_then(|x| x.as_str()) {
            Some(s) if !s.trim().is_empty() => {}
            _ => errs.push(format!("missing `{field}`")),
        }
    }
    if let Some(s) = obj.get("redirect_uri").and_then(|x| x.as_str()) {
        if !(s.starts_with("http://localhost") || s.starts_with("http://127.0.0.1")) {
            errs.push("`redirect_uri` must start with http://localhost or http://127.0.0.1".into());
        }
    }
    for field in ["authorize_url", "token_url"] {
        if let Some(s) = obj.get(field).and_then(|x| x.as_str()) {
            if !is_absolute_http(s) {
                errs.push(format!("`{field}` must be an absolute http(s) URL"));
            }
        }
    }
    if let Some(v) = obj.get("token_request_encoding") {
        match v.as_str() {
            Some("json") | Some("form") => {}
            _ => errs.push("`token_request_encoding` must be `json` or `form`".into()),
        }
    }
    for field in ["scopes", "required_scopes", "account_claim"] {
        if let Some(v) = obj.get(field) {
            match v.as_array() {
                Some(a) if a.iter().all(|x| x.is_string()) => {}
                _ => errs.push(format!("`{field}` must be an array of strings")),
            }
        }
    }
    if let Some(v) = obj.get("derived_headers") {
        match v.as_array() {
            Some(items) if items.iter().all(|h| {
                h.get("name").and_then(|x| x.as_str()).is_some()
                    && h.get("jwt_claim").and_then(|x| x.as_array()).is_some()
            }) => {}
            _ => errs.push("`derived_headers` must be [{\"name\", \"jwt_claim\": [...]}]".into()),
        }
    }
    if let Some(v) = obj.get("authorize_params") {
        if v.as_object().map(|o| o.values().all(|x| x.is_string())) != Some(true) {
            errs.push("`authorize_params` must be {\"name\": \"value\"}".into());
        }
    }
    errs
}

fn validate_helper(v: &serde_json::Value) -> Vec<String> {
    let mut errs = Vec::new();
    let Some(obj) = v.as_object() else {
        return vec!["must be an object".into()];
    };
    match obj.get("command") {
        Some(serde_json::Value::Array(items))
            if !items.is_empty() && items.iter().all(|x| x.is_string()) => {}
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {}
        _ => errs.push("`command` must be a non-empty array of strings".into()),
    }
    if let Some(ttl) = obj.get("ttl_secs") {
        match ttl.as_u64() {
            Some(n) if n <= 31_536_000 => {}
            _ => errs.push("`ttl_secs` must be a number up to 31536000".into()),
        }
    }
    errs
}
