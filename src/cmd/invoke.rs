// SPDX-License-Identifier: BUSL-1.1
use anyhow::Result;
use clap::Args as ClapArgs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{thread, time::Duration};

use crate::api::{self, Endpoint};
use crate::ui;

#[derive(ClapArgs)]
pub struct Args {
    pub agent_id: String,
    pub prompt: String,
    /// Wait for the run to finish (or pause for approval) and print the result.
    /// Default is fire-and-return: queue the run and exit immediately.
    #[arg(short, long)]
    pub wait: bool,
    /// If the run pauses on a human-approval gate, keep polling after you
    /// approve it (in another terminal) until it finishes. Implies --wait.
    #[arg(long)]
    pub wait_approvals: bool,
}

#[derive(Serialize)]
struct InvokeRequest<'a> {
    prompt: &'a str,
    context: Value,
}

#[derive(Deserialize)]
struct Pending {
    request_id: Option<String>,
    tool: Option<String>,
}

/// Runtime-v2 run handle. All fields optional so legacy responses (just
/// `{result, simulated, warnings}`) deserialize cleanly into the `None` arms.
#[derive(Deserialize)]
struct RunHandle {
    run_id: Option<String>,
    status: Option<String>,
    result: Option<String>,
    error: Option<String>,
    suspend_reason: Option<String>,
    pending: Option<Vec<Pending>>,
    warnings: Option<Vec<String>>,
    simulated: Option<bool>,
}

pub fn run(args: Args, ep: &Endpoint) -> Result<i32> {
    let wait = args.wait || args.wait_approvals;

    // Default: fire-and-return via the server's async_mode.
    if !wait {
        let path = format!("/api/platform/agents/{}/invoke?async_mode=true", args.agent_id);
        let _: Value = api::post_json(
            ep,
            &path,
            &InvokeRequest { prompt: &args.prompt, context: Value::Object(Default::default()) },
        )?;
        ui::ok(&format!("Invoked {} — run queued.", args.agent_id));
        println!("  Watch it:  forgeos logs {} --follow", args.agent_id);
        return Ok(0);
    }

    let path = format!("/api/platform/agents/{}/invoke", args.agent_id);
    let handle: RunHandle = api::post_json(
        ep,
        &path,
        &InvokeRequest { prompt: &args.prompt, context: Value::Object(Default::default()) },
    )?;

    render(&handle);

    // If parked on approval and the user asked to wait it out, poll the run
    // until it reaches a terminal state (after they approve elsewhere).
    if args.wait_approvals && handle.status.as_deref() == Some("paused") {
        if let Some(run_id) = handle.run_id.as_deref() {
            return poll_until_terminal(ep, run_id);
        }
    }
    Ok(0)
}

fn render(h: &RunHandle) {
    match h.status.as_deref() {
        Some("paused") => {
            let reason = h.suspend_reason.as_deref().unwrap_or("human_approval");
            ui::warn(&format!("⏸ run paused — awaiting {reason}"));
            if let Some(pending) = &h.pending {
                for p in pending {
                    let rid = p.request_id.as_deref().unwrap_or("?");
                    let tool = p.tool.as_deref().unwrap_or("");
                    println!("  approval {rid}  (tool: {tool})");
                    println!("    approve:  forgeos approvals approve {rid}");
                    println!("    reject:   forgeos approvals reject {rid}");
                }
            }
            if let Some(run_id) = &h.run_id {
                println!("  run id: {run_id}   (poll: forgeos runs status {run_id})");
            }
        }
        Some("failed") => ui::err(h.error.as_deref().unwrap_or("run failed")),
        // completed, or legacy (status absent) — print the result text.
        _ => {
            if let Some(r) = &h.result {
                if !r.is_empty() {
                    println!("{r}");
                }
            }
        }
    }
    if h.simulated.unwrap_or(false) {
        ui::warn("Agent ran in SIMULATED mode — no real LLM call was made.");
    }
    if let Some(warnings) = &h.warnings {
        for w in warnings {
            ui::warn(w);
        }
    }
}

fn poll_until_terminal(ep: &Endpoint, run_id: &str) -> Result<i32> {
    let path = format!("/api/platform/runs/{run_id}");
    eprintln!("waiting for approval + resume of run {run_id} …");
    loop {
        thread::sleep(Duration::from_secs(2));
        let st: RunHandle = match api::get(ep, &path) {
            Ok(v) => v,
            Err(_) => continue, // transient; keep polling
        };
        match st.status.as_deref() {
            Some("completed") => {
                if let Some(r) = &st.result {
                    println!("{r}");
                }
                ui::ok("run completed");
                return Ok(0);
            }
            Some("failed") => {
                ui::err(st.error.as_deref().unwrap_or("run failed"));
                return Ok(1);
            }
            _ => continue, // running / paused — keep waiting
        }
    }
}
