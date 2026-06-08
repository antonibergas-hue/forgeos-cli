// SPDX-License-Identifier: BUSL-1.1
//! `forgeos credentials put github --pat <PAT>` — store a per-user GitHub
//! PAT in the platform's Secret Manager. Write-only by design: there is
//! no `get` verb, secrets only flow into running agent processes.

use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;
use serde_json::Value;

use crate::api::{self, Endpoint};
use crate::ui;

#[derive(Subcommand)]
pub enum CredentialsCmd {
    /// Store a credential. Write-only — there is no `get`.
    #[command(subcommand)]
    Put(PutKind),
}

#[derive(Subcommand)]
pub enum PutKind {
    /// Store a GitHub personal access token for the named user.
    Github {
        /// Personal access token (`repo`, `workflow` scopes minimum).
        #[arg(long)]
        pat: String,
        /// User identifier the secret is scoped to.
        #[arg(long, default_value = "default")]
        user_id: String,
    },
    /// Store an Atlassian/JIRA Cloud credential (url + email + API token).
    Jira {
        /// JIRA base URL, e.g. https://your-org.atlassian.net.
        #[arg(long)]
        url: String,
        /// Atlassian account email.
        #[arg(long)]
        email: String,
        /// Atlassian Cloud API token.
        #[arg(long)]
        token: String,
        /// User the secret is scoped to (defaults to the active context user).
        #[arg(long, default_value = "default")]
        user_id: String,
    },
}

#[derive(Serialize)]
struct PutGithub<'a> {
    pat: &'a str,
    user_id: &'a str,
}

#[derive(Serialize)]
struct PutJira<'a> {
    url: &'a str,
    email: &'a str,
    token: &'a str,
    user_id: &'a str,
}

pub fn run(cmd: CredentialsCmd, ep: &Endpoint) -> Result<i32> {
    match cmd {
        CredentialsCmd::Put(PutKind::Github { pat, user_id }) => {
            let body = PutGithub { pat: &pat, user_id: &user_id };
            let _: Value = api::post_json(ep, "/api/credentials/github", &body)?;
            ui::ok(&format!("Stored github credential for user_id={user_id}"));
            Ok(0)
        }
        CredentialsCmd::Put(PutKind::Jira { url, email, token, user_id }) => {
            let body = PutJira { url: &url, email: &email, token: &token, user_id: &user_id };
            let _: Value = api::post_json(ep, "/api/credentials/jira", &body)?;
            ui::ok(&format!("Stored jira credential for user_id={user_id}"));
            Ok(0)
        }
    }
}
