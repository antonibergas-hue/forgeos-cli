// SPDX-License-Identifier: BUSL-1.1
//! Local-only manifest helpers — `validate` previews a manifest without
//! touching the server. Full schema validation lives on the Python side
//! (`AgentManifest`); here we just surface the fields a user cares about.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub kind: Option<String>,
    pub metadata: Metadata,
    pub spec: Spec,
}

#[derive(Debug, Deserialize)]
pub struct Metadata {
    pub name: String,
    #[serde(default)]
    pub department: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Spec {
    #[serde(default = "default_stack")]
    pub stack: String,
    #[serde(default = "default_exec")]
    pub execution_type: String,
    #[serde(default = "default_ownership")]
    pub ownership: String,
    #[serde(default)]
    pub llm: Option<Llm>,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub event_triggers: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    /// SystemPrompt can be either a string or a `{path: "..."}` mapping;
    /// for the CLI preview we just count characters when it's a string.
    #[serde(default)]
    pub system_prompt: Option<serde_yaml::Value>,
}

#[derive(Debug, Deserialize)]
pub struct Llm {
    #[serde(default = "default_chat_model")]
    pub chat_model: String,
    #[serde(default = "default_provider")]
    pub provider: String,
}

fn default_stack() -> String {
    "forgeos".into()
}
fn default_exec() -> String {
    "event_driven".into()
}
fn default_ownership() -> String {
    "shared".into()
}
fn default_chat_model() -> String {
    "gpt-4o".into()
}
fn default_provider() -> String {
    "openai".into()
}

pub fn load(path: &Path) -> Result<Manifest> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "yaml" | "yml" => {
            serde_yaml::from_str(&raw).with_context(|| format!("parse YAML {}", path.display()))
        }
        "json" => {
            serde_json::from_str(&raw).with_context(|| format!("parse JSON {}", path.display()))
        }
        other => Err(anyhow!("unsupported manifest extension: .{other}")),
    }
}
