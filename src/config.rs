// SPDX-License-Identifier: BUSL-1.1
//! ~/.forgeos/ config + credentials store (kubectl-style).
//!
//! Mirrors the Python `src/forgeos_sdk/config_store.py` so the Rust CLI
//! and any remaining Python tooling share the same files. Credentials
//! live in `~/.forgeos/credentials` at mode 0600; refusing to read the
//! file if it's group/other-readable matches the Python behaviour.

// `anyhow::Context` is imported anonymously (the `as _`) so its
// `.context()` method stays usable on Results while our own
// `pub struct Context` further down can keep the name.
use anyhow::Context as _;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub fn config_dir() -> PathBuf {
    if let Ok(override_dir) = std::env::var("FORGEOS_CONFIG_DIR") {
        return PathBuf::from(override_dir);
    }
    dirs::home_dir().expect("no home directory").join(".forgeos")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.yaml")
}

pub fn credentials_path() -> PathBuf {
    config_dir().join("credentials")
}

pub fn lock_path() -> PathBuf {
    config_dir().join("server.lock")
}

fn ensure_dir() -> Result<()> {
    let d = config_dir();
    fs::create_dir_all(&d).with_context(|| format!("create {}", d.display()))?;
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(&d, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

// ---- config.yaml ----------------------------------------------------------

pub type Config = BTreeMap<String, serde_yaml::Value>;

pub fn load_config() -> Result<Config> {
    let path = config_path();
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    let parsed: Config = serde_yaml::from_str(&raw)
        .with_context(|| format!("parse YAML in {}", path.display()))?;
    Ok(parsed)
}

pub fn save_config(data: &Config) -> Result<()> {
    ensure_dir()?;
    let yaml = serde_yaml::to_string(data)?;
    let path = config_path();
    fs::write(&path, yaml).with_context(|| format!("write {}", path.display()))?;
    // Contexts hold bearer tokens inline — keep config.yaml as tight as
    // the credentials file. If you ever split tokens into a separate
    // file, relax this back to 0644.
    #[cfg(unix)]
    {
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn current_profile() -> Result<String> {
    let cfg = load_config()?;
    match cfg.get("current_profile") {
        Some(serde_yaml::Value::String(s)) => Ok(s.clone()),
        _ => Ok("default".to_string()),
    }
}

pub fn set_current_profile(name: &str) -> Result<()> {
    let mut cfg = load_config()?;
    cfg.insert(
        "current_profile".to_string(),
        serde_yaml::Value::String(name.to_string()),
    );
    save_config(&cfg)
}

// ---- credentials ----------------------------------------------------------

pub type Credentials = BTreeMap<String, BTreeMap<String, String>>;

fn check_credentials_permissions() -> Result<()> {
    let path = credentials_path();
    if !path.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        let mode = fs::metadata(&path)?.permissions().mode() & 0o777;
        let bad = mode & 0o077;
        if bad != 0 {
            bail!(
                "{} has insecure permissions {:#o}. Run: chmod 600 {}",
                path.display(),
                mode,
                path.display()
            );
        }
    }
    Ok(())
}

pub fn load_credentials() -> Result<Credentials> {
    check_credentials_permissions()?;
    let path = credentials_path();
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    let parsed: Credentials = serde_yaml::from_str(&raw)
        .with_context(|| format!("parse YAML in {}", path.display()))?;
    Ok(parsed)
}

pub fn save_credentials(data: &Credentials) -> Result<()> {
    ensure_dir()?;
    let yaml = serde_yaml::to_string(data)?;
    let path = credentials_path();
    fs::write(&path, yaml).with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn get_credential(name: &str, profile: Option<&str>) -> Result<Option<String>> {
    let profile_name = match profile {
        Some(p) => p.to_string(),
        None => current_profile()?,
    };
    let creds = load_credentials()?;
    Ok(creds
        .get(&profile_name)
        .and_then(|bucket| bucket.get(name))
        .cloned())
}

pub fn set_credential(name: &str, value: &str, profile: Option<&str>) -> Result<()> {
    let profile_name = match profile {
        Some(p) => p.to_string(),
        None => current_profile()?,
    };
    let mut creds = load_credentials()?;
    creds
        .entry(profile_name)
        .or_default()
        .insert(name.to_string(), value.to_string());
    save_credentials(&creds)
}

pub fn delete_credential(name: &str, profile: Option<&str>) -> Result<bool> {
    let profile_name = match profile {
        Some(p) => p.to_string(),
        None => current_profile()?,
    };
    let mut creds = load_credentials()?;
    let removed = creds
        .get_mut(&profile_name)
        .and_then(|bucket| bucket.remove(name))
        .is_some();
    if removed {
        save_credentials(&creds)?;
    }
    Ok(removed)
}

pub fn list_credentials(profile: Option<&str>) -> Result<Vec<String>> {
    let profile_name = match profile {
        Some(p) => p.to_string(),
        None => current_profile()?,
    };
    let creds = load_credentials()?;
    Ok(match creds.get(&profile_name) {
        Some(bucket) => {
            let mut names: Vec<_> = bucket.keys().cloned().collect();
            names.sort();
            names
        }
        None => Vec::new(),
    })
}

// ---- server.lock ----------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerLock {
    pub host: String,
    pub port: u16,
    pub token: String,
    #[serde(default)]
    pub pid: Option<u32>,
}

pub fn read_server_lock() -> Result<ServerLock> {
    let path = lock_path();
    let raw = fs::read_to_string(&path).with_context(|| {
        format!(
            "no server lock at {} — is `forgeos-server` running?",
            path.display()
        )
    })?;
    let lock: ServerLock = serde_json::from_str(&raw)
        .map_err(|e| anyhow!("malformed {}: {}", path.display(), e))?;
    Ok(lock)
}

// ---- contexts (kubectl-style) --------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum AuthScheme {
    #[serde(rename = "bearer")]
    Bearer,
    #[serde(rename = "x-api-key")]
    XApiKey,
}

impl Default for AuthScheme {
    fn default() -> Self {
        AuthScheme::Bearer
    }
}

impl std::fmt::Display for AuthScheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthScheme::Bearer => write!(f, "bearer"),
            AuthScheme::XApiKey => write!(f, "x-api-key"),
        }
    }
}

impl std::str::FromStr for AuthScheme {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "bearer" => Ok(AuthScheme::Bearer),
            "x-api-key" | "xapikey" | "apikey" | "api-key" => Ok(AuthScheme::XApiKey),
            other => bail!("unknown auth scheme: {other:?} (expected: bearer | x-api-key)"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Context {
    pub server: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default, rename = "auth")]
    pub auth_scheme: AuthScheme,
}

const CTX_KEY: &str = "contexts";
const CURRENT_CTX_KEY: &str = "current_context";

pub fn list_contexts() -> Result<BTreeMap<String, Context>> {
    let cfg = load_config()?;
    match cfg.get(CTX_KEY) {
        Some(v) => serde_yaml::from_value(v.clone())
            .with_context(|| format!("parse `{CTX_KEY}` block in config.yaml")),
        None => Ok(BTreeMap::new()),
    }
}

pub fn current_context_name() -> Result<Option<String>> {
    let cfg = load_config()?;
    Ok(match cfg.get(CURRENT_CTX_KEY) {
        Some(serde_yaml::Value::String(s)) => Some(s.clone()),
        _ => None,
    })
}

pub fn current_context() -> Result<Option<(String, Context)>> {
    let Some(name) = current_context_name()? else {
        return Ok(None);
    };
    let mut ctxs = list_contexts()?;
    match ctxs.remove(&name) {
        Some(c) => Ok(Some((name, c))),
        None => Err(anyhow!(
            "current_context = {name:?} but no such entry under `contexts:` in config.yaml"
        )),
    }
}

pub fn set_context(name: &str, ctx: Context) -> Result<()> {
    if name.trim().is_empty() {
        bail!("context name cannot be empty");
    }
    let mut cfg = load_config()?;
    let mut ctxs = list_contexts()?;
    ctxs.insert(name.to_string(), ctx);
    cfg.insert(
        CTX_KEY.to_string(),
        serde_yaml::to_value(&ctxs).context("serialize contexts")?,
    );
    save_config(&cfg)
}

pub fn use_context(name: &str) -> Result<()> {
    let ctxs = list_contexts()?;
    if !ctxs.contains_key(name) {
        bail!(
            "no context named {name:?}. Available: {}",
            ctxs.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    let mut cfg = load_config()?;
    cfg.insert(
        CURRENT_CTX_KEY.to_string(),
        serde_yaml::Value::String(name.to_string()),
    );
    save_config(&cfg)
}

pub fn delete_context(name: &str) -> Result<bool> {
    let mut cfg = load_config()?;
    let mut ctxs = list_contexts()?;
    let removed = ctxs.remove(name).is_some();
    if removed {
        cfg.insert(
            CTX_KEY.to_string(),
            serde_yaml::to_value(&ctxs).context("serialize contexts")?,
        );
        // If we just removed the active context, unset current_context
        // so subsequent commands don't silently target a ghost.
        if let Some(serde_yaml::Value::String(cur)) = cfg.get(CURRENT_CTX_KEY).cloned()
            && cur == name
        {
            cfg.remove(CURRENT_CTX_KEY);
        }
        save_config(&cfg)?;
    }
    Ok(removed)
}
