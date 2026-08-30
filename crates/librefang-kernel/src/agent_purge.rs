//! Purging every trace of an agent — shared by the CLI, the API, the
//! dashboard and the TUI.
//!
//! Deleting an agent removes it from the roster and stops it running, but
//! leaves its workspace directory and any agent-type of the same name on
//! disk, and (for an agent whose roster entry is already gone) its sessions
//! and memories in the database with nothing left pointing at them. This
//! module is the one place that cleans all of it.
//!
//! Deliberately a free function rather than a `Kernel` method: the CLI purges
//! without a running daemon by opening the database directly, and hanging
//! this off the kernel would force it to boot one.

use librefang_memory::MemorySubstrate;
use std::path::Path;

/// What a purge actually removed. Every field is what happened, not what was
/// attempted, so a caller can report the truth rather than an intention.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct PurgeReport {
    /// The agent had a roster entry, and it (plus its sessions, memories and
    /// KV rows, via the substrate's cascade) was removed.
    pub roster_entry_removed: bool,
    /// The agent's workspace directory was deleted.
    pub workspace_removed: bool,
    /// An agent-type template of the same name was deleted.
    pub agent_type_removed: bool,
}

impl PurgeReport {
    /// True when nothing at all was found to remove — the caller asked to
    /// purge something that leaves no trace anywhere.
    pub fn is_empty(&self) -> bool {
        !self.roster_entry_removed && !self.workspace_removed && !self.agent_type_removed
    }
}

/// Remove every trace of `agent_name`: roster entry (cascading to sessions,
/// memories and KV rows), workspace directory, and any agent-type template
/// with the same name.
///
/// Idempotent by design: purging an agent that is already partly gone cleans
/// up whatever remains and reports it, rather than failing. That is the whole
/// point — the caller is here precisely because a previous delete left
/// something behind.
pub fn purge_agent(
    substrate: &MemorySubstrate,
    home: &Path,
    agent_name: &str,
) -> Result<PurgeReport, String> {
    if agent_name.trim().is_empty()
        || agent_name.contains('/')
        || agent_name.contains('\\')
        || agent_name.contains("..")
    {
        return Err(format!("invalid agent name: {agent_name:?}"));
    }

    let mut report = PurgeReport::default();

    let entries = substrate
        .load_all_agents()
        .map_err(|e| format!("read roster: {e}"))?;
    if let Some(entry) = entries.iter().find(|e| e.name == agent_name) {
        substrate
            .remove_agent(entry.id)
            .map_err(|e| format!("remove roster entry, sessions and memories: {e}"))?;
        report.roster_entry_removed = true;
    }

    let workspace = home.join("workspaces").join("agents").join(agent_name);
    if workspace.is_dir() {
        std::fs::remove_dir_all(&workspace)
            .map_err(|e| format!("remove workspace {}: {e}", workspace.display()))?;
        report.workspace_removed = true;
    }

    let type_file = home
        .join("workspaces")
        .join("agent-types")
        .join(format!("{agent_name}.toml"));
    if type_file.is_file() {
        std::fs::remove_file(&type_file)
            .map_err(|e| format!("remove agent-type {}: {e}", type_file.display()))?;
        report.agent_type_removed = true;
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home_with(agents: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for a in agents {
            let ws = dir.path().join("workspaces").join("agents").join(a);
            std::fs::create_dir_all(ws.join(".identity")).unwrap();
            std::fs::write(ws.join("agent.toml"), "x").unwrap();
            let types = dir.path().join("workspaces").join("agent-types");
            std::fs::create_dir_all(&types).unwrap();
            std::fs::write(types.join(format!("{a}.toml")), "x").unwrap();
        }
        dir
    }

    #[test]
    fn it_removes_the_workspace_and_the_agent_type() {
        let home = home_with(&["alpha", "beta"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();

        let report = purge_agent(&substrate, home.path(), "alpha").unwrap();

        assert!(report.workspace_removed);
        assert!(report.agent_type_removed);
        assert!(!home.path().join("workspaces/agents/alpha").exists());
        assert!(!home
            .path()
            .join("workspaces/agent-types/alpha.toml")
            .exists());
    }

    #[test]
    fn it_leaves_every_other_agent_alone() {
        let home = home_with(&["alpha", "beta"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();

        purge_agent(&substrate, home.path(), "alpha").unwrap();

        assert!(home
            .path()
            .join("workspaces/agents/beta/agent.toml")
            .exists());
        assert!(home
            .path()
            .join("workspaces/agent-types/beta.toml")
            .exists());
    }

    #[test]
    fn purging_something_already_gone_is_not_an_error() {
        let home = home_with(&[]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();

        let report = purge_agent(&substrate, home.path(), "nobody").unwrap();

        assert!(report.is_empty());
    }

    #[test]
    fn a_path_shaped_name_never_reaches_the_filesystem() {
        let home = home_with(&["alpha"]);
        let substrate = MemorySubstrate::open_in_memory(0.01).unwrap();

        for evil in ["../../etc", "a/b", "..", ""] {
            assert!(
                purge_agent(&substrate, home.path(), evil).is_err(),
                "{evil:?} must be rejected before any join"
            );
        }
        assert!(home.path().join("workspaces/agents/alpha").exists());
    }
}
