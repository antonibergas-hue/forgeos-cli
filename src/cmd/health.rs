// SPDX-License-Identifier: BUSL-1.1
use anyhow::Result;

use crate::api::{self, Endpoint};

pub fn run(ep: &Endpoint) -> Result<i32> {
    let payload = api::health(ep)?;
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(0)
}
