//! Session artifact capture (plan task T3.3).
//!
//! Two layers, per operator decision D4:
//!
//! 1. **Immediate capture.** A successful `file_write` / `apply_patch` /
//!    `web_fetch_to_file` hands the resulting bytes to the kernel's
//!    [`ArtifactRecorder`] for the current session. The tool already holds
//!    (or just wrote) the bytes, so this costs one hash + blob write.
//! 2. **End-of-turn sweep.** If any shell/command tool was *dispatched*
//!    during the turn, the agent loop asks the kernel to walk the workspace
//!    for files modified at or after the turn start. That catches
//!    `echo > f`, `curl -o`, and anything a script wrote. Turns that ran no
//!    command skip the walk entirely, which keeps the common path cheap.
//!
//! Container / mount interception is out of scope here: a file written inside
//! a container filesystem never becomes visible to this layer. Follow-up.

use crate::kernel_handle::prelude::*;
use librefang_types::agent::SessionId;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime};

/// mtime slack subtracted from the turn start before comparing.
///
/// Filesystem timestamp granularity (and a file written microseconds before
/// the loop's own `SystemTime::now`) can put a genuinely new file just short
/// of the boundary; two seconds covers that without reaching a previous turn.
const SWEEP_SLACK: Duration = Duration::from_secs(2);

/// Largest file the immediate path records.
///
/// shortcut: one flat cap; the immediate path holds the file in memory
/// already, so this only stops a pathologically large `file_write` from
/// turning into a second large allocation in the store.
const MAX_IMMEDIATE_CAPTURE_BYTES: u64 = 50 * 1024 * 1024;

/// Sessions whose current turn dispatched a shell/command tool.
///
/// Keyed by session rather than carried as a task-local because wrapping the
/// whole agent-loop future in another `.scope()` pushed a downstream test's
/// async-block layout past rustc's recursion limit. The kernel serialises
/// turns per agent/session, so one flag per session is enough to answer "did
/// this turn run a command?"; an overlapping turn on the same session can at
/// worst cause one extra (idempotent) sweep.
///
/// Fork hazard: a forked child that shares the parent's session id can
/// consume — or be observed by — the parent's flag. Low likelihood in practice
/// because the sweep is the only consumer, and a sweep is idempotent; both
/// parents and children only need *a* sweep after a command, not their own.
fn shell_turns() -> &'static Mutex<HashSet<SessionId>> {
    static REG: OnceLock<Mutex<HashSet<SessionId>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashSet::new()))
}

fn recover_lock<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("artifact capture registry lock poisoned; recovering state");
            let guard = poisoned.into_inner();
            lock.clear_poison();
            guard
        }
    }
}

/// Mark the current turn as having run a command.
///
/// Called on *dispatch*, not success: a command that failed may still have
/// written a file before failing, and the sweep is the only thing that sees
/// it. A tool call with no session id (the MCP bridge) is ignored.
///
/// A deferred-approval resume restores the originating session id, so it *does*
/// set this flag — but no agent-loop end follows that dispatch, so nothing
/// consumes it until the next turn on that session. That turn sweeps from its
/// own start, which is later than the resume, so files the approved command
/// wrote are not seen.
///
/// shortcut: shell writes from a deferred resume are not captured; carry a
/// turn id alongside the flag if that ever matters.
///
/// Incognito turns never reach this function: dispatch passes
/// `capture_artifacts = false` for them.
pub fn note_shell_tool(session_id: Option<SessionId>) {
    if let Some(session_id) = session_id {
        recover_lock(shell_turns()).insert(session_id);
    }
}

/// Consume the "a command ran" flag for `session_id`.
///
/// Called once at end of turn; the flag does not leak into the next one.
pub fn take_shell_ran(session_id: SessionId) -> bool {
    recover_lock(shell_turns()).remove(&session_id)
}

/// Tools whose dispatch means "a command may have written into the workspace".
pub fn is_shell_tool(tool_name: &str) -> bool {
    matches!(tool_name, "shell_exec" | "process_start" | "docker_exec")
}

/// End-of-turn sweep, when the turn ran a command.
///
/// Runs the blocking kernel-side walk on a blocking thread so a large
/// workspace does not park an async worker.
pub async fn sweep_turn_artifacts(
    kernel: &Arc<dyn KernelHandle>,
    session_id: SessionId,
    workspace_root: &Path,
    turn_started: SystemTime,
    shell_ran: bool,
) {
    if !shell_ran {
        return;
    }
    let since = turn_started
        .checked_sub(SWEEP_SLACK)
        .unwrap_or(turn_started);
    let kernel = Arc::clone(kernel);
    let root = workspace_root.to_path_buf();
    let joined = tokio::task::spawn_blocking(move || {
        kernel.sweep_workspace_artifacts(session_id, &root, since)
    })
    .await;
    match joined {
        Ok(Ok(recorded)) => {
            tracing::debug!(recorded, "artifact sweep finished");
        }
        Ok(Err(e)) => tracing::warn!(error = %e, "artifact sweep failed"),
        Err(e) => tracing::warn!(error = %e, "artifact sweep task failed"),
    }
}

/// Record bytes a file tool just wrote.
///
/// `raw_path` is the model-supplied path; it is resolved through the same
/// workspace sandbox the tool used, and anything that escapes (or resolves
/// into a named workspace outside the primary root, which has no
/// workspace-relative spelling) is skipped rather than recorded under a
/// misleading path.
pub fn capture_written_bytes(
    kernel: Option<&Arc<dyn KernelHandle>>,
    session_id: Option<SessionId>,
    workspace_root: Option<&Path>,
    raw_path: Option<&str>,
    additional_roots: &[&Path],
    bytes: &[u8],
) {
    let (Some(kernel), Some(session_id), Some(root), Some(raw_path)) =
        (kernel, session_id, workspace_root, raw_path)
    else {
        return;
    };
    let Ok(resolved) =
        crate::workspace_sandbox::resolve_sandbox_path_ext(raw_path, root, additional_roots)
    else {
        return;
    };
    record_resolved(kernel, session_id, root, &resolved, bytes);
}

/// Record a file tool's target by reading it back from disk.
///
/// Used where the tool only returns a summary (`apply_patch`,
/// `web_fetch_to_file`). A file that was deleted, is not a regular file, or
/// exceeds the immediate cap is skipped silently.
pub fn capture_file_from_disk(
    kernel: Option<&Arc<dyn KernelHandle>>,
    session_id: Option<SessionId>,
    workspace_root: Option<&Path>,
    raw_path: Option<&str>,
    additional_roots: &[&Path],
) {
    let (Some(kernel), Some(session_id), Some(root), Some(raw_path)) =
        (kernel, session_id, workspace_root, raw_path)
    else {
        return;
    };
    let Ok(resolved) =
        crate::workspace_sandbox::resolve_sandbox_path_ext(raw_path, root, additional_roots)
    else {
        return;
    };
    let Ok(metadata) = std::fs::metadata(&resolved) else {
        return;
    };
    if !metadata.is_file() || metadata.len() > MAX_IMMEDIATE_CAPTURE_BYTES {
        return;
    }
    let Ok(bytes) = std::fs::read(&resolved) else {
        return;
    };
    record_resolved(kernel, session_id, root, &resolved, &bytes);
}

/// Record the files an `apply_patch` run touched, after it succeeded.
///
/// `DeleteFile` has no bytes left to record. `UpdateFile` with a `move_to`
/// is recorded at its destination; the old path no longer exists and its
/// previous content is already in the store if it had been captured before.
pub fn capture_patch_targets(
    kernel: Option<&Arc<dyn KernelHandle>>,
    session_id: Option<SessionId>,
    workspace_root: Option<&Path>,
    patch: Option<&str>,
    additional_roots: &[&Path],
) {
    let Some(patch) = patch else {
        return;
    };
    let Ok(ops) = crate::apply_patch::parse_patch(patch) else {
        return;
    };
    for op in ops {
        match op {
            crate::apply_patch::PatchOp::AddFile { path, .. } => capture_file_from_disk(
                kernel,
                session_id,
                workspace_root,
                Some(&path),
                additional_roots,
            ),
            crate::apply_patch::PatchOp::UpdateFile { path, move_to, .. } => {
                if let Some(dest) = move_to {
                    capture_file_from_disk(
                        kernel,
                        session_id,
                        workspace_root,
                        Some(&dest),
                        additional_roots,
                    );
                } else {
                    capture_file_from_disk(
                        kernel,
                        session_id,
                        workspace_root,
                        Some(&path),
                        additional_roots,
                    );
                }
            }
            crate::apply_patch::PatchOp::DeleteFile { .. } => {}
        }
    }
}

fn record_resolved(
    kernel: &Arc<dyn KernelHandle>,
    session_id: SessionId,
    workspace_root: &Path,
    resolved: &Path,
    bytes: &[u8],
) {
    if bytes.len() as u64 > MAX_IMMEDIATE_CAPTURE_BYTES {
        return;
    }
    let Some(relative) = librefang_memory::artifact_store::workspace_relative_path_for_resolved(
        workspace_root,
        resolved,
    ) else {
        // shortcut: only paths under the primary workspace root have a stable
        // workspace-relative spelling. A write into a named workspace is
        // skipped until artifact paths grow a mount-qualified form.
        return;
    };
    if let Err(e) = kernel.record_artifact(session_id, &relative, bytes, None) {
        tracing::warn!(path = %relative, error = %e, "failed to record session artifact");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_tools_are_recognised() {
        for name in ["shell_exec", "process_start", "docker_exec"] {
            assert!(is_shell_tool(name), "{name} must trigger a sweep");
        }
        for name in ["file_write", "file_read", "apply_patch", "web_fetch"] {
            assert!(!is_shell_tool(name), "{name} must not trigger a sweep");
        }
    }

    #[test]
    fn shell_flag_is_per_session_and_consumed_once() {
        let first = SessionId::new();
        let second = SessionId::new();

        assert!(!take_shell_ran(first), "a fresh session has no flag");
        note_shell_tool(Some(first));
        assert!(take_shell_ran(first), "the turn's own flag must be seen");
        assert!(
            !take_shell_ran(first),
            "taking the flag must clear it so the next turn does not sweep"
        );
        assert!(
            !take_shell_ran(second),
            "another session's flag must not leak"
        );
    }

    #[test]
    fn note_shell_tool_without_a_session_is_a_no_op() {
        // Must not panic when a tool runs outside a session-bound turn (MCP
        // bridge, deferred approval resume).
        note_shell_tool(None);
    }
}
