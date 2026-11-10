//! [`kernel_handle::ArtifactRecorder`] — session artifact capture backed by
//! the substrate's content-addressed `ArtifactStore`.
//!
//! The store is attached at boot from `[artifacts]`; when it is absent (test
//! kernels, embedders that never call `attach_artifact_store`) capture is a
//! silent no-op rather than an error.

use librefang_memory::artifact_store::{SWEEP_MAX_FILES, SWEEP_MAX_FILE_BYTES};
use librefang_runtime::kernel_handle;
use librefang_types::agent::SessionId;
use std::path::Path;
use std::time::SystemTime;

use super::super::LibreFangKernel;

impl kernel_handle::ArtifactRecorder for LibreFangKernel {
    fn record_artifact(
        &self,
        session_id: SessionId,
        path: &str,
        bytes: &[u8],
        mime_type: Option<&str>,
    ) -> kernel_handle::KernelResult<()> {
        let Some(store) = self.memory.substrate.artifacts() else {
            return Ok(());
        };
        // No message id is available at the tool-dispatch boundary, and an
        // invented one would look like real attribution later.
        store.put(&session_id.0.to_string(), None, path, bytes, mime_type)?;
        Ok(())
    }

    fn sweep_workspace_artifacts(
        &self,
        session_id: SessionId,
        workspace_root: &Path,
        since: SystemTime,
    ) -> kernel_handle::KernelResult<usize> {
        let Some(store) = self.memory.substrate.artifacts() else {
            return Ok(0);
        };
        let report = store.record_workspace_sweep(
            &session_id.0.to_string(),
            workspace_root,
            since,
            SWEEP_MAX_FILES,
            SWEEP_MAX_FILE_BYTES,
        )?;
        Ok(report.recorded)
    }
}
