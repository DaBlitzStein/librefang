//! Resolve the agent-templates directory inside a `librefang-registry` checkout.
//!
//! [`librefang/librefang-registry`](https://github.com/librefang/librefang-registry)
//! is renaming its `agents/` directory to `agent-types/` (upstream naming
//! cleanup). We don't control when that lands — it can happen any day — so
//! every site that resolves this directory has to work with both names
//! simultaneously, with no window where a fresh sync comes up empty.
//!
//! Before this module existed, each call site did a bare
//! `registry_cache.join("agents")` + `.exists()` check. The day the registry
//! renames, that check silently returns `false`, the caller's `if` block is
//! skipped, and a fresh install ends up with zero preinstalled agent
//! templates — no error, no warning, nothing in the logs to point at why.
//! [`resolve_agent_types_dir`] is the single place that fixes this: prefer
//! the canonical new name, fall back to the legacy name with a one-shot
//! warning, and log loudly (not silently return "not found") when neither
//! directory exists — that last case is exactly the failure mode that used
//! to vanish without a trace.

use std::path::{Path, PathBuf};

/// Canonical (post-rename) name of the agent-templates directory inside a
/// registry checkout.
pub const AGENT_TYPES_DIR_NAME: &str = "agent-types";

/// Legacy name, still served by the registry until the rename lands upstream.
pub const LEGACY_AGENTS_DIR_NAME: &str = "agents";

/// Resolve the agent-templates directory within a registry checkout.
///
/// `registry_cache` is the root of the checkout (e.g. `~/.librefang/registry`
/// or the pinned test fixture directory) — the directory that directly
/// contains `providers/`, `hands/`, `agent-types/` / `agents/`, etc.
///
/// Resolution order:
/// 1. `{registry_cache}/agent-types/` — the canonical name. Used silently
///    when present, including when the legacy directory also exists (the new
///    name always wins so a transitional registry state where upstream ships
///    both doesn't accidentally pin callers to the old one).
/// 2. `{registry_cache}/agents/` — the legacy name. Used as a fallback with a
///    single `tracing::warn!` so operators get a signal that the registry
///    they're syncing from hasn't renamed yet, without flooding the log (this
///    is called once per sync / cache rebuild by every caller, never once
///    per agent or per routed message).
/// 3. Neither exists — `tracing::error!` and `None`. This is the case that
///    used to be indistinguishable from "the registry genuinely ships no
///    agent templates right now": callers must be able to tell "nothing to
///    sync" apart from "the sync's fan-out silently skipped its block", and
///    this log line is that signal.
pub fn resolve_agent_types_dir(registry_cache: &Path) -> Option<PathBuf> {
    let canonical = registry_cache.join(AGENT_TYPES_DIR_NAME);
    if canonical.is_dir() {
        return Some(canonical);
    }

    let legacy = registry_cache.join(LEGACY_AGENTS_DIR_NAME);
    if legacy.is_dir() {
        tracing::warn!(
            path = %legacy.display(),
            "registry checkout still serves the legacy '{LEGACY_AGENTS_DIR_NAME}/' directory name; \
             the canonical name is '{AGENT_TYPES_DIR_NAME}/' — this fallback exists for the \
             registry's in-progress rename and will keep working until it completes"
        );
        return Some(legacy);
    }

    let registry_cache_display = registry_cache.display();
    tracing::error!(
        registry_cache = %registry_cache_display,
        tried_canonical = %canonical.display(),
        tried_legacy = %legacy.display(),
        "registry checkout has neither '{AGENT_TYPES_DIR_NAME}/' nor legacy '{LEGACY_AGENTS_DIR_NAME}/' — \
         no agent templates are available from this checkout. This is not necessarily an empty \
         registry: verify the sync actually ran and populated {registry_cache_display}"
    );
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    /// Minimal in-memory writer so tests can assert on emitted log lines
    /// without touching stdout/stderr. Mirrors the pattern already used in
    /// `librefang-runtime/tests/instrument_span_fields.rs`.
    #[derive(Clone, Default)]
    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl io::Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for CaptureWriter {
        type Writer = CaptureWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Run `f` under a fresh tracing subscriber and return everything it logged.
    fn capture_logs(f: impl FnOnce()) -> String {
        let writer = CaptureWriter::default();
        let buf = writer.0.clone();
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(writer)
            .with_ansi(false)
            .with_target(false);
        let _guard = tracing_subscriber::registry().with(layer).set_default();
        f();
        let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        captured
    }

    #[test]
    fn only_legacy_agents_dir_resolves_and_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("agents");
        std::fs::create_dir_all(&legacy).unwrap();

        let logs = capture_logs(|| {
            let resolved = resolve_agent_types_dir(tmp.path());
            assert_eq!(resolved, Some(legacy.clone()), "must resolve legacy dir");
        });

        assert!(
            logs.contains("legacy") && logs.contains("agent-types"),
            "expected a warning naming both the legacy fallback and the canonical name, got: {logs}"
        );
    }

    #[test]
    fn only_agent_types_dir_resolves_without_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical = tmp.path().join("agent-types");
        std::fs::create_dir_all(&canonical).unwrap();

        let logs = capture_logs(|| {
            let resolved = resolve_agent_types_dir(tmp.path());
            assert_eq!(
                resolved,
                Some(canonical.clone()),
                "must resolve canonical dir"
            );
        });

        assert!(
            logs.is_empty(),
            "canonical-only resolution must not warn, got: {logs}"
        );
    }

    #[test]
    fn both_present_prefers_agent_types() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical = tmp.path().join("agent-types");
        let legacy = tmp.path().join("agents");
        std::fs::create_dir_all(&canonical).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();

        let logs = capture_logs(|| {
            let resolved = resolve_agent_types_dir(tmp.path());
            assert_eq!(
                resolved,
                Some(canonical.clone()),
                "canonical name must win when both exist"
            );
        });

        assert!(
            logs.is_empty(),
            "no warning expected when the canonical name is present, got: {logs}"
        );
    }

    #[test]
    fn neither_present_errors_loudly() {
        let tmp = tempfile::tempdir().unwrap();

        let logs = capture_logs(|| {
            let resolved = resolve_agent_types_dir(tmp.path());
            assert_eq!(resolved, None, "must return None when neither dir exists");
        });

        assert!(
            logs.contains("ERROR"),
            "missing-both case must log at error level, got: {logs}"
        );
        assert!(
            logs.contains("agent-types") && logs.contains("agents"),
            "error must name both directories that were tried, got: {logs}"
        );
    }
}
