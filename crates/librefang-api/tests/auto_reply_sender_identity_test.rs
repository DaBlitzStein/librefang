//! Integration test: the auto-reply turn runs under the sender's identity (refs #8423).
//!
//! `KernelBridgeAdapter::check_auto_reply` launches a full agent turn — tools included — and it used to do so through `KernelApi::send_message`, which carries no sender.
//! A turn that reaches the tool authorization gate without one is answered by `guest_gate`: the read-only tools are allowed and everything else is queued as `NeedsApproval`, so an agent answered through auto-reply could execute nothing.
//!
//! The turn's identity is observable where the production instance was caught: `usage_events.channel`, stamped from the `SenderContext` (see `attribution_channel` in `kernel/agent_execution.rs`).
//! With no context the column stays NULL — the same value an unidentified turn hands the tool gate.
//!
//! The kernel points at a local OpenAI-compatible stub so the turn actually runs and records: the driverless default short-circuits at `!driver.is_configured()` before any usage row exists, and a real provider would make this a network test.
//!
//! Run: cargo test -p librefang-api --test auto_reply_sender_identity_test

use librefang_api::channel_bridge::KernelBridgeAdapter;
use librefang_channels::bridge::ChannelBridgeHandle;
use librefang_channels::types::SenderContext;
use librefang_kernel::KernelApi;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, AgentManifest};
use librefang_types::config::{AutoReplyConfig, DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const API_KEY_ENV: &str = "AUTO_REPLY_IDENTITY_TEST_KEY";

/// Boot a kernel whose provider is a local OpenAI-compatible stub, with the
/// auto-reply engine on.
fn boot(server: &MockServer) -> (Arc<LibreFangKernel>, tempfile::TempDir) {
    // Read by the kernel when it builds the driver for the `openai` provider.
    std::env::set_var(API_KEY_ENV, "sk-test-auto-reply-identity");

    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    std::fs::create_dir_all(home.join("data")).expect("data dir");

    let config = KernelConfig {
        home_dir: home.clone(),
        data_dir: home.join("data"),
        default_model: DefaultModelConfig {
            provider: "openai".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key_env: API_KEY_ENV.to_string(),
            base_url: Some(server.uri()),
            ..DefaultModelConfig::default()
        },
        auto_reply: AutoReplyConfig {
            enabled: true,
            ..AutoReplyConfig::default()
        },
        ..KernelConfig::default()
    };

    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.clone().set_self_handle();
    (kernel, tmp)
}

fn spawn_agent(kernel: &Arc<LibreFangKernel>) -> AgentId {
    let manifest = AgentManifest {
        name: "auto-reply-agent".to_string(),
        source_template: None,
        ..AgentManifest::default()
    };
    kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent_typed")
}

/// The `channel` column of the most recent usage row for `agent_id`.
fn latest_usage_channel(kernel: &Arc<LibreFangKernel>, agent_id: AgentId) -> Option<String> {
    let pool = kernel.memory_substrate().pool();
    let conn = pool.get().expect("pool connection");
    conn.query_row(
        "SELECT channel FROM usage_events WHERE agent_id = ?1 ORDER BY timestamp DESC LIMIT 1",
        rusqlite::params![agent_id.0.to_string()],
        |row| row.get::<_, Option<String>>(0),
    )
    .expect("the auto-reply turn must have recorded a usage row")
}

/// A Telegram DM as the channel bridge builds it: the platform id is both the
/// sender and the chat, and it is the pair the RBAC gate resolves bindings on.
fn telegram_dm() -> SenderContext {
    SenderContext {
        channel: "telegram".to_string(),
        user_id: "34387719".to_string(),
        chat_id: Some("34387719".to_string()),
        display_name: "user".to_string(),
        is_group: false,
        ..Default::default()
    }
}

/// The auto-reply turn must record the channel it arrived on.
///
/// Without it the tool authorization gate has no channel and no sender, resolves
/// the call as `guest_gate`, and turns every non-read-only tool into an approval
/// request.
#[tokio::test(flavor = "multi_thread")]
async fn auto_reply_turn_records_the_sender_channel() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "hello" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8 }
        })))
        .mount(&server)
        .await;

    let (kernel, _tmp) = boot(&server);
    let agent_id = spawn_agent(&kernel);

    let adapter = KernelBridgeAdapter::new(kernel.clone() as Arc<dyn KernelApi>);

    adapter
        .check_auto_reply(agent_id, "hello", &telegram_dm())
        .await;

    assert_eq!(
        latest_usage_channel(&kernel, agent_id).as_deref(),
        Some("telegram"),
        "the auto-reply turn recorded no channel, so it ran without a SenderContext — the \
         tool authorization gate reads that as an unrecognised sender and forces every tool \
         outside the read-only allowlist into approval"
    );
}
