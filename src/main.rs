// Copyright 2024-2026 Awake Venture Studio (awakeventurestudio.co),
// a Making Science Group, SA company.
// SPDX-License-Identifier: BUSL-1.1
//
// ForgeOS CLI — single static binary client for the Python platform.
//
// All operations go over HTTP against a forgeos-server daemon. The server's
// host/port/token are discovered from ~/.forgeos/server.lock unless
// overridden by --remote / --token or the FORGEOS_REMOTE / FORGEOS_TOKEN
// env vars.

mod api;
mod cmd;
mod config;
mod manifest;
mod ui;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "forgeos",
    version,
    about = "ForgeOS CLI — declare and manage agents from the command line",
    long_about = None
)]
struct Cli {
    /// Server base URL. Default: read from ~/.forgeos/server.lock.
    #[arg(long, global = true, env = "FORGEOS_REMOTE")]
    remote: Option<String>,

    /// Bearer token. Default: read from ~/.forgeos/server.lock.
    #[arg(long, global = true, env = "FORGEOS_TOKEN")]
    token: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate an agent manifest without deploying. No server required.
    Validate(cmd::validate::Args),
    /// Deploy an agent from a manifest file.
    Deploy(cmd::deploy::Args),
    /// List deployed agents.
    List(cmd::list::Args),
    /// Show full manifest + live status for one agent.
    Describe(cmd::describe::Args),
    /// Invoke an agent with a prompt.
    Invoke(cmd::invoke::Args),
    /// Stream per-agent activity: run start/end + every tool call.
    Logs(cmd::logs::Cmd),
    /// Undeploy an agent.
    Undeploy(cmd::undeploy::Args),
    /// Undeploy all agents in a team (server endpoint required; not yet implemented).
    UndeployTeam(cmd::stub::TeamArgs),
    /// Platform health check.
    Health,
    /// Manage local config and credentials in ~/.forgeos/.
    #[command(subcommand)]
    Config(cmd::config::ConfigCmd),
    /// List / approve / reject pending human-in-the-loop requests.
    #[command(subcommand)]
    Approvals(cmd::approvals::ApprovalsCmd),
    /// Answer a pending A2H text/choice/number question.
    Answer(cmd::answer::Cmd),
    /// Manage per-user credentials (write-only).
    #[command(subcommand)]
    Credentials(cmd::credentials::CredentialsCmd),
}

fn main() {
    let cli = Cli::parse();
    let ctx = cli.token_and_remote();
    let result = match cli.command {
        Command::Validate(args) => cmd::validate::run(args),
        Command::Deploy(args) => cmd::deploy::run(args, &ctx),
        Command::List(args) => cmd::list::run(args, &ctx),
        Command::Describe(args) => cmd::describe::run(args, &ctx),
        Command::Invoke(args) => cmd::invoke::run(args, &ctx),
        Command::Logs(args) => cmd::logs::run(args, &ctx),
        Command::Undeploy(args) => cmd::undeploy::run(args, &ctx),
        Command::UndeployTeam(args) => cmd::stub::undeploy_team(args),
        Command::Health => cmd::health::run(&ctx),
        Command::Config(sub) => cmd::config::run(sub),
        Command::Approvals(sub) => cmd::approvals::run(sub, &ctx),
        Command::Answer(args) => cmd::answer::run(args, &ctx),
        Command::Credentials(sub) => cmd::credentials::run(sub, &ctx),
    };
    match result {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            ui::err(&format!("{e:#}"));
            std::process::exit(1);
        }
    }
}

impl Cli {
    fn token_and_remote(&self) -> api::Endpoint {
        api::Endpoint::resolve(self.remote.clone(), self.token.clone())
    }
}
