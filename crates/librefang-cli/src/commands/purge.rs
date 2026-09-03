//! `librefang purge --agent <name>` — remove every trace of an agent.
//!
//! Thin wrapper over `librefang_kernel::agent_purge`, the shared
//! implementation (written to be shared; the CLI is the only caller today).
//! Opens the database directly so the command works with no daemon running —
//! which is the usual situation when cleaning up from a previous partial delete.

use crate::commands::common::{librefang_home, prompt_yes_no};
use crate::i18n;
use librefang_kernel::agent_purge::PurgeReport;

/// Memory decay rate handed to `MemorySubstrate::open` (0.0 = no decay,
/// 1.0 = aggressive decay; the kernel's own default is 0.1). Purge only
/// deletes rows and never runs decay or consolidation, so the value never
/// fires — the substrate just requires one.
const PURGE_DECAY_RATE: f32 = 0.01;

pub(crate) fn cmd_purge(agent: &str, yes: bool, dry_run: bool) -> i32 {
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

    let substrate = match librefang_memory::MemorySubstrate::open(&db, PURGE_DECAY_RATE) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "{}",
                i18n::t_args("purge-failed-open-database", &[("error", &e.to_string())])
            );
            return 1;
        }
    };

    if dry_run {
        let plan = librefang_kernel::agent_purge::plan_purge(&substrate, &home, agent);
        return print_outcome(agent, &plan.preview, &plan.failures, "purge-dry-run-header");
    }

    // Destructive path: warn, then confirm unless --yes. On a non-TTY stdin
    // the prompt reads EOF and answers "no", so --yes is effectively required
    // there — exactly the gate the review asked for.
    eprintln!(
        "{}",
        i18n::t_args("purge-confirm-warning", &[("agent", agent)])
    );
    if !yes && !prompt_yes_no(&i18n::t("label-confirm-prompt"), false) {
        eprintln!("{}", i18n::t("label-aborted"));
        return 1;
    }

    let outcome = librefang_kernel::agent_purge::purge_agent(&substrate, &home, agent);
    print_outcome(
        agent,
        &outcome.report,
        &outcome.failures,
        "purge-purged-header",
    )
}

/// Print the report as localized lines and the failures as localized error
/// lines. `header` picks the dry-run ("would purge") or the real ("purged")
/// heading; returns the process exit code (0 clean, 1 on any failure).
fn print_outcome(agent: &str, report: &PurgeReport, failures: &[String], header: &str) -> i32 {
    if report.is_empty() && failures.is_empty() {
        println!(
            "{}",
            i18n::t_args("purge-nothing-to-purge", &[("agent", agent)])
        );
        return 0;
    }
    if !report.is_empty() {
        println!("{}", i18n::t_args(header, &[("agent", agent)]));
        if report.roster_entry_removed {
            println!("{}", i18n::t("purge-removed-roster-entry"));
        }
        if report.orphaned_data_removed {
            println!("{}", i18n::t("purge-removed-orphaned-data"));
        }
        if report.identity_record_removed {
            println!("{}", i18n::t("purge-removed-identity-record"));
        }
        if report.workspace_removed {
            println!("{}", i18n::t("purge-removed-workspace"));
        }
        if report.agent_type_removed {
            println!("{}", i18n::t("purge-removed-agent-type"));
        }
    }
    for f in failures {
        eprintln!("{}", i18n::t_args("purge-failure-line", &[("error", f)]));
    }
    if failures.is_empty() {
        0
    } else {
        1
    }
}
