// SPDX-License-Identifier: BUSL-1.1
//! `forgeos approvals <subcommand>` — drive the A2H human-approval queue.

use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;
use serde_json::Value;

use crate::api::{self, Endpoint};
use crate::ui;

#[derive(Subcommand)]
pub enum ApprovalsCmd {
    /// List pending human-approval requests.
    List {
        /// Filter to requests fired by this agent_id.
        #[arg(long)]
        from_agent: Option<String>,
        /// Compact output (just request_id + a one-line summary).
        #[arg(short, long)]
        short: bool,
    },
    /// Approve a pending request.
    Approve {
        request_id: String,
        #[arg(long)]
        notes: Option<String>,
    },
    /// Reject a pending request.
    Reject {
        request_id: String,
        #[arg(long)]
        reason: Option<String>,
    },
}

#[derive(Serialize)]
struct ApproveBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
}

#[derive(Serialize)]
struct RejectBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// POST an approval (shared by the `approve` subcommand and the interactive
/// chat `[Y/n]` flow).
pub fn approve(ep: &Endpoint, request_id: &str, notes: Option<&str>) -> Result<()> {
    let _: Value = api::post_json(
        ep,
        &format!("/api/approvals/{request_id}/approve"),
        &ApproveBody { notes: notes.map(str::to_string) },
    )?;
    Ok(())
}

/// POST a rejection (shared by the `reject` subcommand and the chat flow).
pub fn reject(ep: &Endpoint, request_id: &str, reason: Option<&str>) -> Result<()> {
    let _: Value = api::post_json(
        ep,
        &format!("/api/approvals/{request_id}/reject"),
        &RejectBody { reason: reason.map(str::to_string) },
    )?;
    Ok(())
}

pub fn run(cmd: ApprovalsCmd, ep: &Endpoint) -> Result<i32> {
    match cmd {
        ApprovalsCmd::List { from_agent, short } => list(ep, from_agent, short),
        ApprovalsCmd::Approve { request_id, notes } => {
            approve(ep, &request_id, notes.as_deref())?;
            ui::ok(&format!("Approved {request_id}"));
            Ok(0)
        }
        ApprovalsCmd::Reject { request_id, reason } => {
            reject(ep, &request_id, reason.as_deref())?;
            ui::ok(&format!("Rejected {request_id}"));
            Ok(0)
        }
    }
}

fn list(ep: &Endpoint, from_agent: Option<String>, short: bool) -> Result<i32> {
    let path = match &from_agent {
        Some(a) => format!("/api/approvals?from_agent={a}"),
        None => "/api/approvals".to_string(),
    };
    let items: Vec<Value> = api::get(ep, &path)?;
    if items.is_empty() {
        ui::warn("No pending approvals");
        return Ok(0);
    }
    if short {
        println!(
            "{:<18}  {:<22}  {:<16}  {}",
            "REQUEST_ID", "RUN_ID", "ISSUE", "QUESTION"
        );
        println!(
            "{}  {}  {}  {}",
            "-".repeat(18), "-".repeat(22), "-".repeat(16), "-".repeat(40)
        );
        for r in &items {
            let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            // Runtime-v2: which run/continuation this approval blocks (empty
            // for legacy approvals not tied to a durable run).
            let run_id = r
                .get("run_id")
                .or_else(|| r.get("continuation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let run_short: String = run_id.chars().take(22).collect();
            // A2H request shape (a2h/v1):
            //   { content: { question, context: { issue_key, ... }, ... }, ... }
            let content = r.get("content").and_then(|v| v.as_object());
            let issue = content
                .and_then(|m| m.get("context"))
                .and_then(|v| v.as_object())
                .and_then(|m| m.get("issue_key"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let q = content
                .and_then(|m| m.get("question"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let q_short: String = q.chars().take(40).collect();
            println!("{id:<18}  {run_short:<22}  {issue:<16}  {q_short}");
        }
        // Footer goes to stderr so stdout stays pipeable (`forgeos approvals
        // list --short | awk` etc.).
        eprintln!("\n{} pending", items.len());
        return Ok(0);
    }
    // Full JSON dump — must be valid JSON on stdout so it pipes into jq.
    println!("{}", serde_json::to_string_pretty(&items)?);
    eprintln!("{} pending", items.len());
    Ok(0)
}
