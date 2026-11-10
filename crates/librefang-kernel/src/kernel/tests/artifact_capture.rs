//! A deferred `file_write` resumed with "Allow once" must inherit the
//! originating turn's incognito state (T3.3).
//!
//! The approval flow itself is irrelevant here: both tests build the
//! `DeferredToolExecution` the live turn would have persisted and call the
//! same resume entry point the approval handler calls.

use crate::kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::config::KernelConfig;
use librefang_types::tool::DeferredToolExecution;

/// Boot a real kernel on a temp home, with the artifact store attached the
/// way `boot_with_config` does in production.
fn boot_kernel() -> (std::sync::Arc<LibreFangKernel>, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let home_dir = home.path().to_path_buf();
    std::fs::create_dir_all(home_dir.join("data")).unwrap();
    let config = KernelConfig {
        home_dir: home_dir.clone(),
        data_dir: home_dir.join("data"),
        ..KernelConfig::default()
    };
    let kernel =
        std::sync::Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel should boot"));
    kernel.set_self_handle();
    (kernel, home)
}

fn deferred_file_write(
    workspace: &std::path::Path,
    session: SessionId,
    incognito: bool,
) -> DeferredToolExecution {
    DeferredToolExecution {
        agent_id: AgentId::new().to_string(),
        tool_use_id: "tu-capture".to_string(),
        tool_name: "file_write".to_string(),
        input: serde_json::json!({"path": "resumed.txt", "content": "resumed\n"}),
        allowed_tools: None,
        allowed_env_vars: None,
        exec_policy: None,
        sender_id: None,
        channel: None,
        chat_id: None,
        account_id: None,
        workspace_root: Some(workspace.to_path_buf()),
        force_human: false,
        session_id: Some(session),
        incognito,
    }
}

/// Resume `deferred` through the kernel and return the artifact paths it
/// recorded for the deferred session.
async fn resume_and_collect(
    kernel: &LibreFangKernel,
    deferred: &DeferredToolExecution,
) -> Vec<String> {
    let result = kernel
        .execute_deferred_tool(deferred)
        .await
        .expect("deferred resume must run");
    assert!(!result.is_error, "{}", result.content);

    let session = deferred.session_id.expect("test sets a session");
    kernel
        .memory
        .substrate
        .artifacts()
        .expect("boot attaches the artifact store")
        .list(&session.0.to_string())
        .expect("list artifacts")
        .into_iter()
        .map(|row| row.path)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn deferred_resume_from_a_normal_turn_records_the_file() {
    let (kernel, _home) = boot_kernel();
    let workspace = tempfile::tempdir().unwrap();
    let session = SessionId::new();
    let deferred = deferred_file_write(workspace.path(), session, false);

    let paths = resume_and_collect(kernel.as_ref(), &deferred).await;

    assert_eq!(paths, ["resumed.txt"]);
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("resumed.txt")).unwrap(),
        "resumed\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deferred_resume_from_an_incognito_turn_records_nothing() {
    let (kernel, _home) = boot_kernel();
    let workspace = tempfile::tempdir().unwrap();
    let session = SessionId::new();
    let deferred = deferred_file_write(workspace.path(), session, true);

    let paths = resume_and_collect(kernel.as_ref(), &deferred).await;

    assert!(
        paths.is_empty(),
        "an incognito deferred write must not be recorded: {paths:?}"
    );
    assert!(
        workspace.path().join("resumed.txt").exists(),
        "the approved write itself still runs — incognito hides history, not tools"
    );
}
