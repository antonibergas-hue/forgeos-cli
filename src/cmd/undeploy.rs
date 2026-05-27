// SPDX-License-Identifier: BUSL-1.1
use anyhow::{bail, Result};
use clap::Args as ClapArgs;
use serde::Deserialize;

use crate::api::{self, Endpoint};
use crate::ui;

#[derive(ClapArgs)]
pub struct Args {
    pub agent_id: String,
}

#[derive(Deserialize)]
struct UndeployResponse {
    #[serde(default)]
    removed: bool,
}

pub fn run(args: Args, ep: &Endpoint) -> Result<i32> {
    let resp: UndeployResponse =
        api::delete(ep, &format!("/api/platform/agents/{}", args.agent_id))?;
    if !resp.removed {
        bail!("agent '{}' not found", args.agent_id);
    }
    ui::ok(&format!("Undeployed {}", args.agent_id));
    Ok(0)
}
