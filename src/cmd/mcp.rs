// SPDX-License-Identifier: BUSL-1.1
//! `forgeos mcp <subcommand>` — register / list / remove platform-scoped MCP
//! servers. Backed by GET/POST/DELETE /api/platform/mcp/servers.

use anyhow::{bail, Result};
use clap::Subcommand;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::api::{self, Endpoint};
use crate::ui;

#[derive(Subcommand)]
pub enum McpCmd {
    /// Register an MCP server — platform-scoped, or per-user with --user.
    Register {
        /// Logical server name (tools surface as mcp__<name>__<tool>).
        server_name: String,
        /// Package / launch spec (e.g. "mcp-atlassian", "@scope/pkg").
        package: String,
        /// Environment variable, repeatable: --env KEY=VAL. Use a 'secret:<name>'
        /// value to resolve from the secret store at connect time.
        #[arg(long = "env", value_name = "KEY=VAL")]
        env: Vec<String>,
        /// Secret env var, repeatable: --secret KEY=VAL. The value is stored
        /// encrypted server-side and referenced from the MCP env. Requires --user.
        #[arg(long = "secret", value_name = "KEY=VAL")]
        secret: Vec<String>,
        /// Launch argument, repeatable: --arg VALUE.
        #[arg(long = "arg", value_name = "VALUE")]
        args: Vec<String>,
        /// Register for the active context's user (per-user MCP) instead of
        /// platform-wide. The connection uses that user's stored secrets.
        #[arg(long)]
        user: bool,
    },
    /// List registered MCP servers (secrets redacted).
    List {
        /// Emit raw JSON (for piping into jq).
        #[arg(long)]
        json: bool,
    },
    /// Remove a registered MCP server by name.
    Rm { server_name: String },
}

#[derive(Serialize)]
struct McpConfigRequest {
    server_name: String,
    package: String,
    env_vars: BTreeMap<String, String>,
    args: Vec<String>,
}

#[derive(Serialize)]
struct UserMcpRequest {
    package: String,
    env_vars: BTreeMap<String, String>,
    secrets: BTreeMap<String, String>,
    args: Vec<String>,
}

#[derive(Deserialize)]
struct McpServer {
    #[serde(default)]
    server_name: Option<String>,
    #[serde(default)]
    package: Option<String>,
    #[serde(default)]
    env_vars: Option<BTreeMap<String, String>>,
}

pub fn run(cmd: McpCmd, ep: &Endpoint) -> Result<i32> {
    match cmd {
        McpCmd::Register { server_name, package, env, secret, args, user } => {
            if user {
                register_user(ep, server_name, package, env, secret, args)
            } else {
                if !secret.is_empty() {
                    bail!("--secret requires --user (per-user encrypted secrets). For a \
                           platform server, pass a 'secret:<name>' value via --env instead.");
                }
                register(ep, server_name, package, env, args)
            }
        }
        McpCmd::List { json } => list(ep, json),
        McpCmd::Rm { server_name } => rm(ep, &server_name),
    }
}

fn parse_env(pairs: Vec<String>) -> Result<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    for p in pairs {
        match p.split_once('=') {
            Some((k, v)) if !k.is_empty() => {
                map.insert(k.to_string(), v.to_string());
            }
            _ => bail!("--env expects KEY=VAL, got {p:?}"),
        }
    }
    Ok(map)
}

fn register(
    ep: &Endpoint,
    server_name: String,
    package: String,
    env: Vec<String>,
    args: Vec<String>,
) -> Result<i32> {
    let body = McpConfigRequest {
        server_name: server_name.clone(),
        package,
        env_vars: parse_env(env)?,
        args,
    };
    let _: Value = api::post_json(ep, "/api/platform/mcp/servers", &body)?;
    ui::ok(&format!("Registered MCP server {server_name:?} (platform)"));
    Ok(0)
}

fn register_user(
    ep: &Endpoint,
    server_name: String,
    package: String,
    env: Vec<String>,
    secret: Vec<String>,
    args: Vec<String>,
) -> Result<i32> {
    // Identity comes from the active context's user.
    let user = match api::describe_target(ep) {
        Ok(r) => r.user,
        Err(e) => return Err(e),
    };
    let user = match user {
        Some(u) if !u.is_empty() => u,
        _ => bail!(
            "no user on the active context. Set one with \
             `forgeos config set-context <name> --user <id>` (or --user-id on a per-call basis)."
        ),
    };
    let body = UserMcpRequest {
        package,
        env_vars: parse_env(env)?,
        secrets: parse_env(secret)?,
        args,
    };
    let path = format!("/api/users/{user}/mcp/{server_name}");
    let _: Value = api::post_json(ep, &path, &body)?;
    let n = body.secrets.len();
    ui::ok(&format!(
        "Registered MCP server {server_name:?} for user {user:?} ({n} encrypted secret(s))"
    ));
    Ok(0)
}

fn list(ep: &Endpoint, json: bool) -> Result<i32> {
    let raw: Value = api::get(ep, "/api/platform/mcp/servers")?;
    if json {
        println!("{}", serde_json::to_string_pretty(&raw).unwrap_or_default());
        return Ok(0);
    }
    let servers: Vec<McpServer> = serde_json::from_value(raw).unwrap_or_default();
    if servers.is_empty() {
        ui::warn("No MCP servers registered");
        return Ok(0);
    }
    println!(
        "{:<18}  {:<26}  {}",
        "SERVER".dimmed(), "PACKAGE".dimmed(), "ENV (redacted)".dimmed(),
    );
    for s in servers {
        let name = s.server_name.as_deref().unwrap_or("?");
        let pkg = s.package.as_deref().unwrap_or("?");
        let keys = s
            .env_vars
            .as_ref()
            .map(|m| m.keys().map(|k| format!("{k}=***")).collect::<Vec<_>>().join(","))
            .unwrap_or_default();
        println!("{name:<18}  {pkg:<26}  {keys}");
    }
    Ok(0)
}

fn rm(ep: &Endpoint, server_name: &str) -> Result<i32> {
    let _: Value = api::delete(ep, &format!("/api/platform/mcp/servers/{server_name}"))?;
    ui::ok(&format!("Removed MCP server {server_name:?}"));
    Ok(0)
}
