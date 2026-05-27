// SPDX-License-Identifier: BUSL-1.1
use anyhow::Result;
use clap::Args as ClapArgs;
use serde_json::Value;

use crate::api::{self, Endpoint};
use crate::ui;

#[derive(ClapArgs)]
pub struct Args {
    /// Emit raw JSON instead of the table (for piping into jq).
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: Args, ep: &Endpoint) -> Result<i32> {
    let agents: Vec<Value> = api::get(ep, "/api/platform/agents")?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&agents).unwrap_or_default());
        return Ok(0);
    }
    if agents.is_empty() {
        ui::warn("No agents deployed");
        return Ok(0);
    }
    println!(
        "{:<14}  {:<30}  {:<10}  {:<14}  {:<12}",
        "AGENT_ID", "NAME", "STACK", "TYPE", "STATUS"
    );
    println!(
        "{}  {}  {}  {}  {}",
        "-".repeat(14),
        "-".repeat(30),
        "-".repeat(10),
        "-".repeat(14),
        "-".repeat(12)
    );
    for a in agents {
        let id = a.get("agent_id").and_then(|v| v.as_str()).unwrap_or("?");
        let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let stack = a.get("stack").and_then(|v| v.as_str()).unwrap_or("?");
        let exec = a
            .get("execution_type")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let status = a.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        println!("{id:<14}  {name:<30}  {stack:<10}  {exec:<14}  {status:<12}");
    }
    Ok(0)
}
