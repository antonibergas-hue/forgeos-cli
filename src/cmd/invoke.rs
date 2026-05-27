// SPDX-License-Identifier: BUSL-1.1
use anyhow::Result;
use clap::Args as ClapArgs;
use serde::Serialize;
use serde_json::Value;

use crate::api::{self, Endpoint};
use crate::ui;

#[derive(ClapArgs)]
pub struct Args {
    pub agent_id: String,
    pub prompt: String,
    /// Wait for the run to finish and print the full result (blocking).
    /// Default is fire-and-return: queue the run and exit immediately.
    #[arg(short, long)]
    pub wait: bool,
}

#[derive(Serialize)]
struct InvokeRequest<'a> {
    prompt: &'a str,
    context: Value,
}

pub fn run(args: Args, ep: &Endpoint) -> Result<i32> {
    // Default: fire-and-return via the server's async_mode (returns immediately
    // with {status:"accepted"}). Use --wait to block on the full result.
    if !args.wait {
        let path = format!(
            "/api/platform/agents/{}/invoke?async_mode=true",
            args.agent_id
        );
        let _result: Value = api::post_json(
            ep,
            &path,
            &InvokeRequest {
                prompt: &args.prompt,
                context: Value::Object(Default::default()),
            },
        )?;
        ui::ok(&format!("Invoked {} — run queued.", args.agent_id));
        println!(
            "  Watch it:  forgeos logs {} --follow",
            args.agent_id
        );
        return Ok(0);
    }

    let path = format!("/api/platform/agents/{}/invoke", args.agent_id);
    let result: Value = api::post_json(
        ep,
        &path,
        &InvokeRequest {
            prompt: &args.prompt,
            context: Value::Object(Default::default()),
        },
    )?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    if result
        .get("simulated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        eprintln!();
        ui::warn("Agent ran in SIMULATED mode — no real LLM call was made.");
    }
    if let Some(warnings) = result.get("warnings").and_then(|v| v.as_array()) {
        for w in warnings {
            if let Some(s) = w.as_str() {
                ui::warn(s);
            }
        }
    }
    Ok(0)
}
