// SPDX-License-Identifier: BUSL-1.1
//! `forgeos deploy <file>` — POST the manifest YAML to the server.
//!
//! Two pieces of work happen client-side:
//!
//! 1. If the manifest references `system_prompt: { file: ... }`, we read
//!    that file relative to the manifest and inline its contents so the
//!    server doesn't need access to the operator's local filesystem.
//! 2. We then send the resulting YAML to /api/platform/agents/from-yaml
//!    as a raw text/yaml body, where `AgentManifest.from_dict` does the
//!    full schema validation. Keeps the Rust side schema-light.

use anyhow::{bail, Context, Result};
use clap::Args as ClapArgs;
use serde::Deserialize;
use serde_yaml::Value;
use std::path::PathBuf;

use crate::api::{self, Endpoint};
use crate::ui;

#[derive(ClapArgs)]
pub struct Args {
    /// Path to agent.yaml or agent.json
    pub file: PathBuf,
}

#[derive(Deserialize)]
struct DeployResponse {
    agent_id: String,
}

pub fn run(args: Args, ep: &Endpoint) -> Result<i32> {
    let raw = std::fs::read_to_string(&args.file)
        .with_context(|| format!("read {}", args.file.display()))?;

    if raw.contains("\nkind: Team") || raw.starts_with("kind: Team") {
        bail!("team manifests are not yet supported by the Rust CLI; split into individual agents");
    }

    let base = args.file.parent().unwrap_or_else(|| std::path::Path::new("."));
    let resolved = inline_system_prompt(&raw, base)?;
    let resp: DeployResponse =
        api::post_yaml(ep, "/api/platform/agents/from-yaml", &resolved)?;
    ui::ok(&format!("Deployed agent: {}", resp.agent_id));
    Ok(0)
}

/// Walk the parsed YAML, and when `spec.system_prompt` looks like
/// `{ file: "path/to/prompt.md" }`, read that path relative to the
/// manifest and replace it with `system_prompt: { content: <text> }`
/// so the server receives a self-contained document.
///
/// String form (`system_prompt: "literal"`) is left untouched.
fn inline_system_prompt(yaml: &str, base_dir: &std::path::Path) -> Result<String> {
    let mut doc: Value = serde_yaml::from_str(yaml).context("parse manifest YAML")?;
    let spec = doc
        .get_mut(Value::String("spec".into()))
        .and_then(|v| v.as_mapping_mut());
    let Some(spec_map) = spec else {
        return Ok(yaml.to_string());
    };
    let sp_key = Value::String("system_prompt".into());
    let Some(sp) = spec_map.get_mut(&sp_key) else {
        return Ok(yaml.to_string());
    };
    // Only act when system_prompt is a mapping with a `file:` key.
    let Some(map) = sp.as_mapping_mut() else {
        return Ok(yaml.to_string());
    };
    let file_key = Value::String("file".into());
    let Some(Value::String(rel)) = map.get(&file_key).cloned() else {
        return Ok(yaml.to_string());
    };
    let path = base_dir.join(&rel);
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("read system_prompt file {}", path.display()))?;
    map.remove(&file_key);
    map.insert(Value::String("content".into()), Value::String(content));
    serde_yaml::to_string(&doc).context("re-serialize manifest with inlined prompt")
}
