// SPDX-License-Identifier: BUSL-1.1
//! `forgeos chat <agent_id>` — interactive A2H chat session with an agent.
//!
//! Opens a session via POST /api/a2h/v1/chats, then loops:
//!   1. read a line from the user
//!   2. POST it as a `human` chat message
//!   3. invoke the agent (sync, with prior history in the prompt)
//!   4. POST the agent's reply as an `agent` chat message
//!   5. display it
//! `/exit`, `/quit`, or EOF closes the session.

use anyhow::Result;
use clap::Args as ClapArgs;
use colored::Colorize;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

use crate::api::{self, Endpoint};
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
    let mut history: Vec<(String, String)> = vec![];
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
        let user_msg = line.trim();
        if user_msg.is_empty() {
            continue;
        }
        if user_msg == "/exit" || user_msg == "/quit" {
            break;
        }

        // Post the human message into the chat.
        let _: Value = api::post_json(
            ep,
            &format!("/api/a2h/v1/chats/{}/messages", chat_id),
            &json!({
                "role": "human",
                "sender": args.as_user,
                "content": user_msg,
            }),
        )?;

        // Compose the invocation prompt with prior turns for coherence.
        // (forgeos invoke today is stateless across calls; we feed history
        // through the prompt itself. Bounded to the last ~30 messages.)
        let mut composed = String::new();
        if !history.is_empty() {
            composed.push_str("Conversation so far:\n");
            for (role, content) in &history {
                composed.push_str(&format!("[{}]: {}\n", role, content));
            }
            composed.push('\n');
        }
        composed.push_str(&format!("[user]: {}", user_msg));

        // Invoke the agent synchronously (no async_mode here — we want the
        // reply now).
        let invoke_body = json!({
            "prompt": composed,
            "context": {
                "chat_id": chat_id,
                "session_id": chat_id,
            },
        });
        let invoke_resp: Value = api::post_json(
            ep,
            &format!("/api/platform/agents/{}/invoke", args.agent_id),
            &invoke_body,
        )?;
        let agent_reply = invoke_resp
            .get("result")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                invoke_resp
                    .get("error")
                    .and_then(|v| v.as_str())
                    .map(|s| format!("(agent error: {})", s))
                    .unwrap_or_else(|| "(no output)".to_string())
            });

        // Post the agent's reply into the chat.
        let _: Value = api::post_json(
            ep,
            &format!("/api/a2h/v1/chats/{}/messages", chat_id),
            &json!({
                "role": "agent",
                "sender": agent_name,
                "content": agent_reply,
            }),
        )?;

        println!("{} {}\n", "Agent>".green().bold(), agent_reply);

        history.push(("user".to_string(), user_msg.to_string()));
        history.push(("agent".to_string(), agent_reply));
        if history.len() > 30 {
            let drop = history.len() - 30;
            history.drain(0..drop);
        }
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
