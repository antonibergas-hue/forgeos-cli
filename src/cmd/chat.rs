// SPDX-License-Identifier: BUSL-1.1
//! `forgeos chat <agent_id>` — interactive A2H chat session with an agent.
//!
//! Each message becomes a durable run on the server (the worker tier processes
//! every LLM turn as its own Redis task). The CLI drives one turn like this:
//!   1. read a line from the user
//!   2. POST it as a `human` chat message (audit trail)
//!   3. invoke the agent with `session_id` = chat id (server keeps memory)
//!   4. poll the run; if it PAUSES on a human-approval gate (e.g. the agent
//!      wants to send an email), show the pending action and ask `Approve? [Y/n]`
//!      inline — approve/reject, then keep polling until the run completes
//!   5. print the agent's reply and post it as an `agent` chat message
//! `/exit`, `/quit`, or EOF closes the session.

use anyhow::Result;
use clap::Args as ClapArgs;
use colored::Colorize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{self, BufRead, Stdin, Write};

use crate::api::{self, Endpoint};
use crate::cmd::approvals;
use crate::cmd::run_poll::{self, Pending, RunHandle, RunState};
use crate::ui;

#[derive(ClapArgs)]
pub struct Args {
    /// Agent id (from `forgeos list`).
    pub agent_id: String,

    /// Your name in the chat. Default: "operator".
    #[arg(long, default_value = "operator")]
    pub as_user: String,

    /// Optional opening topic / title for the session.
    #[arg(long)]
    pub topic: Option<String>,
}

/// What a single conversational turn produced.
enum TurnOutcome {
    Reply(String),
    Failed(String),
}

pub fn run(args: Args, ep: &Endpoint) -> Result<i32> {
    // 1. Resolve the agent so we know its name + namespace.
    let agent_path = format!("/api/platform/agents/{}", args.agent_id);
    let agent: Value = api::get(ep, &agent_path)?;
    let agent_name = agent
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("agent")
        .to_string();
    let namespace = agent
        .get("namespace")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();

    // 2. Open the A2H chat session.
    let open_body = json!({
        "agent_pid": args.agent_id,
        "agent_namespace": namespace,
        "agent_name": agent_name,
        "human_name": args.as_user,
        "human_namespace": namespace,
        "topic": args.topic.clone().unwrap_or_default(),
    });
    let session: Value = api::post_json(ep, "/api/a2h/v1/chats", &open_body)?;
    let chat_id = session
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if chat_id.is_empty() {
        anyhow::bail!("server did not return a chat id");
    }

    ui::ok(&format!(
        "chat opened: {}  (agent: {}/{})",
        chat_id, namespace, agent_name
    ));
    println!(
        "  type {} (or {}) to end. EOF / Ctrl-D also works.\n",
        "/exit".bold(),
        "/quit".bold()
    );

    // 3. REPL.
    let stdin = io::stdin();
    let mut line = String::new();
    loop {
        line.clear();
        print!("{} ", "You>".cyan().bold());
        io::stdout().flush().ok();
        let n = stdin.lock().read_line(&mut line)?;
        if n == 0 {
            break; // EOF
        }
        let user_msg = line.trim().to_string();
        if user_msg.is_empty() {
            continue;
        }
        if user_msg == "/exit" || user_msg == "/quit" {
            break;
        }

        // Post the human message into the chat (audit trail). `client_drives`
        // tells the server NOT to auto-invoke the agent — this CLI drives its
        // own /invoke below (so it can show the [Y/n] approval inline). Without
        // this the agent would run twice per turn.
        let _: Value = api::post_json(
            ep,
            &format!("/api/a2h/v1/chats/{}/messages", chat_id),
            &json!({ "role": "human", "sender": args.as_user, "content": user_msg,
                     "client_drives": true }),
        )?;

        // Invoke the agent as a durable run. Memory is server-side, keyed by
        // session_id — no need to stuff prior turns into the prompt.
        let invoke_body = json!({
            "prompt": user_msg,
            "session_id": chat_id,
            "context": { "chat_id": chat_id, "session_id": chat_id },
        });
        let handle: RunHandle = match api::post_json(
            ep,
            &format!("/api/platform/agents/{}/invoke", args.agent_id),
            &invoke_body,
        ) {
            Ok(h) => h,
            Err(e) => {
                ui::err(&format!("invoke failed: {e}"));
                continue; // keep the session open so the user can retry
            }
        };

        // Drive the run to a reply, handling any [Y/n] approval pauses.
        let outcome = match resolve_turn(ep, handle, &stdin) {
            Ok(o) => o,
            Err(e) => {
                ui::err(&format!("turn failed: {e}"));
                continue;
            }
        };

        let reply = match outcome {
            TurnOutcome::Reply(text) => text,
            TurnOutcome::Failed(err) => {
                ui::err(&format!("agent run failed: {err}"));
                continue;
            }
        };

        // Post the agent's reply into the chat + display it.
        let _: Value = api::post_json(
            ep,
            &format!("/api/a2h/v1/chats/{}/messages", chat_id),
            &json!({ "role": "agent", "sender": agent_name, "content": reply }),
        )?;
        println!("{} {}\n", "Agent>".green().bold(), reply);
    }

    // 4. Close the session.
    let _: Value = api::post_json(
        ep,
        &format!("/api/a2h/v1/chats/{}/close", chat_id),
        &json!({ "reason": "user exit" }),
    )?;
    ui::ok("chat closed.");
    Ok(0)
}

/// Drive a run handle to a terminal reply. Loops while the run is `running`
/// (poll) or `paused` (prompt the operator, approve/reject, then poll again),
/// so a turn that pauses several times is handled transparently.
fn resolve_turn(ep: &Endpoint, mut handle: RunHandle, stdin: &Stdin) -> Result<TurnOutcome> {
    // Request ids we've already approved/rejected this turn. Prevents re-acting
    // on the same (already-consumed) request while the async resume is still in
    // flight and the run momentarily still reads `paused` with that id.
    let mut handled: HashSet<String> = HashSet::new();
    loop {
        match run_poll::classify(handle.status.as_deref()) {
            RunState::Completed => {
                let text = handle.result.clone().unwrap_or_default();
                if text.is_empty() {
                    return Ok(TurnOutcome::Reply("(no output)".to_string()));
                }
                return Ok(TurnOutcome::Reply(text));
            }
            RunState::Failed => {
                return Ok(TurnOutcome::Failed(
                    handle.error.clone().unwrap_or_else(|| "run failed".to_string()),
                ));
            }
            RunState::Running => {
                let Some(run_id) = handle.poll_id().map(str::to_string) else {
                    return Ok(TurnOutcome::Failed(
                        "run is running but the server returned no run id to poll".to_string(),
                    ));
                };
                handle = run_poll::poll_until_actionable(ep, &run_id, &handled)?;
            }
            RunState::Paused => {
                let pendings = handle.pending.take().unwrap_or_default();
                if pendings.is_empty() {
                    return Ok(TurnOutcome::Failed(
                        "run paused for approval but the server listed no pending action"
                            .to_string(),
                    ));
                }
                for p in &pendings {
                    let Some(request_id) = p.request_id.as_deref() else {
                        ui::warn("pending approval has no request id; skipping");
                        continue;
                    };
                    // Skip requests already approved/rejected (stale paused snapshot).
                    if handled.contains(request_id) {
                        continue;
                    }
                    print_pending(p);
                    match prompt_approval(stdin)? {
                        Some(true) => {
                            approvals::approve(ep, request_id, None)?;
                            handled.insert(request_id.to_string());
                            ui::ok(&format!("approved {request_id}"));
                        }
                        Some(false) => {
                            approvals::reject(ep, request_id, Some("rejected from chat"))?;
                            handled.insert(request_id.to_string());
                            ui::warn(&format!("rejected {request_id}"));
                        }
                        None => {
                            // EOF at the prompt — reject and stop, never auto-send.
                            approvals::reject(ep, request_id, Some("rejected (eof)"))?;
                            return Ok(TurnOutcome::Failed(
                                "approval aborted (EOF)".to_string(),
                            ));
                        }
                    }
                }
                let Some(run_id) = handle.poll_id().map(str::to_string) else {
                    return Ok(TurnOutcome::Failed(
                        "run paused but the server returned no run id to poll".to_string(),
                    ));
                };
                handle = run_poll::poll_until_actionable(ep, &run_id, &handled)?;
            }
            RunState::Unknown => {
                // Treat as terminal: surface a result if any, else an error.
                if let Some(text) = handle.result.clone() {
                    return Ok(TurnOutcome::Reply(text));
                }
                return Ok(TurnOutcome::Failed(format!(
                    "unknown run status: {}",
                    handle.status.as_deref().unwrap_or("?")
                )));
            }
        }
    }
}

/// Show the operator what the agent wants to do before they approve it.
fn print_pending(p: &Pending) {
    let tool = p.tool.as_deref().unwrap_or("(tool)");
    ui::warn(&format!("⏸ the agent wants to run {}", tool.bold().yellow()));
    if let Some(args) = p.args() {
        // Surface the email-shaped fields if present; otherwise dump the args.
        let to = args
            .get("to")
            .or_else(|| args.get("recipient"))
            .and_then(|v| v.as_str());
        let subject = args.get("subject").and_then(|v| v.as_str());
        let body = args
            .get("body")
            .or_else(|| args.get("text"))
            .and_then(|v| v.as_str());
        if to.is_some() || subject.is_some() || body.is_some() {
            if let Some(to) = to {
                println!("    to:      {to}");
            }
            if let Some(subject) = subject {
                println!("    subject: {subject}");
            }
            if let Some(body) = body {
                let preview: String = body.chars().take(200).collect();
                let ell = if body.chars().count() > 200 { "…" } else { "" };
                println!("    body:    {preview}{ell}");
            }
        } else {
            // Non-email tool — show the raw args compactly.
            if let Ok(s) = serde_json::to_string(args) {
                println!("    args: {s}");
            }
        }
    }
}

/// Prompt `Approve? [Y/n]` on the same stdin the REPL reads from.
/// Returns Some(true)=approve, Some(false)=reject, None=EOF. Empty enter
/// defaults to approve; an unrecognised answer re-prompts (bounded).
fn prompt_approval(stdin: &Stdin) -> Result<Option<bool>> {
    for _ in 0..3 {
        print!("{} ", "Approve? [Y/n]".bold());
        io::stdout().flush().ok();
        let mut buf = String::new();
        let n = stdin.lock().read_line(&mut buf)?;
        if n == 0 {
            return Ok(None); // EOF
        }
        match buf.trim().to_lowercase().as_str() {
            "" | "y" | "yes" => return Ok(Some(true)),
            "n" | "no" => return Ok(Some(false)),
            _ => {
                ui::warn("please answer y or n");
                continue;
            }
        }
    }
    // Too many invalid answers — fail safe (do not send).
    Ok(Some(false))
}
