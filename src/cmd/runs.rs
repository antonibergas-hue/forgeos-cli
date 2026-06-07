// SPDX-License-Identifier: BUSL-1.1
//! `forgeos runs <subcommand>` — inspect runtime-v2 runs by their handle
//! (the continuation id returned by `invoke`). Backed by
//! GET /api/platform/runs/{run_id}.

use anyhow::Result;
use clap::Subcommand;
use colored::Colorize;
use serde::Deserialize;
use std::{thread, time::Duration};

use crate::api::{self, Endpoint};
use crate::ui;

#[derive(Subcommand)]
pub enum RunsCmd {
    /// Show a run's current status (running | paused | completed | failed).
    Status {
        run_id: String,
        /// Emit raw JSON (for piping into jq).
        #[arg(long)]
        json: bool,
    },
    /// Poll a run until it reaches a terminal state.
    Watch { run_id: String },
}

#[derive(Deserialize)]
struct Pending {
    request_id: Option<String>,
    tool: Option<String>,
}

#[derive(Deserialize)]
struct Run {
    run_id: Option<String>,
    status: Option<String>,
    suspend_reason: Option<String>,
    pending: Option<Vec<Pending>>,
    result: Option<String>,
    error: Option<String>,
}

pub fn run(cmd: RunsCmd, ep: &Endpoint) -> Result<i32> {
    match cmd {
        RunsCmd::Status { run_id, json } => status(ep, &run_id, json),
        RunsCmd::Watch { run_id } => watch(ep, &run_id),
    }
}

fn fetch(ep: &Endpoint, run_id: &str) -> Result<Run> {
    api::get(ep, &format!("/api/platform/runs/{run_id}"))
}

fn render(r: &Run) {
    let status = r.status.as_deref().unwrap_or("unknown");
    let badge = match status {
        "completed" => status.green(),
        "failed" => status.red(),
        "paused" => status.yellow(),
        _ => status.normal(),
    };
    println!("run {}  [{}]", r.run_id.as_deref().unwrap_or("?"), badge);
    if status == "paused" {
        if let Some(reason) = &r.suspend_reason {
            println!("  awaiting: {reason}");
        }
        for p in r.pending.iter().flatten() {
            let rid = p.request_id.as_deref().unwrap_or("?");
            let tool = p.tool.as_deref().unwrap_or("");
            println!("  approval {rid}  (tool: {tool})  → forgeos approvals approve {rid}");
        }
    }
    if let Some(out) = &r.result {
        if !out.is_empty() {
            println!("  result: {out}");
        }
    }
    if let Some(err) = &r.error {
        println!("  error: {err}");
    }
}

fn status(ep: &Endpoint, run_id: &str, json: bool) -> Result<i32> {
    if json {
        let v: serde_json::Value = api::get(ep, &format!("/api/platform/runs/{run_id}"))?;
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(0);
    }
    render(&fetch(ep, run_id)?);
    Ok(0)
}

fn watch(ep: &Endpoint, run_id: &str) -> Result<i32> {
    loop {
        let r = fetch(ep, run_id)?;
        render(&r);
        match r.status.as_deref() {
            Some("completed") => {
                ui::ok("run completed");
                return Ok(0);
            }
            Some("failed") => {
                ui::err("run failed");
                return Ok(1);
            }
            _ => thread::sleep(Duration::from_secs(2)),
        }
    }
}
