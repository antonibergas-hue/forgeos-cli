// SPDX-License-Identifier: BUSL-1.1
//! `forgeos edit <agent_id>` — open the agent's manifest in $EDITOR (git-commit
//! style) and apply the changes in place via PUT /api/platform/agents/{id}.
//!
//! Edge cases, mirroring `git commit` with no `-m`:
//!   * editor exits non-zero (e.g. vim `:cq`)  -> abort, agent untouched
//!   * buffer saved unchanged / closed unsaved -> abort, agent untouched
//!   * buffer emptied                          -> abort, agent untouched
//!   * YAML no longer parses                    -> abort, edits preserved on disk

use anyhow::{anyhow, bail, Context, Result};
use clap::Args as ClapArgs;
use serde_json::Value;
use serde_yaml::{Mapping, Value as Yaml};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use crate::api::{self, Endpoint};
use crate::ui;

#[derive(ClapArgs)]
pub struct Args {
    /// Agent ID (from `forgeos list`).
    pub agent_id: String,

    /// Editor command (overrides $FORGEOS_EDITOR / $VISUAL / $EDITOR).
    #[arg(long)]
    pub editor: Option<String>,
}

const HEADER: &str = "\
# forgeos edit — change the fields below and save to update the agent in place.
#
# Lines starting with '#' are ignored. Saving with NO changes, or leaving an
# empty buffer, aborts the edit — nothing is sent and the agent is untouched.
#
# Editable here: name, description, department, execution_type, schedule,
# chat_model, provider, goal, tools, event_triggers, metadata, system_prompt.
#
# Capabilities / boundaries / governance are NOT editable here — for those,
# `forgeos undeploy <id>` then `forgeos deploy <manifest>`.
#
# Notes: an empty `tools` or `event_triggers` list is ignored by the server
# (it will not clear them); set `schedule: \"\"` to clear a schedule.
";

pub fn run(args: Args, ep: &Endpoint) -> Result<i32> {
    let path = format!("/api/platform/agents/{}", args.agent_id);

    // 1. Fetch current state (404s here if the id is unknown).
    let current: Value = api::get(ep, &path)?;

    // 2. Render the honored, editable fields as YAML and prepend the header.
    let body = build_editable_yaml(&current)?;
    let template = format!("{HEADER}\n{body}");

    // 3. Drop it in a temp file.
    let tmp = temp_path(&args.agent_id);
    std::fs::write(&tmp, &template)
        .with_context(|| format!("write temp file {}", tmp.display()))?;

    // 4. Hand the terminal to the editor.
    let status = launch_editor(args.editor.as_deref(), &tmp)?;
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        bail!("editor exited with an error — edit aborted, agent unchanged");
    }

    // 5. Read it back and decide whether anything actually changed.
    let edited = std::fs::read_to_string(&tmp)
        .with_context(|| format!("read back {}", tmp.display()))?;

    if strip_comments(&edited).trim().is_empty() {
        let _ = std::fs::remove_file(&tmp);
        ui::warn("Empty manifest — edit aborted, agent unchanged.");
        return Ok(0);
    }
    if normalize(&edited) == normalize(&template) {
        let _ = std::fs::remove_file(&tmp);
        ui::warn("No changes — edit aborted, agent unchanged.");
        return Ok(0);
    }

    // 6. Parse the edited YAML. On failure, keep the file so work isn't lost.
    let parsed: Value = serde_yaml::from_str(&edited).map_err(|e| {
        anyhow!(
            "your edits are not valid YAML: {e}\n\
             They are preserved at {} — fix and re-apply.",
            tmp.display()
        )
    })?;
    let update = to_update_body(&parsed)
        .map_err(|e| anyhow!("{e}\nYour edits are preserved at {}", tmp.display()))?;

    // 7. Apply in place.
    let _resp: Value = api::put_json(ep, &path, &update)?;
    let _ = std::fs::remove_file(&tmp);
    ui::ok(&format!("Updated agent {}", args.agent_id));
    Ok(0)
}

/// Build the YAML the user edits — only the fields `update_agent` honors,
/// in a stable, human-friendly order.
fn build_editable_yaml(cur: &Value) -> Result<String> {
    let str_field = |k: &str| Yaml::from(cur.get(k).and_then(Value::as_str).unwrap_or("").to_string());
    let str_arr = |k: &str| {
        let items: Vec<Yaml> = cur
            .get(k)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(Yaml::from).collect())
            .unwrap_or_default();
        Yaml::Sequence(items)
    };

    let (chat_model, provider) = cur
        .get("llm_config")
        .and_then(Value::as_object)
        .map(|l| {
            (
                l.get("chat_model").and_then(Value::as_str).unwrap_or("").to_string(),
                l.get("provider").and_then(Value::as_str).unwrap_or("").to_string(),
            )
        })
        .unwrap_or_default();

    // metadata, minus internal `_`-prefixed keys (the server merges, so the
    // hidden ones survive untouched).
    let mut meta = Mapping::new();
    if let Some(obj) = cur.get("metadata").and_then(Value::as_object) {
        for (k, v) in obj {
            if k.starts_with('_') {
                continue;
            }
            meta.insert(
                Yaml::from(k.clone()),
                serde_yaml::to_value(v).unwrap_or(Yaml::Null),
            );
        }
    }

    let mut m = Mapping::new();
    m.insert(Yaml::from("name"), str_field("name"));
    m.insert(Yaml::from("description"), str_field("description"));
    m.insert(Yaml::from("department"), str_field("department"));
    m.insert(Yaml::from("execution_type"), str_field("execution_type"));
    m.insert(Yaml::from("schedule"), str_field("schedule"));
    m.insert(Yaml::from("chat_model"), Yaml::from(chat_model));
    m.insert(Yaml::from("provider"), Yaml::from(provider));
    m.insert(Yaml::from("goal"), str_field("goal"));
    m.insert(Yaml::from("tools"), str_arr("tools"));
    m.insert(Yaml::from("event_triggers"), str_arr("event_triggers"));
    m.insert(Yaml::from("metadata"), Yaml::Mapping(meta));
    m.insert(Yaml::from("system_prompt"), str_field("system_prompt"));

    serde_yaml::to_string(&Yaml::Mapping(m)).context("serialize editable manifest")
}

/// Validate the parsed edits and hand back the JSON body to PUT. Extra keys are
/// harmless (the server model ignores them); we just guarantee a usable `name`.
fn to_update_body(parsed: &Value) -> Result<Value> {
    let obj = parsed
        .as_object()
        .ok_or_else(|| anyhow!("manifest must be a YAML mapping of fields"))?;
    let name = obj.get("name").and_then(Value::as_str).unwrap_or("").trim();
    if name.is_empty() {
        bail!("`name` must not be empty");
    }
    Ok(parsed.clone())
}

/// Resolve and launch the editor, inheriting the current terminal so a
/// full-screen editor (vim, nano, …) works. Honors editor commands with
/// arguments, e.g. `code --wait` or `vim -p`.
fn launch_editor(override_editor: Option<&str>, file: &Path) -> Result<ExitStatus> {
    let editor = override_editor
        .map(str::to_string)
        .or_else(|| std::env::var("FORGEOS_EDITOR").ok())
        .or_else(|| std::env::var("VISUAL").ok())
        .or_else(|| std::env::var("EDITOR").ok())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "vi".to_string());

    let mut parts = editor.split_whitespace();
    let prog = parts
        .next()
        .ok_or_else(|| anyhow!("empty editor command"))?;
    Command::new(prog)
        .args(parts)
        .arg(file)
        .status()
        .with_context(|| format!("failed to launch editor '{editor}'"))
}

fn temp_path(agent_id: &str) -> PathBuf {
    let safe: String = agent_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .collect();
    std::env::temp_dir().join(format!("forgeos-edit-{safe}-{}.yaml", std::process::id()))
}

/// Drop whole-line comments — used only to decide whether the buffer is
/// effectively empty (the real parse runs against the raw text).
fn strip_comments(s: &str) -> String {
    s.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Trailing-whitespace-insensitive form so an editor's stray newline doesn't
/// read as a change.
fn normalize(s: &str) -> String {
    s.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}
