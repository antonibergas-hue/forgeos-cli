// SPDX-License-Identifier: BUSL-1.1
//! `forgeos validate <file>` — local-only manifest preview.

use anyhow::Result;
use clap::Args as ClapArgs;
use std::path::PathBuf;

use crate::manifest;
use crate::ui;

#[derive(ClapArgs)]
pub struct Args {
    /// Path to agent.yaml or agent.json
    pub file: PathBuf,
}

pub fn run(args: Args) -> Result<i32> {
    let m = manifest::load(&args.file)?;
    ui::ok(&format!("Manifest valid: {}", m.metadata.name));
    println!("  Stack:          {}", m.spec.stack);
    println!("  Execution type: {}", m.spec.execution_type);
    println!("  Ownership:      {}", m.spec.ownership);
    if let Some(llm) = m.spec.llm.as_ref() {
        println!("  Model:          {} ({})", llm.chat_model, llm.provider);
    }
    if let Some(s) = m.spec.schedule.as_ref() {
        println!("  Schedule:       {s}");
    }
    if !m.spec.event_triggers.is_empty() {
        println!("  Events:         {:?}", m.spec.event_triggers);
    }
    if !m.spec.tools.is_empty() {
        let preview: Vec<&str> = m
            .spec
            .tools
            .iter()
            .take(3)
            .map(|s| s.as_str())
            .collect();
        println!(
            "  Tools:          {} ({}...)",
            m.spec.tools.len(),
            preview.join(", ")
        );
    }
    if let Some(serde_yaml::Value::String(s)) = m.spec.system_prompt.as_ref() {
        println!("  System prompt:  {} chars", s.chars().count());
    }
    Ok(0)
}
