// SPDX-License-Identifier: BUSL-1.1
//! `forgeos config <subcommand>` — manage ~/.forgeos/.

use anyhow::Result;
use clap::Subcommand;

use crate::config;
use crate::ui;

#[derive(Subcommand)]
pub enum ConfigCmd {
    /// Print config + credentials (values redacted).
    View,
    /// Show the active profile.
    CurrentProfile,
    /// Switch the active profile.
    UseProfile {
        name: String,
    },
    /// Store a credential under the current (or specified) profile.
    SetCredential {
        name: String,
        value: String,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Print a stored credential.
    GetCredential {
        name: String,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Delete a stored credential.
    DeleteCredential {
        name: String,
        #[arg(long)]
        profile: Option<String>,
    },
    /// List credential names in a profile.
    ListCredentials {
        #[arg(long)]
        profile: Option<String>,
    },
    /// Define or update a remote context (server URL + auth token).
    SetContext {
        /// Context name (e.g. "prod", "staging", "local").
        name: String,
        #[arg(long)]
        server: String,
        #[arg(long)]
        token: Option<String>,
        /// Auth scheme expected by the server.
        #[arg(long, default_value = "bearer", value_parser = ["bearer", "x-api-key"])]
        auth: String,
    },
    /// Switch the CLI to a defined context.
    UseContext {
        name: String,
    },
    /// Show the name of the active context.
    CurrentContext,
    /// List all defined contexts.
    GetContexts,
    /// Delete a context by name.
    DeleteContext {
        name: String,
    },
    /// Show the URL+auth the CLI would target on the next call.
    Target,
}

pub fn run(cmd: ConfigCmd) -> Result<i32> {
    match cmd {
        ConfigCmd::View => view(),
        ConfigCmd::CurrentProfile => {
            println!("{}", config::current_profile()?);
            Ok(0)
        }
        ConfigCmd::UseProfile { name } => {
            config::set_current_profile(&name)?;
            ui::ok(&format!("Active profile is now {name:?}"));
            Ok(0)
        }
        ConfigCmd::SetCredential {
            name,
            value,
            profile,
        } => {
            config::set_credential(&name, &value, profile.as_deref())?;
            let p = match profile {
                Some(p) => p,
                None => config::current_profile()?,
            };
            ui::ok(&format!("Stored credential {name:?} in profile {p:?}"));
            Ok(0)
        }
        ConfigCmd::GetCredential { name, profile } => match config::get_credential(&name, profile.as_deref())? {
            Some(v) => {
                println!("{v}");
                Ok(0)
            }
            None => {
                ui::err(&format!("Credential {name:?} not found"));
                Ok(1)
            }
        },
        ConfigCmd::DeleteCredential { name, profile } => {
            let removed = config::delete_credential(&name, profile.as_deref())?;
            if removed {
                let p = match profile {
                    Some(p) => p,
                    None => config::current_profile()?,
                };
                ui::ok(&format!("Deleted credential {name:?} from profile {p:?}"));
                Ok(0)
            } else {
                ui::err(&format!("Credential {name:?} not found"));
                Ok(1)
            }
        }
        ConfigCmd::ListCredentials { profile } => {
            let names = config::list_credentials(profile.as_deref())?;
            let p = match &profile {
                Some(p) => p.clone(),
                None => config::current_profile()?,
            };
            if names.is_empty() {
                println!("# no credentials in profile {p:?}");
            } else {
                println!("# profile: {p}");
                for n in names {
                    println!("{n}");
                }
            }
            Ok(0)
        }
        ConfigCmd::SetContext {
            name,
            server,
            token,
            auth,
        } => {
            let auth_scheme: config::AuthScheme = auth.parse()?;
            let server_norm = normalize_server(&server);
            config::set_context(
                &name,
                config::Context {
                    server: server_norm.clone(),
                    token,
                    auth_scheme: auth_scheme.clone(),
                },
            )?;
            ui::ok(&format!(
                "Stored context {name:?} -> {server_norm} (auth={auth_scheme})"
            ));
            // Auto-activate when this is the first context defined.
            let all = config::list_contexts()?;
            if all.len() == 1 || config::current_context_name()?.is_none() {
                config::use_context(&name)?;
                ui::ok(&format!("Active context is now {name:?}"));
            } else {
                let cur = config::current_context_name()?
                    .unwrap_or_else(|| "<none>".to_string());
                println!("  (active context is still {cur:?}; run `forgeos config use-context {name}` to switch)");
            }
            Ok(0)
        }
        ConfigCmd::UseContext { name } => {
            config::use_context(&name)?;
            ui::ok(&format!("Active context is now {name:?}"));
            Ok(0)
        }
        ConfigCmd::CurrentContext => {
            match config::current_context_name()? {
                Some(n) => println!("{n}"),
                None => {
                    ui::err("no context selected. Run `forgeos config get-contexts` to list available ones, then `forgeos config use-context <name>`.");
                    return Ok(1);
                }
            }
            Ok(0)
        }
        ConfigCmd::GetContexts => {
            let all = config::list_contexts()?;
            let cur = config::current_context_name()?;
            if all.is_empty() {
                println!("# no contexts defined");
                return Ok(0);
            }
            println!(
                "{:<6}  {:<14}  {:<11}  {}",
                "CUR", "NAME", "AUTH", "SERVER"
            );
            println!(
                "{}  {}  {}  {}",
                "-".repeat(6),
                "-".repeat(14),
                "-".repeat(11),
                "-".repeat(40)
            );
            for (name, ctx) in all {
                let marker = if Some(&name) == cur.as_ref() { "*" } else { "" };
                println!(
                    "{marker:<6}  {name:<14}  {auth:<11}  {server}",
                    auth = ctx.auth_scheme,
                    server = ctx.server,
                );
            }
            Ok(0)
        }
        ConfigCmd::DeleteContext { name } => {
            if config::delete_context(&name)? {
                ui::ok(&format!("Deleted context {name:?}"));
                Ok(0)
            } else {
                ui::err(&format!("Context {name:?} not found"));
                Ok(1)
            }
        }
        ConfigCmd::Target => {
            // Build an Endpoint with no overrides so the resolution
            // matches what every other subcommand would see.
            let ep = crate::api::Endpoint::resolve(None, None);
            match crate::api::describe_target(&ep) {
                Ok(r) => {
                    let source = match &r.source {
                        crate::api::ResolvedSource::Flags => "flags".to_string(),
                        crate::api::ResolvedSource::Context(n) => format!("context {n:?}"),
                        crate::api::ResolvedSource::Lockfile => "server.lock".to_string(),
                    };
                    println!("server: {}", r.base);
                    println!("auth:   {}", r.auth);
                    println!("source: {source}");
                    Ok(0)
                }
                Err(e) => {
                    ui::err(&format!("{e:#}"));
                    Ok(1)
                }
            }
        }
    }
}

fn normalize_server(s: &str) -> String {
    let s = s.trim().trim_end_matches('/');
    if s.starts_with("http://") || s.starts_with("https://") {
        s.to_string()
    } else {
        format!("https://{s}")
    }
}

fn view() -> Result<i32> {
    let mut cfg = config::load_config()?;
    // Redact context tokens so `forgeos config view` is safe to paste.
    if let Some(serde_yaml::Value::Mapping(ctxs)) = cfg.get_mut("contexts") {
        for (_, v) in ctxs.iter_mut() {
            if let serde_yaml::Value::Mapping(m) = v
                && let Some(t) = m.get_mut(serde_yaml::Value::String("token".into()))
                && !t.is_null()
            {
                *t = serde_yaml::Value::String("***".into());
            }
        }
    }

    let creds = config::load_credentials()?;
    let redacted: std::collections::BTreeMap<&String, std::collections::BTreeMap<&String, &str>> =
        creds
            .iter()
            .map(|(p, bucket)| (p, bucket.iter().map(|(k, _)| (k, "***")).collect()))
            .collect();
    println!("# {} (tokens redacted)", config::config_path().display());
    println!("{}", serde_yaml::to_string(&cfg)?);
    println!("# {} (values redacted)", config::credentials_path().display());
    println!("{}", serde_yaml::to_string(&redacted)?);
    Ok(0)
}
