// SPDX-License-Identifier: BUSL-1.1
//! `forgeos stop <agent_id>` — pause a deployed agent without removing it.
//!
//! Hits POST /api/platform/agents/{id}/stop, which (server-side):
//!   - removes any scheduled cron job for this agent (the scheduler stops firing)
//!   - unsubscribes from the event bus
//!   - transitions the process to STOPPED and drops its checkpoint
//!   - keeps the agent in the registry as STATUS=stopped
//!
//! To bring it back, just `forgeos deploy` the manifest again — the scheduler
//! will re-register the cron. To remove entirely, use `forgeos undeploy`.

use anyhow::Result;
use clap::Args as ClapArgs;
use serde::Deserialize;
use serde_json::Value;

use crate::api::{self, Endpoint};
use crate::ui;

#[derive(ClapArgs)]
pub struct Args {
    /// Agent id (from `forgeos list`).
    pub agent_id: String,
}

#[derive(Deserialize)]
struct StopResponse {
    #[serde(default)]
    ok: bool,
}

pub fn run(args: Args, ep: &Endpoint) -> Result<i32> {
    let path = format!("/api/platform/agents/{}/stop", args.agent_id);
    // The server's /stop endpoint ignores the body, but post_json wants one.
    let body = Value::Object(Default::default());
    let resp: StopResponse = api::post_json(ep, &path, &body)?;
    if !resp.ok {
        anyhow::bail!("server returned ok=false for {}", args.agent_id);
    }
    ui::ok(&format!(
        "Stopped {} (scheduler off, kept in registry; `forgeos deploy` to re-enable)",
        args.agent_id
    ));
    Ok(0)
}
