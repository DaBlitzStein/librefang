//! T3.3 at the loop boundary.
//!
//! What each test pins:
//!
//! - `normal_turn_records_a_file_write_as_an_artifact` — the loop's immediate
//!   capture path reaches the recorder with the exact bytes and path.
//! - `incognito_turn_records_nothing` — `LoopOptions::incognito` disables that
//!   path while the tool itself still runs.
//! - `shell_turn_requests_a_sweep_and_incognito_does_not` — the end-of-turn
//!   hook calls the recorder's sweep exactly when a command ran and the turn
//!   is not incognito.
//! - `incognito_loop_end_consumes_a_seeded_marker_without_sweeping` — an
//!   incognito end consumes a marker that is already set without sweeping, so
//!   a stale flag cannot leak into a later turn.
//!
//! Deleting the sweep hook fails the third. The dispatch-level
//! `capture_artifacts = false` guard on the marker itself is pinned in
//! `tool_runner::tests::artifact_capture`.

use super::*;
use librefang_kernel_handle::ApiAuthSnapshot;
use librefang_types::agent::SessionId;
use librefang_types::config::{ExecPolicy, ExecSecurityMode};
use librefang_types::message::{ContentBlock, StopReason, TokenUsage};
use std::sync::Mutex;

/// Kernel handle whose only opinion is what the recorder was asked to do.
struct CaptureKernel {
    artifacts: Mutex<Vec<(SessionId, String, Vec<u8>)>>,
    sweeps: Mutex<Vec<SessionId>>,
}

impl Default for CaptureKernel {
    fn default() -> Self {
        Self {
            artifacts: Mutex::new(Vec::new()),
            sweeps: Mutex::new(Vec::new()),
        }
    }
}

impl librefang_kernel_handle::ArtifactRecorder for CaptureKernel {
    fn record_artifact(
        &self,
        session_id: SessionId,
        path: &str,
        bytes: &[u8],
        _mime_type: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        self.artifacts
            .lock()
            .unwrap()
            .push((session_id, path.to_string(), bytes.to_vec()));
        Ok(())
    }

    fn sweep_workspace_artifacts(
        &self,
        session_id: SessionId,
        _workspace_root: &std::path::Path,
        _since: std::time::SystemTime,
    ) -> Result<usize, librefang_kernel_handle::KernelOpError> {
        self.sweeps.lock().unwrap().push(session_id);
        Ok(1)
    }
}

/// One tool call, then a plain answer.
struct ToolThenTextDriver {
    calls: AtomicU32,
    tool_name: &'static str,
    input: serde_json::Value,
}

impl ToolThenTextDriver {
    fn new(tool_name: &'static str, input: serde_json::Value) -> Self {
        Self {
            calls: AtomicU32::new(0),
            tool_name,
            input,
        }
    }
}

#[async_trait]
impl LlmDriver for ToolThenTextDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let usage = TokenUsage {
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        };
        if self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
            Ok(CompletionResponse {
                text_synthesized_from_thinking: false,
                content: vec![ContentBlock::ToolUse {
                    id: "t1".to_string(),
                    name: self.tool_name.to_string(),
                    input: self.input.clone(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ToolCall {
                    id: "t1".to_string(),
                    name: self.tool_name.to_string(),
                    input: self.input.clone(),
                }],
                usage,
                actual_provider: None,
                actual_model: None,
            })
        } else {
            Ok(CompletionResponse {
                text_synthesized_from_thinking: false,
                content: vec![ContentBlock::Text {
                    text: "done".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage,
                actual_provider: None,
                actual_model: None,
            })
        }
    }
}

fn manifest_with_exec() -> AgentManifest {
    let mut manifest = super::integration::test_manifest();
    manifest.exec_policy = Some(ExecPolicy {
        mode: ExecSecurityMode::Full,
        ..ExecPolicy::default()
    });
    manifest
}

async fn run_one_turn(
    kernel: &Arc<dyn librefang_kernel_handle::KernelHandle>,
    workspace: &std::path::Path,
    driver: ToolThenTextDriver,
    tool_name: &str,
    incognito: bool,
    mut session: librefang_memory::session::Session,
) -> librefang_memory::session::Session {
    let memory = librefang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let manifest = manifest_with_exec();
    let tool = fake_tool(tool_name);
    let opts = LoopOptions {
        incognito,
        ..LoopOptions::default()
    };

    run_agent_loop(
        &manifest,
        "do the thing",
        &mut session,
        &memory,
        Arc::new(driver),
        std::slice::from_ref(&tool),
        Some(Arc::clone(kernel)),
        None,
        None,
        None,
        None,
        None,
        Some(workspace),
        None, // on_phase
        None, // media_engine
        None, // media_drivers
        None, // tts_engine
        None, // docker_config
        None, // hooks
        None, // context_window_tokens
        None, // process_manager
        None, // checkpoint_manager
        None, // process_registry
        None, // user_content_blocks
        None, // proactive_memory
        None, // context_engine
        None, // pending_messages
        &opts,
    )
    .await
    .expect("the scripted turn should complete");

    session
}

#[tokio::test]
async fn normal_turn_records_a_file_write_as_an_artifact() {
    let workspace = tempfile::tempdir().unwrap();
    let capture = Arc::new(CaptureKernel::default());
    let kernel: Arc<dyn librefang_kernel_handle::KernelHandle> = capture.clone();
    let driver = ToolThenTextDriver::new(
        "file_write",
        serde_json::json!({"path": "notes/a.md", "content": "hi\n"}),
    );

    let session = run_one_turn(
        &kernel,
        workspace.path(),
        driver,
        "file_write",
        false,
        super::integration::fresh_session(),
    )
    .await;

    let artifacts = capture.artifacts.lock().unwrap();
    assert_eq!(artifacts.len(), 1, "one successful write → one artifact");
    assert_eq!(artifacts[0].0, session.id);
    assert_eq!(artifacts[0].1, "notes/a.md");
    assert_eq!(artifacts[0].2, b"hi\n");
    assert!(capture.sweeps.lock().unwrap().is_empty());
}

#[tokio::test]
async fn incognito_turn_records_nothing() {
    let workspace = tempfile::tempdir().unwrap();
    let capture = Arc::new(CaptureKernel::default());
    let kernel: Arc<dyn librefang_kernel_handle::KernelHandle> = capture.clone();
    let driver = ToolThenTextDriver::new(
        "file_write",
        serde_json::json!({"path": "notes/a.md", "content": "hi\n"}),
    );

    run_one_turn(
        &kernel,
        workspace.path(),
        driver,
        "file_write",
        true,
        super::integration::fresh_session(),
    )
    .await;

    assert!(
        capture.artifacts.lock().unwrap().is_empty(),
        "an incognito turn must not leave an artifact row or blob"
    );
    assert!(
        capture.sweeps.lock().unwrap().is_empty(),
        "an incognito turn must not sweep the workspace"
    );
    // The write itself still happened — incognito hides history, not tools.
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("notes/a.md")).unwrap(),
        "hi\n"
    );
}

#[tokio::test]
async fn shell_turn_requests_a_sweep_and_incognito_does_not() {
    for incognito in [false, true] {
        let workspace = tempfile::tempdir().unwrap();
        let capture = Arc::new(CaptureKernel::default());
        let kernel: Arc<dyn librefang_kernel_handle::KernelHandle> = capture.clone();
        let driver = ToolThenTextDriver::new(
            "shell_exec",
            serde_json::json!({"command": "echo hi > made.txt"}),
        );

        let session = run_one_turn(
            &kernel,
            workspace.path(),
            driver,
            "shell_exec",
            incognito,
            super::integration::fresh_session(),
        )
        .await;

        let sweeps = capture.sweeps.lock().unwrap();
        if incognito {
            assert!(sweeps.is_empty(), "incognito must not sweep");
        } else {
            assert_eq!(
                sweeps.as_slice(),
                &[session.id],
                "a turn that ran a command must ask for the workspace sweep"
            );
        }
    }
}

#[tokio::test]
async fn incognito_loop_end_consumes_a_seeded_marker_without_sweeping() {
    let workspace = tempfile::tempdir().unwrap();
    let capture = Arc::new(CaptureKernel::default());
    let kernel: Arc<dyn librefang_kernel_handle::KernelHandle> = capture.clone();
    let driver = ToolThenTextDriver::new("file_read", serde_json::json!({"path": "missing.txt"}));
    let session = super::integration::fresh_session();
    // A marker can already be set when the turn begins (it was seeded here
    // directly; a deferred resume is the production shape).
    crate::artifact_capture::note_shell_tool(Some(session.id));

    let session = run_one_turn(
        &kernel,
        workspace.path(),
        driver,
        "file_read",
        true,
        session,
    )
    .await;

    assert!(
        capture.sweeps.lock().unwrap().is_empty(),
        "an incognito turn must not sweep"
    );
    assert!(
        !crate::artifact_capture::take_shell_ran(session.id),
        "the incognito loop end must consume the marker, not leave it for a later turn"
    );
}

// ---- role-trait boilerplate (same shape as `vision_gate::VisionKernel`) ----

impl librefang_kernel_handle::CatalogQuery for CaptureKernel {}
impl librefang_kernel_handle::SkillHistory for CaptureKernel {}

#[async_trait]
impl librefang_kernel_handle::AgentControl for CaptureKernel {
    async fn spawn_agent(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn send_to_agent(
        &self,
        _: &str,
        _: &str,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    fn list_agents(&self) -> Vec<librefang_kernel_handle::AgentInfo> {
        vec![]
    }
    fn kill_agent(&self, _: &str) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    fn find_agents(&self, _: &str) -> Vec<librefang_kernel_handle::AgentInfo> {
        vec![]
    }
}

impl librefang_kernel_handle::MemoryAccess for CaptureKernel {
    fn memory_store(
        &self,
        _: &str,
        _: serde_json::Value,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    fn memory_recall(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    fn memory_list(
        &self,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait]
impl librefang_kernel_handle::TaskQueue for CaptureKernel {
    async fn task_post(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: &librefang_kernel_handle::TaskPostOptions,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_claim(
        &self,
        _: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_complete(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_list(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_delete(&self, _: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_retry(&self, _: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_get(
        &self,
        _: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_update_status(
        &self,
        _: &str,
        _: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait]
impl librefang_kernel_handle::EventBus for CaptureKernel {
    async fn publish_event(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait]
impl librefang_kernel_handle::KnowledgeGraph for CaptureKernel {
    async fn knowledge_add_entity(
        &self,
        _: &librefang_types::memory::Entity,
        _: &str,
        _: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn knowledge_add_relation(
        &self,
        _: &librefang_types::memory::Relation,
        _: &str,
        _: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn knowledge_query(
        &self,
        _: librefang_types::memory::GraphPattern,
        _: Option<&str>,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError>
    {
        Err("not used".into())
    }
}

impl librefang_kernel_handle::ApiAuth for CaptureKernel {
    fn auth_snapshot(&self) -> ApiAuthSnapshot {
        ApiAuthSnapshot::default()
    }
}

impl librefang_kernel_handle::SessionWriter for CaptureKernel {
    fn inject_attachment_blocks(
        &self,
        _: librefang_types::agent::AgentId,
        _: librefang_types::agent::SessionId,
        _: Vec<ContentBlock>,
    ) {
    }
}

impl librefang_kernel_handle::WikiAccess for CaptureKernel {}
impl librefang_kernel_handle::CronControl for CaptureKernel {}
impl librefang_kernel_handle::ApprovalGate for CaptureKernel {}
impl librefang_kernel_handle::HandsControl for CaptureKernel {}
impl librefang_kernel_handle::A2ARegistry for CaptureKernel {}
impl librefang_kernel_handle::ChannelSender for CaptureKernel {}
impl librefang_kernel_handle::PromptStore for CaptureKernel {}
impl librefang_kernel_handle::WorkflowRunner for CaptureKernel {}
impl librefang_kernel_handle::GoalControl for CaptureKernel {}
impl librefang_kernel_handle::ToolPolicy for CaptureKernel {}
impl librefang_kernel_handle::AcpFsBridge for CaptureKernel {}
impl librefang_kernel_handle::AcpTerminalBridge for CaptureKernel {}
