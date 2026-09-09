//! `librefang purge --agent <name>` — remove every trace of an agent.
//!
//! Thin wrapper over `librefang_kernel::agent_purge`, the shared
//! implementation (written to be shared; the CLI is the only caller today).
//! Opens the database directly so the command works with no daemon running —
//! which is the usual situation when cleaning up from a previous partial delete.

use crate::commands::common::prompt_yes_no;
use crate::i18n;
use librefang_kernel::agent_purge::PurgeReport;
use librefang_memory::MemorySubstrate;
use librefang_types::config::KernelConfig;
use std::path::Path;

/// Memory decay rate handed to `MemorySubstrate::open` (0.0 = no decay,
/// 1.0 = aggressive decay; the kernel's own default is 0.1). Purge only
/// deletes rows and never runs decay or consolidation, so the value never
/// fires — the substrate just requires one.
const PURGE_DECAY_RATE: f32 = 0.01;

pub(crate) fn cmd_purge(config: Option<&Path>, agent: &str, yes: bool, dry_run: bool) -> i32 {
    let config = match resolve_config(config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", i18n::t_args("purge-failed-config", &[("error", &e)]));
            return 1;
        }
    };
    let home = config.home_dir.clone();
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

    let substrate = match MemorySubstrate::open(&db, PURGE_DECAY_RATE) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "{}",
                i18n::t_args("purge-failed-open-database", &[("error", &e.to_string())])
            );
            return 1;
        }
    };

    purge_with(&substrate, &config, &db, agent, dry_run, |_| {
        // On a non-TTY stdin the prompt reads EOF and answers "no", so --yes
        // is effectively required there — exactly the gate the review asked
        // for.
        yes || prompt_yes_no(&i18n::t("label-confirm-prompt"), false)
    })
}

/// Resolve the configuration that names the purge target, refusing to guess.
///
/// The config decides which database and which workspaces root the command
/// deletes from, so a config that cannot be loaded is not a degraded mode — it
/// means the target is unknown. `load_config` answers a failure with
/// `KernelConfig::default()`, and for `--config /srv/instance-b/config.toml` a
/// TOML error in that file would therefore retarget the deletion at the default
/// installation and purge its unrelated agent of the same name. Substituting a
/// target is not something a warning can make safe, which is why this uses the
/// strict loader: `try_load_config` returns `Err` for a read failure, a TOML
/// syntax error, a broken `include` chain, a failed migration, a deserialize
/// mismatch and an unknown field under `strict_config`, where `load_config`
/// falls back to defaults for all but the last three.
///
/// The one tolerated absence is a config file that is not there at the *default*
/// location and was not asked for by path. Nothing is substituted in that case:
/// the defaults describe the same default installation the operator meant, and
/// an install that never wrote a `config.toml` still has data worth cleaning up.
/// A default config that exists but cannot be read is a failure like any other —
/// it may well be the file that points `home_dir` somewhere else.
fn resolve_config(explicit: Option<&Path>) -> Result<KernelConfig, String> {
    let path = explicit
        .map(Path::to_path_buf)
        .unwrap_or_else(librefang_kernel::config::default_config_path);
    if explicit.is_none() && !path.exists() {
        return Ok(KernelConfig::default());
    }
    librefang_kernel::config::try_load_config(&path)
}

/// The command's decisions, with the database and the confirmation both
/// supplied by the caller so a test can drive them.
///
/// `confirm` is called with the planned report, and only when there is
/// something to purge: a prompt over a plan that removes nothing is a
/// question with one honest answer.
fn purge_with(
    substrate: &MemorySubstrate,
    config: &KernelConfig,
    db: &Path,
    agent: &str,
    dry_run: bool,
    confirm: impl FnOnce(&PurgeReport) -> bool,
) -> i32 {
    // Name the database before describing the plan. Removal categories say what
    // kind of thing goes; only the path says *whose*, which is the difference
    // between confirming a cleanup and confirming it against the wrong
    // installation.
    println!(
        "{}",
        i18n::t_args(
            "purge-database-line",
            &[("path", &db.display().to_string())]
        )
    );

    // Plan first in both directions. The destructive path used to print a
    // static warning listing everything a purge *can* remove and prompt on
    // that, so the operator confirmed a template rather than what was about to
    // happen — and was prompted even when the answer changed nothing.
    let plan = librefang_kernel::agent_purge::plan_purge(substrate, config, agent);
    let header = if dry_run {
        "purge-dry-run-header"
    } else {
        "purge-confirm-header"
    };
    let code = print_outcome(agent, &plan.preview, &plan.failures, header);
    if dry_run || code != 0 || plan.preview.is_empty() {
        // A plan with failures is one `purge_agent` refuses to execute, and an
        // empty one has nothing to confirm.
        return code;
    }

    eprintln!(
        "{}",
        i18n::t_args("purge-confirm-warning", &[("agent", agent)])
    );
    if !confirm(&plan.preview) {
        eprintln!("{}", i18n::t("label-aborted"));
        return 1;
    }

    let outcome = librefang_kernel::agent_purge::purge_agent(substrate, config, agent);
    print_outcome(
        agent,
        &outcome.report,
        &outcome.failures,
        "purge-purged-header",
    )
}

/// Print the report as localized lines and the failures as localized error
/// lines. `header` picks the dry-run ("would purge"), the confirmation
/// ("about to purge") or the real ("purged") heading; returns the process exit
/// code (0 clean, 1 on any failure).
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
        if report.workspace_unresolved {
            println!("{}", i18n::t("purge-workspace-unresolved"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn cfg_for(home: &tempfile::TempDir) -> KernelConfig {
        KernelConfig {
            home_dir: home.path().to_path_buf(),
            ..KernelConfig::default()
        }
    }

    /// The destructive path asks the operator about a plan, so a purge that
    /// would remove nothing asks nothing. It used to print the static warning
    /// and prompt regardless, because it never planned before prompting.
    #[test]
    fn a_purge_that_removes_nothing_never_prompts() {
        let home = tempfile::tempdir().unwrap();
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();

        let db = home.path().join("data").join("librefang.db");
        let code = purge_with(&substrate, &cfg_for(&home), &db, "nobody", false, |_| {
            panic!("prompted over a plan that removes nothing")
        });

        assert_eq!(code, 0);
    }

    /// And when there is something to remove, what reaches the prompt is the
    /// plan itself — not a template listing everything a purge can touch.
    #[test]
    fn the_prompt_is_shown_the_planned_report() {
        let home = tempfile::tempdir().unwrap();
        let types = librefang_types::agent_type_store::agent_types_dir_in(home.path());
        std::fs::create_dir_all(&types).unwrap();
        let agent_type = types.join("alpha.toml");
        std::fs::write(&agent_type, "x").unwrap();
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let seen = Cell::new(false);

        let db = home.path().join("data").join("librefang.db");
        let code = purge_with(&substrate, &cfg_for(&home), &db, "alpha", false, |plan| {
            seen.set(true);
            assert!(plan.agent_type_removed);
            assert!(!plan.roster_entry_removed);
            assert!(!plan.workspace_removed);
            false
        });

        assert!(seen.get(), "the plan never reached the prompt");
        assert_eq!(code, 1, "declining is not success");
        assert!(agent_type.exists(), "declining must not delete anything");
    }

    /// Serializes the two env-var-mutating tests below.
    /// `LIBREFANG_HOME` is process-wide state, and the whole point of these
    /// tests is what the command resolves it to.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// Point `LIBREFANG_HOME` at `home` for the duration of `body`, restoring
    /// the previous value afterwards.
    fn with_librefang_home<T>(home: &Path, body: impl FnOnce() -> T) -> T {
        let previous = std::env::var("LIBREFANG_HOME").ok();
        // SAFETY: every caller holds `env_lock`, so no other thread in this
        // process is reading or writing the variable concurrently.
        unsafe { std::env::set_var("LIBREFANG_HOME", home) };
        let out = body();
        // SAFETY: see above — still under `env_lock`.
        unsafe {
            match previous {
                Some(v) => std::env::set_var("LIBREFANG_HOME", v),
                None => std::env::remove_var("LIBREFANG_HOME"),
            }
        }
        out
    }

    /// A default installation carrying an agent-type template for `agent`, and
    /// a database for `cmd_purge` to find. Returns the template's path so a
    /// test can assert it survived.
    fn default_installation_with(home: &Path, agent: &str) -> std::path::PathBuf {
        let types = librefang_types::agent_type_store::agent_types_dir_in(home);
        std::fs::create_dir_all(&types).unwrap();
        let agent_type = types.join(format!("{agent}.toml"));
        std::fs::write(&agent_type, "x").unwrap();
        let data = home.join("data");
        std::fs::create_dir_all(&data).unwrap();
        drop(MemorySubstrate::open(&data.join("librefang.db"), PURGE_DECAY_RATE).unwrap());
        agent_type
    }

    /// A config that cannot be loaded must abort the purge, not silently
    /// retarget it at the default installation. `--config <broken>` used to
    /// warn and fall back to `KernelConfig::default()`, whose `home_dir` is the
    /// default installation — so a TOML error deleted an unrelated agent that
    /// merely shared a name with the intended one.
    #[test]
    fn an_unloadable_explicit_config_purges_nothing_from_the_default_installation() {
        let _guard = env_lock();
        let default_home = tempfile::tempdir().unwrap();
        let agent_type = default_installation_with(default_home.path(), "worker");

        let broken = tempfile::tempdir().unwrap();
        let broken_config = broken.path().join("config.toml");
        std::fs::write(&broken_config, "this is not = = valid toml").unwrap();

        let code = with_librefang_home(default_home.path(), || {
            cmd_purge(Some(&broken_config), "worker", true, false)
        });

        assert_eq!(code, 1, "an unloadable config must fail the command");
        assert!(
            agent_type.exists(),
            "purge substituted the default installation as its target"
        );
    }

    /// The same substitution, through the failure mode that survives a naive
    /// fix: `load_config` answers a *missing* file with `Ok(defaults)` rather
    /// than `Err`, so a `--config` pointing at a path that does not exist —
    /// a typo, an unmounted volume — never reaches the error branch at all.
    #[test]
    fn a_missing_explicit_config_purges_nothing_from_the_default_installation() {
        let _guard = env_lock();
        let default_home = tempfile::tempdir().unwrap();
        let agent_type = default_installation_with(default_home.path(), "worker");

        let elsewhere = tempfile::tempdir().unwrap();
        let absent = elsewhere.path().join("never-written.toml");

        let code = with_librefang_home(default_home.path(), || {
            cmd_purge(Some(&absent), "worker", true, false)
        });

        assert_eq!(code, 1, "a config that is not there must fail the command");
        assert!(
            agent_type.exists(),
            "purge substituted the default installation as its target"
        );
    }

    /// A config file that is simply absent from the default location is not a
    /// failure: the defaults describe the very installation the operator meant,
    /// so nothing is substituted and the command still works on an install that
    /// never wrote a `config.toml`.
    #[test]
    fn a_missing_default_config_still_purges_the_default_installation() {
        let _guard = env_lock();
        let default_home = tempfile::tempdir().unwrap();
        let agent_type = default_installation_with(default_home.path(), "worker");

        let code = with_librefang_home(default_home.path(), || {
            cmd_purge(None, "worker", true, false)
        });

        assert_eq!(code, 0, "a config-less installation is still purgeable");
        assert!(!agent_type.exists(), "the agent-type template survived");
    }
}
