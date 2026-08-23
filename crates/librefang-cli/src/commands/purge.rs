//! `librefang purge --agent <name>` — remove every trace of an agent.
//!
//! Thin wrapper over `librefang_kernel::agent_purge::purge_agent`, the shared
//! implementation the API, the dashboard and the TUI use too. Opens the
//! database directly so the command works with no daemon running — which is
//! the usual situation when cleaning up after an agent that is already gone.

use crate::commands::common::librefang_home;

pub fn cmd_purge(agent: &str) -> i32 {
    let home = librefang_home();
    let db = home.join("data").join("librefang.db");
    if !db.exists() {
        eprintln!("Purge failed: no database at {}", db.display());
        return 1;
    }

    let substrate = match librefang_memory::MemorySubstrate::open(&db, 0.01) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Purge failed: open database: {e}");
            return 1;
        }
    };

    match librefang_kernel::agent_purge::purge_agent(&substrate, &home, agent) {
        Ok(report) if report.is_empty() => {
            println!("Nothing to purge: '{agent}' left no trace in this installation.");
            0
        }
        Ok(report) => {
            println!("Purged '{agent}':");
            if report.roster_entry_removed {
                println!("  - roster entry, sessions, memories and KV rows");
            }
            if report.workspace_removed {
                println!("  - workspace directory");
            }
            if report.agent_type_removed {
                println!("  - agent-type template");
            }
            0
        }
        Err(e) => {
            eprintln!("Purge failed: {e}");
            1
        }
    }
}
