//! `librefang purge --agent <name>` — remove every trace of an agent.
//!
//! Thin wrapper over `librefang_kernel::agent_purge::purge_agent`, the shared
//! implementation the API, the dashboard and the TUI use too. Opens the
//! database directly so the command works with no daemon running — which is
//! the usual situation when cleaning up after an agent that is already gone.

use crate::commands::common::librefang_home;
use crate::i18n;

pub fn cmd_purge(agent: &str) -> i32 {
    let home = librefang_home();
    let db = home.join("data").join("librefang.db");
    if !db.exists() {
        eprintln!(
            "{}",
            i18n::t_args(
                "purge-failed-no-database",
                &[("path", &db.display().to_string())]
            )
        );
        return 1;
    }

    let substrate = match librefang_memory::MemorySubstrate::open(&db, 0.01) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "{}",
                i18n::t_args("purge-failed-open-database", &[("error", &e.to_string())])
            );
            return 1;
        }
    };

    match librefang_kernel::agent_purge::purge_agent(&substrate, &home, agent) {
        Ok(report) if report.is_empty() => {
            println!(
                "{}",
                i18n::t_args("purge-nothing-to-purge", &[("agent", agent)])
            );
            0
        }
        Ok(report) => {
            println!(
                "{}",
                i18n::t_args("purge-purged-header", &[("agent", agent)])
            );
            if report.roster_entry_removed {
                println!("{}", i18n::t("purge-removed-roster-entry"));
            }
            if report.workspace_removed {
                println!("{}", i18n::t("purge-removed-workspace"));
            }
            if report.agent_type_removed {
                println!("{}", i18n::t("purge-removed-agent-type"));
            }
            0
        }
        Err(e) => {
            eprintln!(
                "{}",
                i18n::t_args("purge-failed", &[("error", &e.to_string())])
            );
            1
        }
    }
}
