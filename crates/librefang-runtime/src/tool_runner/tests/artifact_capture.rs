//! T3.3 artifact capture through the real dispatcher.
//!
//! `DispatchCapture` stands in for the kernel and records what the runtime
//! hands to `ArtifactRecorder`; the end-of-turn sweep records that it was
//! asked to walk a workspace.

use super::*;
use crate::artifact_capture::{sweep_turn_artifacts, take_shell_ran};
use librefang_types::agent::SessionId;
use librefang_types::tool::ToolResult;
use std::path::Path;
use std::time::SystemTime;

#[allow(clippy::too_many_arguments)]
async fn call_tool(
    tool_name: &str,
    input: serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
    session: Option<SessionId>,
    exec_policy: Option<&librefang_types::config::ExecPolicy>,
) -> ToolResult {
    let session = session.map(|id| id.0.to_string());
    execute_tool(
        "capture-id",
        tool_name,
        &input,
        kernel,
        None,               // allowed_tools
        Some("agent-1"),    // caller_agent_id
        None,               // skill_registry
        None,               // allowed_skills
        None,               // mcp_connections
        None,               // web_ctx
        None,               // browser_ctx
        None,               // allowed_env_vars
        workspace_root,     // workspace_root
        None,               // media_engine
        None,               // media_drivers
        exec_policy,        // exec_policy
        None,               // tts_engine
        None,               // tts_config
        None,               // docker_config
        None,               // process_manager
        None,               // process_registry
        None,               // sender_id
        None,               // channel
        None,               // chat_id
        None,               // checkpoint_manager
        None,               // interrupt
        session.as_deref(), // session_id
        None,               // dangerous_command_checker
        None,               // available_tools
        0,                  // spill_threshold_bytes
        0,                  // max_artifact_bytes
    )
    .await
}

#[tokio::test]
async fn successful_file_write_is_recorded_with_exact_bytes_and_relative_path() {
    let workspace = tempfile::tempdir().unwrap();
    let capture = Arc::new(DispatchCapture::default());
    let kernel: Arc<dyn KernelHandle> = capture.clone();
    let session = SessionId::new();

    let result = call_tool(
        "file_write",
        serde_json::json!({"path": "notes/a.md", "content": "hello\n"}),
        Some(&kernel),
        Some(workspace.path()),
        Some(session),
        None,
    )
    .await;
    assert!(!result.is_error, "{}", result.content);

    let artifacts = capture.artifacts.lock().unwrap();
    assert_eq!(artifacts.len(), 1, "one successful write → one artifact");
    let (recorded_session, path, bytes) = &artifacts[0];
    assert_eq!(*recorded_session, session);
    assert_eq!(path, "notes/a.md");
    assert_eq!(bytes, b"hello\n", "the store must receive the exact bytes");
}

#[tokio::test]
async fn failed_file_write_is_not_recorded() {
    let workspace = tempfile::tempdir().unwrap();
    // A directory at the target path makes the final rename fail.
    std::fs::create_dir(workspace.path().join("sub")).unwrap();
    let capture = Arc::new(DispatchCapture::default());
    let kernel: Arc<dyn KernelHandle> = capture.clone();

    let result = call_tool(
        "file_write",
        serde_json::json!({"path": "sub", "content": "x"}),
        Some(&kernel),
        Some(workspace.path()),
        Some(SessionId::new()),
        None,
    )
    .await;
    assert!(result.is_error, "{:?}", result.content);

    assert!(
        capture.artifacts.lock().unwrap().is_empty(),
        "a failed write must not leave a misleading artifact version"
    );
}

#[tokio::test]
async fn successful_apply_patch_records_each_written_file() {
    let workspace = tempfile::tempdir().unwrap();
    let capture = Arc::new(DispatchCapture::default());
    let kernel: Arc<dyn KernelHandle> = capture.clone();
    let session = SessionId::new();

    let result = call_tool(
        "apply_patch",
        serde_json::json!({
            "patch": "*** Begin Patch\n*** Add File: a.txt\n+one\n*** Add File: sub/b.txt\n+two\n*** End Patch\n"
        }),
        Some(&kernel),
        Some(workspace.path()),
        Some(session),
        None,
    )
    .await;
    assert!(!result.is_error, "{}", result.content);

    let artifacts = capture.artifacts.lock().unwrap();
    let mut paths: Vec<&str> = artifacts.iter().map(|(_, path, _)| path.as_str()).collect();
    paths.sort();
    assert_eq!(paths, ["a.txt", "sub/b.txt"]);
    let mut contents: Vec<&[u8]> = artifacts
        .iter()
        .map(|(_, _, bytes)| bytes.as_slice())
        .collect();
    contents.sort();
    assert_eq!(contents, [b"one".as_slice(), b"two".as_slice()]);
}

/// Editor stand-in: records what the ACP buffer write was asked to hold.
#[derive(Default)]
struct FakeEditor {
    written: std::sync::Mutex<Vec<(std::path::PathBuf, String)>>,
}

#[async_trait::async_trait]
impl librefang_kernel_handle::AcpFsClient for FakeEditor {
    async fn read_text_file(
        &self,
        _path: std::path::PathBuf,
        _line: Option<u32>,
        _limit: Option<u32>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn write_text_file(
        &self,
        path: std::path::PathBuf,
        content: String,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        self.written.lock().unwrap().push((path, content));
        Ok(())
    }

    fn capabilities(&self) -> (bool, bool) {
        (true, true)
    }
}

#[tokio::test]
async fn editor_buffer_write_is_recorded_even_before_save() {
    let workspace = tempfile::tempdir().unwrap();
    let editor = Arc::new(FakeEditor::default());
    let capture = Arc::new(DispatchCapture {
        acp_client: Some(editor.clone()),
        ..Default::default()
    });
    let kernel: Arc<dyn KernelHandle> = capture.clone();
    let session = SessionId::new();

    let result = call_tool(
        "file_write",
        serde_json::json!({"path": "buffer.md", "content": "hi\n"}),
        Some(&kernel),
        Some(workspace.path()),
        Some(session),
        None,
    )
    .await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        editor.written.lock().unwrap().as_slice(),
        &[(std::path::PathBuf::from("buffer.md"), "hi\n".to_string())]
    );
    assert!(
        !workspace.path().join("buffer.md").exists(),
        "the fake editor holds the buffer in memory; this is the no-disk case"
    );

    let artifacts = capture.artifacts.lock().unwrap();
    assert_eq!(artifacts.len(), 1, "the buffer content must be recorded");
    assert_eq!(artifacts[0].1, "buffer.md");
    assert_eq!(artifacts[0].2, b"hi\n");
}

/// A turn that only ran a non-command tool must not walk the workspace.
#[tokio::test]
async fn sweep_is_not_requested_when_no_shell_tool_ran() {
    let workspace = tempfile::tempdir().unwrap();
    let capture = Arc::new(DispatchCapture::default());
    let kernel: Arc<dyn KernelHandle> = capture.clone();
    let session = SessionId::new();
    let started = SystemTime::now();

    let result = call_tool(
        "file_write",
        serde_json::json!({"path": "a.txt", "content": "x"}),
        Some(&kernel),
        Some(workspace.path()),
        Some(session),
        None,
    )
    .await;
    assert!(!result.is_error, "{}", result.content);

    let shell_ran = take_shell_ran(session);
    assert!(
        !shell_ran,
        "a non-command tool must not arm the end-of-turn sweep"
    );
    sweep_turn_artifacts(&kernel, session, workspace.path(), started, shell_ran).await;
    assert!(
        capture.sweeps.lock().unwrap().is_empty(),
        "no command ran, so the workspace must not be walked"
    );
}

/// A command tool — even one whose output only reaches the disk through a
/// shell redirect — turns the sweep on for the turn.
#[tokio::test]
async fn shell_exec_marks_the_turn_and_requests_the_sweep() {
    let workspace = tempfile::tempdir().unwrap();
    let capture = Arc::new(DispatchCapture::default());
    let kernel: Arc<dyn KernelHandle> = capture.clone();
    let session = SessionId::new();
    let started = SystemTime::now();
    let full_exec = librefang_types::config::ExecPolicy {
        mode: librefang_types::config::ExecSecurityMode::Full,
        ..Default::default()
    };

    let result = call_tool(
        "shell_exec",
        serde_json::json!({"command": "echo hi > made.txt"}),
        Some(&kernel),
        Some(workspace.path()),
        Some(session),
        Some(&full_exec),
    )
    .await;
    assert!(!result.is_error, "{}", result.content);

    let shell_ran = take_shell_ran(session);
    assert!(shell_ran, "a dispatched shell tool must arm the sweep");
    assert!(
        workspace.path().join("made.txt").exists(),
        "the shell command must actually have written its file"
    );
    sweep_turn_artifacts(&kernel, session, workspace.path(), started, shell_ran).await;
    assert_eq!(
        capture.sweeps.lock().unwrap().as_slice(),
        &[session],
        "the turn's session must be the one swept"
    );
}
