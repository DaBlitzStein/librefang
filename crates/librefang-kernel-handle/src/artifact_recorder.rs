// ============================================================================
// 21. ArtifactRecorder — session-scoped capture of files an agent's turn
//     wrote into its workspace, backed by the content-addressed artifact
//     store (`[artifacts]`, schema v66).
//
//     Two callers, two shapes:
//       * `record_artifact` — the runtime captures the result of a file
//         tool it just ran, while it still holds the bytes.
//       * `sweep_workspace_artifacts` — the agent loop, at the end of a turn
//         in which a shell/command tool ran, asks the kernel to walk the
//         workspace for anything modified since the turn started. Only that
//         path can see files produced by `echo > f`, `curl -o`, scripts.
//
//     Both default to a no-op so test doubles and stub kernels that predate
//     this capability keep compiling and behave exactly as before.
// ============================================================================

use super::*;
use librefang_types::agent::SessionId;
use std::path::Path;
use std::time::SystemTime;

pub trait ArtifactRecorder: Send + Sync {
    /// Record `bytes` as the next version of `path` for `session_id`.
    ///
    /// `path` must already be workspace-relative and normalised by the
    /// caller; this layer stores it verbatim. Message attribution is not
    /// wired yet, so every row carries a `NULL` `message_id`.
    ///
    /// Blocking I/O (hashing, blob write, SQLite insert). Callers already on
    /// a filesystem-write path may call it directly; a sweep that records
    /// many files should run on a blocking thread.
    fn record_artifact(
        &self,
        _session_id: SessionId,
        _path: &str,
        _bytes: &[u8],
        _mime_type: Option<&str>,
    ) -> KernelResult<()> {
        Ok(())
    }

    /// Record every regular file under `workspace_root` modified at or after
    /// `since`, skipping VCS/build directories and oversized files and
    /// capping how many files one sweep records.
    ///
    /// Returns the number of files recorded. A partial failure (one
    /// unreadable file) is logged and does not abort the sweep.
    ///
    /// Blocking I/O: walks the tree and writes blobs + rows.
    fn sweep_workspace_artifacts(
        &self,
        _session_id: SessionId,
        _workspace_root: &Path,
        _since: SystemTime,
    ) -> KernelResult<usize> {
        Ok(0)
    }
}
