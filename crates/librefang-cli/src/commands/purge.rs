//! `librefang purge --agent <name>` — remove every trace of an agent.
//!
//! Deletes the agent's workspace directory, its roster entry, its sessions,
//! memories and KV rows (the substrate's `remove_agent` cascade), and any
//! agent-type with the same name. Meant for agents the operator already
//! deleted but whose data lingers — the cleanup the dashboard delete does
//! not do.

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

    let entries = match substrate.load_all_agents() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Purge failed: read roster: {e}");
            return 1;
        }
    };
    let target = entries.iter().find(|e| e.name == agent);
    if let Some(entry) = target {
        if let Err(e) = substrate.remove_agent(entry.id) {
            eprintln!("Purge failed: remove roster/sessions/memories: {e}");
            return 1;
        }
    }

    let workspace = home.join("workspaces").join("agents").join(agent);
    if workspace.exists() {
        if let Err(e) = std::fs::remove_dir_all(&workspace) {
            eprintln!("Purge failed: remove workspace: {e}");
            return 1;
        }
    }

    let type_file = home
        .join("workspaces")
        .join("agent-types")
        .join(format!("{agent}.toml"));
    if type_file.exists() {
        let _ = std::fs::remove_file(&type_file);
    }

    println!("Purged '{agent}': roster, sessions, memories, workspace and agent-type removed.");
    0
}
