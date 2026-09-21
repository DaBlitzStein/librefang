//! #8459 — a config reload must not revert a field the on-disk document does not state.
//!
//! Every load funnels through `try_load_config`, which ends by deserializing the resolved document
//! into a `KernelConfig`. `KernelConfig` carries **container-level** `#[serde(default)]`, and that
//! fills each absent field from `KernelConfig::default()` — not from the field type's own `Default`.
//! So a document that parses but is *partial* does not leave the live values alone: it resets
//! everything it does not mention to the compiled default.
//!
//! The hazard is documented as already closed at `crates/librefang-kernel/src/config.rs:209`, and it
//! is, for a file that fails to parse. Strictness stops at parseability, which is what this pins.
//!
//! A partial document is not hypothetical. `persist_identity_sections`
//! (`crates/librefang-api/src/routes/users.rs:1368`) reads the existing file and rewrites only the
//! sections it manages, so on a kernel whose `config_path` does not exist yet — an in-memory boot,
//! the desktop, every integration test, and a deployment pointing `LIBREFANG_CONFIG_PATH` at a fresh
//! path — the first write produces a `config.toml` stating `users` and nothing else. A deployment
//! supplying its token through `LIBREFANG_API_KEY` (`config.rs:121`) has an `api_key` in memory that
//! no file states, which is why the assertion below is not about a path.

use librefang_testing::MockKernelBuilder;

/// The document a partial writer leaves behind when there is nothing to read first.
const PARTIAL_CONFIG: &str = "[[users]]\nname = \"Alice\"\nrole = \"user\"\n";

#[tokio::test(flavor = "multi_thread")]
async fn a_reload_keeps_the_fields_the_document_does_not_state() {
    let (kernel, tmp) = MockKernelBuilder::new()
        .with_config(|c| {
            c.api_key = "a-key-no-file-states".to_string();
            c.default_model.model = "a-model-no-file-states".to_string();
        })
        .build();

    let before = kernel.config_snapshot();
    let (home_before, key_before, model_before) = (
        before.home_dir.clone(),
        before.api_key.clone(),
        before.default_model.model.clone(),
    );
    drop(before);

    // The document states `users`. It does not state a home, a key or a model.
    std::fs::write(tmp.path().join("config.toml"), PARTIAL_CONFIG)
        .expect("write the partial config in the kernel's own config path");

    kernel
        .reload_config()
        .await
        .expect("a document that parses must reload");

    let after = kernel.config_snapshot();
    assert_eq!(
        after.home_dir, home_before,
        "a reload must not invent a home directory the document does not state"
    );
    assert_eq!(
        after.api_key, key_before,
        "a reload must not invent an API key the document does not state"
    );
    assert_eq!(
        after.default_model.model, model_before,
        "a reload must not invent a default model the document does not state"
    );

    kernel.shutdown();
}

/// The same fault, reached the way it was actually found: through the API's config-write path,
/// which creates the partial document itself rather than being handed one.
///
/// Kept separate from the test above because it fails for the same reason but from a different
/// direction — if only one of them goes green after a fix, the fix is in the wrong place.
#[tokio::test(flavor = "multi_thread")]
async fn a_reload_keeps_a_key_the_writer_never_stated() {
    let (kernel, tmp) = MockKernelBuilder::new()
        .with_config(|c| c.api_key = "a-key-no-file-states".to_string())
        .build();

    let key_before = kernel.config_snapshot().api_key.clone();

    // What `persist_identity_sections` leaves behind on a kernel whose `config_path` did not exist:
    // it starts from an empty document and inserts only the sections it owns.
    std::fs::write(tmp.path().join("config.toml"), PARTIAL_CONFIG).expect("write");

    kernel.reload_config().await.expect("reload");

    assert_eq!(
        kernel.config_snapshot().api_key,
        key_before,
        "the write that created this document owns `users`; it must not be read as owning the key"
    );

    kernel.shutdown();
}
