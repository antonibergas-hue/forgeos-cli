// SPDX-License-Identifier: BUSL-1.1
//! Terminal output helpers.

use colored::Colorize;

pub fn ok(msg: &str) {
    println!("{} {}", "✓".green(), msg);
}

pub fn warn(msg: &str) {
    println!("{} {}", "!".yellow(), msg);
}

pub fn err(msg: &str) {
    eprintln!("{} {}", "✗".red(), msg);
}
