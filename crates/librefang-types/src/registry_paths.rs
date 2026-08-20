//! Resolve the agent-types directories used on both sides of a registry
//! sync: the *source* (inside a `librefang-registry` checkout) and the
//! *destination* (this installation's own live agent-type store).
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
//!
//! [`installed_agent_types_dir`] resolves the other side: where THIS
//! installation keeps its own live agent-type manifests
//! (`~/.librefang/agent-types/`) — populated by a registry sync, by
//! `POST /api/agent-types`, by the `agent_type_create` tool, and by
//! `save-as-agent-type`, and read back by every agent_type-spawn resolver
//! and by `GET /api/agent-types`. It used to be `~/.librefang/templates/`,
//! which was wrong on two counts: that directory is a *different* domain
//! (starter/skeleton TOML scaffolding for agent/hand/skill/channel/workflow
//! authoring, #7758) that agent-types writes were silently contaminating,
//! and the registry's copy of it was landing agent-type *instances* inside
//! `~/.librefang/workspaces/agents/` — the deployed-agents directory — so a
//! fresh install's registry sync manufactured dozens of agents the operator
//! never asked to run. [`warn_on_agent_type_like_files_in_templates_dir`]
//! is the read-only, boot-time half of the fix for installs that already
//! have agent-type-shaped files sitting in the old, wrong location: it
//! warns and names the files, but never moves or deletes anything — moving
//! files between two unrelated domains on the operator's behalf is exactly
//! what this fix is trying to stop doing.

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

/// Directory name for this installation's own agent-type manifest store.
/// Deliberately the same string as [`AGENT_TYPES_DIR_NAME`] — it names the
/// same *concept* (agent-type manifests) on the destination side of a sync,
/// just rooted at `home_dir` (`~/.librefang/`) instead of a registry
/// checkout.
pub const INSTALLED_AGENT_TYPES_DIR_NAME: &str = AGENT_TYPES_DIR_NAME;

/// Resolve this installation's canonical agent-type manifest directory
/// (`~/.librefang/agent-types/`).
///
/// This is the single place every reader and writer of agent-type manifests
/// should call instead of hand-rolling `home_dir.join("agent-types")` (or,
/// worse, `home_dir.join("templates")` / `home_dir.join("workspaces").join("agents")`
/// — both wrong locations this directory replaces). See the module doc
/// comment for why those two were wrong.
///
/// Agent-type manifests are stored flat: `agent-types/<name>.toml` — one
/// file per type, matching what `GET/POST/PUT/DELETE /api/agent-types`, the
/// `agent_type_create` tool, and every ephemeral/persistent spawn resolver
/// already expect. This is deliberately NOT the registry checkout's own
/// directory-per-type layout (`agent-types/<name>/agent.toml`, see
/// [`resolve_agent_types_dir`]) — a registry sync flattens on copy.
pub fn installed_agent_types_dir(home_dir: &Path) -> PathBuf {
    home_dir.join(INSTALLED_AGENT_TYPES_DIR_NAME)
}

/// Boot-time diagnostic (never a mutation): warn when
/// `{home_dir}/templates/` — the unrelated starter/skeleton TOML directory
/// for agent/hand/skill/channel/workflow authoring (#7758) — contains files
/// that look like agent-type manifests rather than plain skeletons.
///
/// A now-fixed bug used to write operator- and tool-created agent-types
/// into `templates/` instead of the canonical [`installed_agent_types_dir`].
/// This function does not move, copy, or delete anything — `templates/` and
/// `agent-types/` are different domains, and silently relocating files
/// between them on the operator's behalf is exactly the mistake this fix is
/// correcting. It only surfaces a `tracing::warn!` naming the suspect files
/// and the canonical directory, so the operator can decide.
///
/// Heuristic: a legitimate `templates/` skeleton (e.g. a minimal
/// `name = "…"` / `module = "…"` starter) does not declare a `[model]`
/// table — every agent-type manifest written by `POST /api/agent-types`,
/// the `agent_type_create` tool, or `save-as-agent-type` does, because all
/// three serialize a full `AgentManifest`, which always carries a `model`
/// field. A `[model]` table is therefore a reliable (if imperfect) signal
/// that a `templates/*.toml` file belongs in `agent-types/` instead.
pub fn warn_on_agent_type_like_files_in_templates_dir(home_dir: &Path) {
    let templates_dir = home_dir.join("templates");
    let Ok(entries) = std::fs::read_dir(&templates_dir) else {
        return;
    };

    let mut suspects: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("toml") {
                return None;
            }
            let content = std::fs::read_to_string(&path).ok()?;
            let value: toml::Value = toml::from_str(&content).ok()?;
            value
                .get("model")
                .and_then(|m| m.as_table())
                .map(|_| path.display().to_string())
        })
        .collect();

    if suspects.is_empty() {
        return;
    }
    suspects.sort();

    let canonical = installed_agent_types_dir(home_dir);
    tracing::warn!(
        files = ?suspects,
        canonical_dir = %canonical.display(),
        templates_dir = %templates_dir.display(),
        "found {} file(s) in the 'templates/' scaffold directory that look like agent-type \
         manifests (each declares a [model] table) — 'templates/' holds starter TOML skeletons \
         for agent/hand/skill/channel/workflow authoring, not agent-type storage; the canonical \
         location for agent-types is 'agent-types/'. This is a diagnostic only: nothing was \
         moved or deleted. If these files are meant to be agent-types, move them yourself.",
        suspects.len()
    );
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

    // ---- installed_agent_types_dir ----------------------------------

    #[test]
    fn installed_agent_types_dir_is_home_join_agent_types() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            installed_agent_types_dir(tmp.path()),
            tmp.path().join("agent-types")
        );
    }

    // ---- warn_on_agent_type_like_files_in_templates_dir --------------

    #[test]
    fn templates_dir_missing_is_a_silent_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let logs = capture_logs(|| {
            warn_on_agent_type_like_files_in_templates_dir(tmp.path());
        });
        assert!(
            logs.is_empty(),
            "no templates/ dir at all must not warn, got: {logs}"
        );
    }

    #[test]
    fn plain_skeleton_without_model_table_does_not_warn() {
        let tmp = tempfile::tempdir().unwrap();
        let templates_dir = tmp.path().join("templates");
        std::fs::create_dir_all(&templates_dir).unwrap();
        std::fs::write(
            templates_dir.join("tooled-mission.toml"),
            "name = \"tooled-mission\"\nmodule = \"builtin:chat\"\n",
        )
        .unwrap();

        let logs = capture_logs(|| {
            warn_on_agent_type_like_files_in_templates_dir(tmp.path());
        });
        assert!(
            logs.is_empty(),
            "a skeleton with no [model] table must not be flagged, got: {logs}"
        );
    }

    #[test]
    fn file_with_model_table_warns_and_names_it_without_moving_anything() {
        let tmp = tempfile::tempdir().unwrap();
        let templates_dir = tmp.path().join("templates");
        std::fs::create_dir_all(&templates_dir).unwrap();
        let misplaced = templates_dir.join("misplaced-agent-type.toml");
        std::fs::write(
            &misplaced,
            "name = \"misplaced-agent-type\"\ndescription = \"oops\"\n\n[model]\nprovider = \"default\"\nmodel = \"default\"\n",
        )
        .unwrap();

        let logs = capture_logs(|| {
            warn_on_agent_type_like_files_in_templates_dir(tmp.path());
        });

        assert!(
            logs.contains("WARN") && logs.contains("misplaced-agent-type.toml"),
            "must warn and name the suspect file, got: {logs}"
        );
        assert!(
            logs.contains("agent-types"),
            "warning must point at the canonical directory, got: {logs}"
        );
        assert!(
            misplaced.exists(),
            "the file must never be moved or deleted — diagnostic only"
        );
        assert!(
            !tmp.path().join("agent-types").exists(),
            "no agent-types/ directory should be created as a side effect"
        );
    }
}
