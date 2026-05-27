// SPDX-License-Identifier: BUSL-1.1
//! `forgeos describe <agent_id>` — full manifest + live status for one agent.
//! Backed by GET /api/platform/agents/{id}.

use anyhow::Result;
use clap::Args as ClapArgs;
use colored::Colorize;
use serde_json::Value;

use crate::api::{self, Endpoint};

#[derive(ClapArgs)]
pub struct Args {
    /// Agent ID (from `forgeos list`).
    pub agent_id: String,

    /// Emit raw JSON instead of the formatted view (for piping into jq).
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: Args, ep: &Endpoint) -> Result<i32> {
    let path = format!("/api/platform/agents/{}", args.agent_id);
    let d: Value = api::get(ep, &path)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&d).unwrap_or_default());
        return Ok(0);
    }

    let s = |k: &str| {
        d.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("—")
            .to_string()
    };
    let field = |label: &str, val: &str| {
        println!("{:<14} {}", format!("{label}:").dimmed(), val);
    };

    // Header: name + description.
    println!("{}", s("name").bold());
    if let Some(desc) = d.get("description").and_then(|v| v.as_str()) {
        if !desc.is_empty() {
            println!("{}", desc.dimmed());
        }
    }
    println!();

    field("id", &args.agent_id);
    field("stack", &s("stack"));
    field("type", &s("execution_type"));
    let sched = d
        .get("schedule")
        .and_then(|v| v.as_str())
        .filter(|x| !x.is_empty())
        .unwrap_or("—");
    field("schedule", sched);

    let status = s("status");
    let status_c = match status.as_str() {
        "running" => status.cyan(),
        "completed" | "idle" => status.green(),
        "failed" => status.red(),
        _ => status.normal(),
    };
    println!("{:<14} {}", "status:".dimmed(), status_c);

    field("department", &s("department"));
    field("ownership", &s("ownership"));

    if let Some(llm) = d.get("llm_config").and_then(|v| v.as_object()) {
        let model = llm.get("chat_model").and_then(|v| v.as_str()).unwrap_or("—");
        let provider = llm.get("provider").and_then(|v| v.as_str()).unwrap_or("—");
        field("model", &format!("{model} ({provider})"));
    }

    if let Some(tools) = d.get("tools").and_then(|v| v.as_array()) {
        let names: Vec<String> = tools
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        println!("{:<14} {}", "tools:".dimmed(), names.join(", ").cyan());
    }

    // Metadata (skip internal _-prefixed keys).
    if let Some(meta) = d.get("metadata").and_then(|v| v.as_object()) {
        let visible: Vec<(&String, &Value)> =
            meta.iter().filter(|(k, _)| !k.starts_with('_')).collect();
        if !visible.is_empty() {
            println!("{}", "metadata:".dimmed());
            for (k, v) in visible {
                let val = v.as_str().map(String::from).unwrap_or_else(|| v.to_string());
                println!("    {k:<16} {val}");
            }
        }
    }

    if let Some(sp) = d.get("system_prompt").and_then(|v| v.as_str()) {
        if !sp.is_empty() {
            println!("\n{}", "system_prompt:".dimmed());
            for line in sp.lines() {
                println!("    {line}");
            }
        }
    }

    Ok(0)
}
