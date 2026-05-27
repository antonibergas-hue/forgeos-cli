// SPDX-License-Identifier: BUSL-1.1
//! HTTP client and endpoint discovery.

use anyhow::{anyhow, bail, Result};
use reqwest::blocking::{Client, Response, RequestBuilder};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

use crate::config::{self, AuthScheme};

pub struct Endpoint {
    /// Lazily resolved on first use so subcommands that don't need the
    /// server (like `validate`) can run without a lockfile.
    remote_override: Option<String>,
    token_override: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub base: String,
    pub token: String,
    pub auth: AuthScheme,
    /// What we resolved against (for error messages).
    pub source: ResolvedSource,
}

#[derive(Debug, Clone)]
pub enum ResolvedSource {
    Flags,
    Context(String),
    Lockfile,
}

impl Endpoint {
    pub fn resolve(remote: Option<String>, token: Option<String>) -> Self {
        Self {
            remote_override: remote.map(normalize_remote),
            token_override: token,
        }
    }

    fn resolved(&self) -> Result<Resolved> {
        // Resolution order:
        //   1. Explicit --remote / --token flags
        //   2. Current context in ~/.forgeos/config.yaml
        //   3. ~/.forgeos/server.lock (auto-written by local forgeos-server)
        //   4. Error
        let ctx = config::current_context().ok().flatten();
        let lock = config::read_server_lock().ok();

        let (base, source_for_url) = match (&self.remote_override, &ctx, &lock) {
            (Some(url), _, _) => (url.trim_end_matches('/').to_string(), ResolvedSource::Flags),
            (None, Some((name, c)), _) => (
                c.server.trim_end_matches('/').to_string(),
                ResolvedSource::Context(name.clone()),
            ),
            (None, None, Some(l)) => (
                format!("http://{}:{}", l.host, l.port),
                ResolvedSource::Lockfile,
            ),
            (None, None, None) => bail!(
                "no endpoint configured.\n\
                 Run one of:\n  \
                   forgeos config set-context <name> --server <URL> --token <T> && forgeos config use-context <name>\n  \
                   make server    # for a local forgeos-server\n  \
                 Or pass --remote <URL> --token <T> per-call."
            ),
        };

        let (token, auth) = match (&self.token_override, &ctx, &lock) {
            (Some(t), _, _) => (t.clone(), AuthScheme::Bearer),
            (None, Some((_, c)), _) => match &c.token {
                Some(t) => (t.clone(), c.auth_scheme.clone()),
                None => bail!(
                    "context has no token. Set one with `forgeos config set-context <name> --token <T>`."
                ),
            },
            (None, None, Some(l)) => (l.token.clone(), AuthScheme::Bearer),
            (None, None, None) => unreachable!(),
        };

        Ok(Resolved {
            base,
            token,
            auth,
            source: source_for_url,
        })
    }
}

/// Attach the auth header appropriate for the given scheme.
fn auth_header(req: RequestBuilder, token: &str, auth: &AuthScheme) -> RequestBuilder {
    match auth {
        AuthScheme::Bearer => req.bearer_auth(token),
        AuthScheme::XApiKey => req.header("X-API-Key", token),
    }
}

/// Prepend `http://` when the user passed a bare `host:port` (or just
/// `host`). `reqwest` is strict about absolute URLs and otherwise yields
/// the cryptic ``relative URL without a base``.
fn normalize_remote(s: String) -> String {
    if s.starts_with("http://") || s.starts_with("https://") {
        s
    } else {
        format!("http://{s}")
    }
}

fn client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("build reqwest client")
}

fn check(resp: Response, method: &str, path: &str) -> Result<Response> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let body = resp.text().unwrap_or_default();
    let detail: Option<String> = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v.get("detail").and_then(|d| d.as_str().map(String::from)));
    Err(anyhow!(
        "{} {} failed ({}): {}",
        method,
        path,
        status,
        detail.unwrap_or(body)
    ))
}

/// Translate a reqwest send-error into something actionable.
///
/// "Connection refused" against a base URL whose host/port we got from
/// the lockfile almost always means: the server crashed (kill -9 /
/// reboot) and the lockfile is stale. Point the user at the cleanup
/// path instead of leaving them with a raw socket error.
fn enrich_send(err: reqwest::Error, base: &str, method: &str, path: &str) -> anyhow::Error {
    if err.is_connect() && config::lock_path().exists() {
        return anyhow!(
            "{method} {base}{path} — connection refused.\n\
             A server lockfile exists at {} but the daemon is not listening.\n\
             Likely cause: a previous forgeos-server crashed or was killed.\n\
             Start a fresh one with `make server`, or remove the stale lockfile:\n\
             rm {}",
            config::lock_path().display(),
            config::lock_path().display()
        );
    }
    anyhow!("{method} {base}{path}: {err}")
}

pub fn get<T: DeserializeOwned>(ep: &Endpoint, path: &str) -> Result<T> {
    let r = ep.resolved()?;
    let url = format!("{base}{path}", base = r.base);
    let resp = auth_header(client().get(&url), &r.token, &r.auth)
        .send()
        .map_err(|e| enrich_send(e, &r.base, "GET", path))?;
    let resp = check(resp, "GET", path)?;
    resp.json::<T>()
        .map_err(|e| anyhow!("decode GET {url}: {e}"))
}

pub fn post_json<B: Serialize, T: DeserializeOwned>(
    ep: &Endpoint,
    path: &str,
    body: &B,
) -> Result<T> {
    let r = ep.resolved()?;
    let url = format!("{base}{path}", base = r.base);
    let resp = auth_header(client().post(&url), &r.token, &r.auth)
        .json(body)
        .send()
        .map_err(|e| enrich_send(e, &r.base, "POST", path))?;
    let resp = check(resp, "POST", path)?;
    resp.json::<T>()
        .map_err(|e| anyhow!("decode POST {url}: {e}"))
}

/// POST raw YAML body (Content-Type: text/yaml). Used by `deploy` so the
/// server's AgentManifest.from_dict handles parsing — including resolving
/// system_prompt file references that the Rust client pre-inlined.
pub fn post_yaml<T: DeserializeOwned>(ep: &Endpoint, path: &str, yaml: &str) -> Result<T> {
    let r = ep.resolved()?;
    let url = format!("{base}{path}", base = r.base);
    let resp = auth_header(client().post(&url), &r.token, &r.auth)
        .header("Content-Type", "text/yaml")
        .body(yaml.to_string())
        .send()
        .map_err(|e| enrich_send(e, &r.base, "POST", path))?;
    let resp = check(resp, "POST", path)?;
    resp.json::<T>()
        .map_err(|e| anyhow!("decode POST {url}: {e}"))
}

pub fn delete<T: DeserializeOwned>(ep: &Endpoint, path: &str) -> Result<T> {
    let r = ep.resolved()?;
    let url = format!("{base}{path}", base = r.base);
    let resp = auth_header(client().delete(&url), &r.token, &r.auth)
        .send()
        .map_err(|e| enrich_send(e, &r.base, "DELETE", path))?;
    let resp = check(resp, "DELETE", path)?;
    resp.json::<T>()
        .map_err(|e| anyhow!("decode DELETE {url}: {e}"))
}

/// `/api/health` is unauthenticated.
pub fn health(ep: &Endpoint) -> Result<Value> {
    let r = ep.resolved()?;
    let url = format!("{base}/api/health", base = r.base);
    let resp = client()
        .get(&url)
        .send()
        .map_err(|e| enrich_send(e, &r.base, "GET", "/api/health"))?;
    let resp = check(resp, "GET", "/api/health")?;
    resp.json::<Value>()
        .map_err(|e| anyhow!("decode GET {url}: {e}"))
}

/// Public view of the current endpoint resolution — for `config` commands
/// that want to show what the CLI would target.
pub fn describe_target(ep: &Endpoint) -> Result<Resolved> {
    ep.resolved()
}
