//! Purge every trace of an agent.
//!
//! Written to be shared; the CLI (`librefang purge`) is the only caller
//! today. Deleting an agent removes it from the roster and stops it
//! running, but leaves its workspace directory and any agent-type of the
//! same name on disk, and (for an agent whose roster entry is already gone)
//! its sessions and memories in the database with nothing left pointing at
//! them. This module is the one place that cleans all of it.
//!
//! Deliberately a free function rather than a `Kernel` method: the CLI purges
//! without a running daemon by opening the database directly, and hanging
//! this off the kernel would force it to boot one.

use crate::agent_identity_registry::AgentIdentityRegistry;
use librefang_memory::MemorySubstrate;
use librefang_types::agent::AgentId;
use librefang_types::agent_type_store::{agent_type_path_in, validate_agent_type_name};
use std::path::{Path, PathBuf};

/// What a purge actually removed. Every field is what happened, not what was
/// attempted, so a caller can report the truth rather than an intention.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct PurgeReport {
    /// The agent had a roster entry, and it (plus its sessions, memories and
    /// KV rows, via the substrate's cascade) was removed.
    pub roster_entry_removed: bool,
    /// The roster entry was already gone, but sessions, memories and KV rows
    /// survived it (a partial or pre-cascade delete); they were found by
    /// recovering the agent id from its name and removed the same way.
    pub orphaned_data_removed: bool,
    /// The name → canonical-UUID record in `agent_identities.toml` was
    /// dropped, so a future agent with the same name is not pinned to the
    /// purged agent's UUID.
    pub identity_record_removed: bool,
    /// The agent's workspace directory was deleted.
    pub workspace_removed: bool,
    /// An agent-type template of the same name was deleted.
    pub agent_type_removed: bool,
}

impl PurgeReport {
    /// True when nothing at all was found to remove — the caller asked to
    /// purge something that leaves no trace anywhere.
    pub fn is_empty(&self) -> bool {
        !(self.roster_entry_removed
            || self.orphaned_data_removed
            || self.identity_record_removed
            || self.workspace_removed
            || self.agent_type_removed)
    }
}

/// Read-only preview of what a purge would remove, as computed by
/// [`plan_purge`]. `--dry-run` prints `preview` and stops; the destructive
/// path executes the plan.
#[derive(Debug, Clone, Default)]
pub struct PurgePlan {
    /// What a purge of this agent would remove, computed without writing.
    pub preview: PurgeReport,
    /// Read errors hit while planning. A plan with failures must not be
    /// executed: a roster read failure means live agents cannot be told
    /// apart from orphans, and executing could cascade a live agent's data.
    pub failures: Vec<String>,
    /// The roster entry's id, when the roster still knows this name.
    pub roster_agent_id: Option<AgentId>,
    /// Ids whose substrate rows are orphaned — rows exist, no roster entry
    /// holds the id.
    pub orphan_agent_ids: Vec<AgentId>,
    /// Whether the canonical-UUID registry holds a record for this name.
    pub registry_record: bool,
    /// The workspace directory that would be deleted, when it exists.
    pub workspace: Option<PathBuf>,
    /// The agent-type file that would be deleted, when it exists.
    pub agent_type: Option<PathBuf>,
}

/// What a purge did, alongside everything that went wrong.
///
/// Failures do not abort the run: the roster cascade runs first, so a
/// workspace delete that fails must not leave the caller without the report
/// of what already happened. An outcome with a non-empty `report` and a
/// non-empty `failures` list is a partial purge, and rerunning the command
/// cleans up the rest (the whole module is idempotent by design).
#[derive(Debug, Clone, Default)]
pub struct PurgeOutcome {
    /// What was actually removed.
    pub report: PurgeReport,
    /// Every step that failed, with the reason.
    pub failures: Vec<String>,
}

/// Read-only preview of what [`purge_agent`] would remove for `agent_name`.
/// Never writes. Confirmation prompts and `--dry-run` show this, so the
/// operator confirms what will actually happen rather than a guess.
pub fn plan_purge(substrate: &MemorySubstrate, home: &Path, agent_name: &str) -> PurgePlan {
    let mut failures = Vec::new();
    if let Err(reason) = validate_agent_type_name(agent_name) {
        failures.push(format!("invalid agent name {agent_name:?}: {reason}"));
        return PurgePlan {
            failures,
            ..PurgePlan::default()
        };
    }

    let entries = match substrate.load_all_agents() {
        Ok(entries) => entries,
        Err(e) => {
            failures.push(format!("read roster: {e}"));
            return PurgePlan {
                failures,
                ..PurgePlan::default()
            };
        }
    };

    let registry = AgentIdentityRegistry::load(home);
    let registry_record = registry.get(agent_name).is_some();

    let mut preview = PurgeReport::default();
    let mut roster_agent_id = None;
    let mut orphan_agent_ids = Vec::new();

    match entries.iter().find(|e| e.name == agent_name) {
        Some(entry) => {
            preview.roster_entry_removed = true;
            roster_agent_id = Some(entry.id);
        }
        None => {
            // Orphan path — the whole reason this module exists: the roster
            // entry is gone but its rows are not. Recover the agent id from
            // its name. Two sources: the canonical-UUID registry (covers
            // agents spawned with a random id) and the deterministic
            // name-derived UUID. A candidate that any live roster entry
            // holds (an agent renamed since spawn, keeping its id) is never
            // touched — its data belongs to a running agent.
            let mut candidates: Vec<AgentId> = Vec::new();
            if let Some(id) = registry.get(agent_name) {
                candidates.push(id);
            }
            let derived = AgentId::from_name(agent_name);
            if !candidates.contains(&derived) {
                candidates.push(derived);
            }
            for id in candidates {
                if entries.iter().any(|e| e.id == id) {
                    continue;
                }
                match has_agent_rows(substrate, &id) {
                    Ok(true) => {
                        preview.orphaned_data_removed = true;
                        orphan_agent_ids.push(id);
                    }
                    Ok(false) => {}
                    Err(e) => {
                        // Cannot verify this candidate is safe to cascade;
                        // stop rather than purge on a guess.
                        failures.push(e);
                        break;
                    }
                }
            }
        }
    }
    preview.identity_record_removed = registry_record;

    let workspace = home.join("workspaces").join("agents").join(agent_name);
    let workspace = workspace.is_dir().then_some(workspace);
    if workspace.is_some() {
        preview.workspace_removed = true;
    }
    let agent_type = agent_type_path_in(home, agent_name);
    let agent_type = agent_type.is_file().then_some(agent_type);
    if agent_type.is_some() {
        preview.agent_type_removed = true;
    }

    PurgePlan {
        preview,
        failures,
        roster_agent_id,
        orphan_agent_ids,
        registry_record,
        workspace,
        agent_type,
    }
}

/// Remove every trace of `agent_name`: roster entry (cascading to sessions,
/// memories and KV rows), any orphaned rows left by a previous partial
/// delete, the canonical-UUID registry record, the workspace directory, and
/// any agent-type template with the same name.
///
/// Idempotent by design: purging an agent that is already partly gone cleans
/// up whatever remains and reports it, rather than failing. That is the whole
/// point — the caller is here precisely because a previous delete left
/// something behind.
///
/// Never aborts halfway: every step runs (or is skipped because planning
/// could not prove it safe), and [`PurgeOutcome::failures`] lists everything
/// that went wrong. Rerun the command to finish a partial purge.
pub fn purge_agent(substrate: &MemorySubstrate, home: &Path, agent_name: &str) -> PurgeOutcome {
    let plan = plan_purge(substrate, home, agent_name);
    if !plan.failures.is_empty() {
        // Planning could not prove what is live; executing could cascade a
        // running agent's data. Surface the failure instead of guessing.
        return PurgeOutcome {
            report: PurgeReport::default(),
            failures: plan.failures,
        };
    }

    let mut report = PurgeReport::default();
    let mut failures = Vec::new();

    if let Some(id) = plan.roster_agent_id {
        match substrate.remove_agent(id) {
            Ok(()) => report.roster_entry_removed = true,
            Err(e) => failures.push(format!(
                "remove roster entry, sessions and memories for {id}: {e}"
            )),
        }
    }
    for id in &plan.orphan_agent_ids {
        match substrate.remove_agent(*id) {
            Ok(()) => report.orphaned_data_removed = true,
            Err(e) => failures.push(format!("remove orphaned rows for {id}: {e}")),
        }
    }

    // Drop the name → UUID binding last-but-not-unconditionally: the kernel
    // skips it when the roster row could not be removed (#5117) so the next
    // boot never loads a roster row whose name the registry no longer knows.
    // Same rule here: only unbind when every cascade above succeeded.
    if plan.registry_record && failures.is_empty() {
        let registry = AgentIdentityRegistry::load(home);
        if registry.purge(agent_name).is_some() {
            report.identity_record_removed = true;
        }
    }

    if let Some(workspace) = &plan.workspace {
        match std::fs::remove_dir_all(workspace) {
            Ok(()) => report.workspace_removed = true,
            Err(e) => failures.push(format!("remove workspace {}: {e}", workspace.display())),
        }
    }
    if let Some(agent_type) = &plan.agent_type {
        match std::fs::remove_file(agent_type) {
            Ok(()) => report.agent_type_removed = true,
            Err(e) => failures.push(format!("remove agent-type {}: {e}", agent_type.display())),
        }
    }

    PurgeOutcome { report, failures }
}

/// Whether any agent-scoped substrate row exists for `id` — the check that
/// separates "rows outlived the roster entry" from "the name is simply not
/// in this installation". Scans the same tables the substrate's `remove_agent`
/// cascade clears, so anything the scan can see, the cascade removes.
fn has_agent_rows(substrate: &MemorySubstrate, id: &AgentId) -> Result<bool, String> {
    let conn = substrate
        .pool()
        .get()
        .map_err(|e| format!("acquire database connection: {e}"))?;
    let id = id.0.to_string();
    for table in ["sessions", "canonical_sessions", "memories", "kv_store"] {
        let exists: bool = conn
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE agent_id = ?1)"),
                rusqlite::params![id],
                |row| row.get(0),
            )
            .map_err(|e| format!("scan {table} for rows of agent {id}: {e}"))?;
        if exists {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::agent::{AgentEntry, AgentState};
    use librefang_types::agent_type_store::agent_types_dir_in;

    fn home_with(agents: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for a in agents {
            let ws = dir.path().join("workspaces").join("agents").join(a);
            std::fs::create_dir_all(ws.join(".identity")).unwrap();
            std::fs::write(ws.join("agent.toml"), "x").unwrap();
            let types = agent_types_dir_in(dir.path());
            std::fs::create_dir_all(&types).unwrap();
            std::fs::write(types.join(format!("{a}.toml")), "x").unwrap();
        }
        dir
    }

    /// Seed a full agent footprint: roster entry, a session, a KV row and a
    /// memory row, all under `id`.
    fn seed_agent_rows(substrate: &MemorySubstrate, name: &str, id: AgentId) {
        let entry = AgentEntry {
            id,
            name: name.to_string(),
            state: AgentState::Running,
            ..Default::default()
        };
        substrate.save_agent(&entry).unwrap();
        substrate.create_session(id).unwrap();
        substrate
            .structured_set(id, "purge-test", serde_json::json!("seeded"))
            .unwrap();
        let conn = substrate.pool().get().unwrap();
        conn.execute(
            "INSERT INTO memories (id, agent_id, content, source, created_at, accessed_at) \
             VALUES ('purge-test-memory', ?1, 'remembered', 'test', datetime('now'), datetime('now'))",
            rusqlite::params![id.0.to_string()],
        )
        .unwrap();
    }

    /// Simulate the legacy partial delete this module exists for: the roster
    /// row goes, every other row stays.
    fn delete_roster_row_only(substrate: &MemorySubstrate, id: AgentId) {
        let conn = substrate.pool().get().unwrap();
        conn.execute(
            "DELETE FROM agents WHERE id = ?1",
            rusqlite::params![id.0.to_string()],
        )
        .unwrap();
    }

    fn orphan_row_count(substrate: &MemorySubstrate, id: AgentId) -> i64 {
        let conn = substrate.pool().get().unwrap();
        ["sessions", "memories", "kv_store"]
            .iter()
            .map(|table| {
                conn.query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE agent_id = ?1"),
                    rusqlite::params![id.0.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
            })
            .sum()
    }

    #[test]
    fn it_removes_the_workspace_and_the_agent_type() {
        let home = home_with(&["alpha", "beta"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();

        let outcome = purge_agent(&substrate, home.path(), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(outcome.report.workspace_removed);
        assert!(outcome.report.agent_type_removed);
        assert!(!home.path().join("workspaces/agents/alpha").exists());
        assert!(!agent_type_path_in(home.path(), "alpha").exists());
    }

    #[test]
    fn it_leaves_every_other_agent_alone() {
        let home = home_with(&["alpha", "beta"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();

        purge_agent(&substrate, home.path(), "alpha");

        assert!(home
            .path()
            .join("workspaces/agents/beta/agent.toml")
            .exists());
        assert!(agent_type_path_in(home.path(), "beta").exists());
    }

    #[test]
    fn purging_something_already_gone_is_not_an_error() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();

        let outcome = purge_agent(&substrate, home.path(), "nobody");

        assert!(outcome.failures.is_empty());
        assert!(outcome.report.is_empty());
    }

    #[test]
    fn a_path_shaped_name_never_reaches_the_filesystem() {
        let home = home_with(&["alpha"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();

        for evil in ["../../etc", "a/b", "..", ""] {
            assert!(
                purge_agent(&substrate, home.path(), evil)
                    .failures
                    .iter()
                    .any(|f| f.contains("invalid agent name")),
                "{evil:?} must be rejected before any join"
            );
        }
        assert!(home.path().join("workspaces/agents/alpha").exists());
    }

    /// THE headline case: the roster entry is already gone but its session,
    /// memory and KV rows are not. Purge-by-name must find the id and
    /// cascade the orphans away, not report "nothing to purge".
    #[test]
    fn purge_by_name_cleans_rows_that_outlived_the_roster_entry() {
        let home = home_with(&["alpha"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let id = AgentId::from_name("alpha");
        seed_agent_rows(&substrate, "alpha", id);
        delete_roster_row_only(&substrate, id);
        assert!(orphan_row_count(&substrate, id) > 0, "seed failed");

        let outcome = purge_agent(&substrate, home.path(), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(outcome.report.orphaned_data_removed);
        assert_eq!(
            orphan_row_count(&substrate, id),
            0,
            "orphaned rows survived"
        );
    }

    /// The deterministic name derivation is not the only id source: an agent
    /// spawned with a random UUID leaves a registry record behind. That
    /// record must lead the purge back to the orphaned rows too.
    #[test]
    fn orphan_rows_are_found_through_the_identity_registry_even_for_random_ids() {
        let home = home_with(&["alpha"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let id = AgentId::new();
        seed_agent_rows(&substrate, "alpha", id);
        delete_roster_row_only(&substrate, id);
        let registry = AgentIdentityRegistry::load(home.path());
        registry.register_if_absent("alpha", id);

        let outcome = purge_agent(&substrate, home.path(), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(outcome.report.orphaned_data_removed);
        assert!(outcome.report.identity_record_removed);
        assert_eq!(
            orphan_row_count(&substrate, id),
            0,
            "orphaned rows survived"
        );
        assert!(AgentIdentityRegistry::load(home.path())
            .get("alpha")
            .is_none());
    }

    /// An agent spawned as "alpha" and later renamed to "beta" keeps its id.
    /// Purging the stale name "alpha" must not cascade the live agent's rows.
    #[test]
    fn a_live_agents_id_is_never_treated_as_an_orphan() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        let id = AgentId::from_name("alpha");
        seed_agent_rows(&substrate, "beta", id);

        let outcome = purge_agent(&substrate, home.path(), "alpha");

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(!outcome.report.orphaned_data_removed);
        assert!(outcome.report.is_empty());
        assert_eq!(
            orphan_row_count(&substrate, id),
            3,
            "live agent's rows must survive"
        );
    }

    #[test]
    fn dry_run_plan_matches_what_the_real_purge_removes() {
        let home = home_with(&["alpha"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();
        seed_agent_rows(&substrate, "alpha", AgentId::from_name("alpha"));

        let plan = plan_purge(&substrate, home.path(), "alpha");
        assert!(plan.failures.is_empty(), "{:?}", plan.failures);
        assert!(plan.preview.roster_entry_removed);
        assert!(plan.preview.workspace_removed);
        assert!(plan.preview.agent_type_removed);
        assert!(!plan.preview.identity_record_removed);
        assert!(plan.roster_agent_id.is_some());
        assert!(plan.workspace.is_some());
        assert!(plan.agent_type.is_some());

        // Planning itself must not have touched anything.
        assert!(substrate
            .load_all_agents()
            .unwrap()
            .iter()
            .any(|e| e.name == "alpha"));
        assert!(agent_type_path_in(home.path(), "alpha").exists());

        let outcome = purge_agent(&substrate, home.path(), "alpha");
        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert_eq!(outcome.report, plan.preview);
    }
}
