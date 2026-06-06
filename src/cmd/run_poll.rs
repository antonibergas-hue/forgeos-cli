// SPDX-License-Identifier: BUSL-1.1
//! Shared run-handle model + polling for the runtime-v2 (durable) execution
//! path.
//!
//! An invoke against a worker-tier server returns a *run handle* before the run
//! finishes: it may be `running`, `paused` (awaiting human approval),
//! `completed`, or `failed`. Callers poll `GET /api/platform/runs/{run_id}`
//! until the run settles. Every field is optional so a legacy (non-worker)
//! server's inline `{result, ...}` response — which carries no `status` or
//! `run_id` — still deserializes and is treated as already-completed.

use anyhow::Result;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{thread, time::Duration};

use crate::api::{self, Endpoint};

/// One tool call parked on a human-approval gate. `extra` captures whatever the
/// server attaches beyond the known fields (notably `args` — the tool input —
/// so the operator can see *what* they're approving).
#[allow(dead_code)]
#[derive(Deserialize, Debug, Default)]
pub struct Pending {
    pub request_id: Option<String>,
    pub tool: Option<String>,
    pub tool_use_id: Option<String>,
    pub suspend_reason: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Pending {
    /// Best-effort tool-input object: the server sends it under `args`, but be
    /// tolerant of `input` / `arguments` too.
    pub fn args(&self) -> Option<&Map<String, Value>> {
        for key in ["args", "input", "arguments"] {
            if let Some(Value::Object(m)) = self.extra.get(key) {
                return Some(m);
            }
        }
        None
    }
}

/// Runtime-v2 run handle. All fields optional (see module docs). Some fields
/// (warnings/simulated/suspend_reason) mirror the full server contract for
/// future consumers even when this caller doesn't read them.
#[allow(dead_code)]
#[derive(Deserialize, Debug, Default)]
pub struct RunHandle {
    pub run_id: Option<String>,
    pub continuation_id: Option<String>,
    pub status: Option<String>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub suspend_reason: Option<String>,
    pub pending: Option<Vec<Pending>>,
    pub warnings: Option<Vec<String>>,
    pub simulated: Option<bool>,
}

impl RunHandle {
    /// The id used to poll the run — prefer `run_id`, fall back to
    /// `continuation_id` (they're the same value today, but be defensive).
    pub fn poll_id(&self) -> Option<&str> {
        self.run_id.as_deref().or(self.continuation_id.as_deref())
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum RunState {
    Running,
    Paused,
    Completed,
    Failed,
    Unknown,
}

/// Map a run's `status` string to a state. A *missing* status means the legacy
/// inline path already returned a result, so it is `Completed` (no polling).
pub fn classify(status: Option<&str>) -> RunState {
    match status {
        Some("completed") | Some("done") => RunState::Completed,
        Some("failed") => RunState::Failed,
        Some("paused") | Some("suspended") => RunState::Paused,
        Some("running") | Some("resuming") | Some("queued") => RunState::Running,
        None => RunState::Completed,
        _ => RunState::Unknown,
    }
}

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_TRANSIENT_ERRORS: u32 = 5;

/// Poll `GET /api/platform/runs/{run_id}` every ~2s until the run is no longer
/// `running` (i.e. paused | completed | failed | unknown), and return that
/// settled handle. Bounds consecutive transient GET errors so a dead server
/// surfaces an error instead of spinning forever.
pub fn poll_until_settled(ep: &Endpoint, run_id: &str) -> Result<RunHandle> {
    let path = format!("/api/platform/runs/{run_id}");
    let mut transient = 0u32;
    loop {
        thread::sleep(POLL_INTERVAL);
        match api::get::<RunHandle>(ep, &path) {
            Ok(h) => {
                transient = 0;
                if classify(h.status.as_deref()) != RunState::Running {
                    return Ok(h);
                }
            }
            Err(e) => {
                transient += 1;
                if transient >= MAX_TRANSIENT_ERRORS {
                    return Err(e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_states() {
        assert_eq!(classify(Some("completed")), RunState::Completed);
        assert_eq!(classify(Some("done")), RunState::Completed);
        assert_eq!(classify(Some("failed")), RunState::Failed);
        assert_eq!(classify(Some("paused")), RunState::Paused);
        assert_eq!(classify(Some("suspended")), RunState::Paused);
        assert_eq!(classify(Some("running")), RunState::Running);
        // Legacy inline response: no status == already done.
        assert_eq!(classify(None), RunState::Completed);
        assert_eq!(classify(Some("weird")), RunState::Unknown);
    }

    #[test]
    fn legacy_inline_body_deserializes() {
        let body = serde_json::json!({"result": "hi there", "simulated": false});
        let h: RunHandle = serde_json::from_value(body).unwrap();
        assert_eq!(h.result.as_deref(), Some("hi there"));
        assert_eq!(h.status, None);
        assert_eq!(classify(h.status.as_deref()), RunState::Completed);
    }

    #[test]
    fn worker_paused_body_with_args() {
        let body = serde_json::json!({
            "run_id": "cont_abc",
            "status": "paused",
            "suspend_reason": "human_approval",
            "pending": [{
                "request_id": "req_123",
                "tool": "notify__email",
                "tool_use_id": "tu_1",
                "args": {"to": "a@b.com", "subject": "Q2", "body": "hello"}
            }]
        });
        let h: RunHandle = serde_json::from_value(body).unwrap();
        assert_eq!(classify(h.status.as_deref()), RunState::Paused);
        assert_eq!(h.poll_id(), Some("cont_abc"));
        let p = &h.pending.as_ref().unwrap()[0];
        assert_eq!(p.request_id.as_deref(), Some("req_123"));
        assert_eq!(p.tool.as_deref(), Some("notify__email"));
        let args = p.args().unwrap();
        assert_eq!(args.get("to").unwrap(), "a@b.com");
        assert_eq!(args.get("subject").unwrap(), "Q2");
    }
}
