// SPDX-License-Identifier: BUSL-1.1
//! `forgeos logs <agent_id>` — merged stream of run events + tool calls
//! for a specific agent. Backed by GET /api/platform/agent-logs.

use anyhow::Result;
use clap::Args;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::thread;
use std::time::Duration;

use crate::api::{self, Endpoint};

#[derive(Args)]
pub struct Cmd {
    /// Agent ID (from `forgeos list`).
    pub agent_id: String,

    /// Number of events to fetch on each poll. Default 200.
    #[arg(long, default_value_t = 200)]
    pub tail: usize,

    /// Re-poll every 2s and emit new events as they arrive.
    #[arg(short, long)]
    pub follow: bool,

    /// Emit raw JSON instead of pretty lines (for piping into jq).
    #[arg(long)]
    pub json: bool,
}

#[derive(Deserialize)]
struct Envelope {
    events: Vec<Event>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
struct Event {
    ts: Option<String>,
    agent_id: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    description: Option<String>,
    details: Option<Value>,
}

pub fn run(args: Cmd, ep: &Endpoint) -> Result<i32> {
    let path = format!(
        "/api/platform/agent-logs?agent_id={}&limit={}",
        args.agent_id, args.tail
    );

    // First page: fetch + emit oldest → newest so the human reads top to bottom.
    let env: Envelope = api::get(ep, &path)?;
    let mut events = env.events;
    events.reverse();
    if args.json {
        for e in &events {
            println!("{}", serde_json::to_string(e).unwrap_or_default());
        }
    } else {
        for e in &events {
            print_event(e);
        }
    }

    if !args.follow {
        return Ok(0);
    }

    // Follow mode: poll every 2s, dedupe on a (ts,kind,desc) signature.
    let mut seen: HashSet<String> = events.iter().map(signature).collect();
    loop {
        thread::sleep(Duration::from_secs(2));
        let env: Envelope = match api::get(ep, &path) {
            Ok(v) => v,
            Err(_) => continue, // transient; keep polling
        };
        let mut fresh = env.events;
        fresh.reverse();
        for e in &fresh {
            let sig = signature(e);
            if seen.insert(sig) {
                if args.json {
                    println!("{}", serde_json::to_string(e).unwrap_or_default());
                } else {
                    print_event(e);
                }
            }
        }
    }
}

fn signature(e: &Event) -> String {
    format!(
        "{}|{}|{}",
        e.ts.clone().unwrap_or_default(),
        e.kind.clone().unwrap_or_default(),
        e.description.clone().unwrap_or_default()
    )
}

fn print_event(e: &Event) {
    let ts = e.ts.as_deref().unwrap_or("-");
    let kind = e.kind.as_deref().unwrap_or("?");
    let desc = e.description.as_deref().unwrap_or("");

    // Pull a few common detail fields when present.
    let det = e.details.as_ref().and_then(|v| v.as_object());
    let extra = det
        .map(|d| {
            let mut parts = vec![];
            if let Some(t) = d.get("trigger").and_then(|v| v.as_str()) {
                parts.push(format!("trigger={t}"));
            }
            if let Some(dur) = d.get("duration_ms").and_then(|v| v.as_i64()) {
                parts.push(format!("{:.1}s", (dur as f64) / 1000.0));
            }
            if let Some(err) = d.get("error").and_then(|v| v.as_str()) {
                if !err.is_empty() {
                    parts.push(format!("error={err}"));
                }
            }
            parts.join(" ")
        })
        .unwrap_or_default();

    let kind_colored = match kind {
        k if k.starts_with("run.started") => kind.cyan().to_string(),
        k if k.starts_with("run.completed") => kind.green().to_string(),
        k if k.starts_with("run.failed") => kind.red().to_string(),
        k if k.starts_with("tool.") => kind.yellow().to_string(),
        _ => kind.dimmed().to_string(),
    };

    if extra.is_empty() {
        println!("{}  {:<16}  {}", ts.dimmed(), kind_colored, desc);
    } else {
        println!(
            "{}  {:<16}  {}  {}",
            ts.dimmed(),
            kind_colored,
            desc,
            extra.dimmed()
        );
    }

    // When the platform attaches stdout/stderr/files_changed/pr_url to a
    // dev-tool tool.call, render them as indented continuation lines so the
    // operator can see what opencode / pnpm / gh actually said.
    if let Some(d) = det {
        // Show the call arguments (e.g. the gcloud command, email subject)
        // on an indented continuation line so the operator sees *what* the
        // tool was invoked with, not just that it was.
        if kind.starts_with("tool.") {
            if let Some(a) = d.get("args") {
                if !a.is_null() {
                    let rendered = match a {
                        Value::Object(_) | Value::Array(_) => {
                            serde_json::to_string(a).unwrap_or_default()
                        }
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    if !rendered.is_empty() && rendered != "{}" {
                        println!("    {} {}", "args:".dimmed(), rendered.dimmed());
                    }
                }
            }
        }
        if let Some(s) = d.get("stdout_tail").and_then(|v| v.as_str()) {
            print_block("stdout", s, false);
        }
        if let Some(s) = d.get("stderr_tail").and_then(|v| v.as_str()) {
            print_block("stderr", s, true);
        }
        if let Some(arr) = d.get("files_changed").and_then(|v| v.as_array()) {
            let names: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if !names.is_empty() {
                println!("    {} {}", "files:".dimmed(), names.join(" ").cyan());
            }
        }
        if let Some(u) = d.get("pr_url").and_then(|v| v.as_str()) {
            if !u.is_empty() {
                println!("    {} {}", "pr:".dimmed(), u.green().bold());
            }
        }
    }
}

fn print_block(label: &str, text: &str, is_err: bool) {
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return;
    }
    // Indent each line so the block stays visually attached to its event.
    let label_colored = if is_err {
        label.red().to_string()
    } else {
        label.dimmed().to_string()
    };
    println!("    {label_colored}");
    for line in trimmed.lines() {
        if is_err {
            println!("    │ {}", line.dimmed());
        } else {
            println!("    │ {line}");
        }
    }
}
