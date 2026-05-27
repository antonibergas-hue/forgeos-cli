// SPDX-License-Identifier: BUSL-1.1
//! `forgeos answer <request_id> --text "..." | --value <value>` —
//! respond to a pending A2H freeform/choice/number question.

use anyhow::{bail, Result};
use clap::Args;
use serde::Serialize;
use serde_json::Value;

use crate::api::{self, Endpoint};
use crate::ui;

#[derive(Args)]
pub struct Cmd {
    /// The A2H request_id you're answering.
    pub request_id: String,

    /// Freeform text response (use for response_type=text).
    #[arg(long)]
    pub text: Option<String>,

    /// Structured value (use for response_type=choice/number).
    #[arg(long)]
    pub value: Option<String>,

    /// Who is responding (lands in the audit trail).
    #[arg(long, default_value = "cli")]
    pub responded_by: String,
}

#[derive(Serialize)]
struct Response<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<&'a str>,
}

#[derive(Serialize)]
struct Body<'a> {
    response: Response<'a>,
    responded_by: &'a str,
    channel: &'a str,
}

pub fn run(args: Cmd, ep: &Endpoint) -> Result<i32> {
    if args.text.is_none() && args.value.is_none() {
        bail!("provide --text or --value");
    }
    let body = Body {
        response: Response {
            text: args.text.as_deref(),
            value: args.value.as_deref(),
        },
        responded_by: &args.responded_by,
        channel: "cli",
    };
    let _: Value = api::post_json(
        ep,
        &format!("/api/a2h/requests/{}/respond", args.request_id),
        &body,
    )?;
    ui::ok(&format!("Responded to {}", args.request_id));
    Ok(0)
}
