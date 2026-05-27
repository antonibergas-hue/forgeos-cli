// SPDX-License-Identifier: BUSL-1.1
//! Subcommands deferred until the corresponding server endpoint lands or
//! a Rust TUI is built (e.g. `mc fleet`). Each returns a clear error so
//! users aren't left wondering.

use anyhow::{bail, Result};
use clap::Args as ClapArgs;

#[derive(ClapArgs)]
pub struct TeamArgs {
    pub name: String,
    #[arg(long, default_value = "default")]
    pub namespace: String,
}

pub fn undeploy_team(_args: TeamArgs) -> Result<i32> {
    bail!(
        "`undeploy-team` is not yet implemented in the Rust CLI. \
         The server endpoint exists; the Rust handler is a TODO."
    )
}
