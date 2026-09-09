//! Discover and load agent templates from the agents directory.

use std::path::PathBuf;

/// A discovered agent template.
pub struct AgentTemplate {
    /// Template name (directory name).
    pub name: String,
    /// Description from the manifest.
    pub description: String,
    /// Raw TOML content.
    pub content: String,
}

/// Resolve `$LIBREFANG_HOME`, falling back to `~/.librefang`.
///
/// Delegates to the kernel rather than re-deriving the rule, because template
/// discovery has to agree with the home the kernel resolves
/// `agent_template_candidates` against — a second copy drifting from it is the
/// class of bug #6699 already fixed once. `commands::common` holds two further
/// copies for its own callers; this module deliberately adds none.
fn librefang_home() -> PathBuf {
    librefang_kernel::config::librefang_home()
}

/// `LIBREFANG_AGENTS_DIR`, when it is set and names a real directory.
fn env_template_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var("LIBREFANG_AGENTS_DIR").ok()?);
    dir.is_dir().then_some(dir)
}

/// `~/.librefang/workspaces/agents/` — installed and live-agent templates.
fn workspace_agents_dir() -> Option<PathBuf> {
    let dir = librefang_home().join("workspaces").join("agents");
    dir.is_dir().then_some(dir)
}

/// Load every agent template, highest precedence first. There is no
/// bundled-template fallback — an empty result is what sends
/// `librefang agent new` to `commands/agent.rs`'s `agent-new-no-templates`
/// error instead.
///
/// Order:
/// 1. `LIBREFANG_AGENTS_DIR`, the one source the operator named on this very
///    invocation and which `docs/src/app/integrations/cli/page.mdx` documents
///    as an override. It wins over both on-disk stores, so a developer
///    pointing the CLI at a locally modified template gets that template
///    rather than a same-named copy the registry fan-out wrote into
///    `agent-types/`.
/// 2. `agent-types/<type>.toml` — the canonical operator-authored store, one
///    flat file per type, written by the dashboard editor and by the
///    `agent_type_create` tool (#7758).
/// 3. `workspaces/agents/<type>/agent.toml` — installed / live agents.
///
/// 2 before 3 is the precedence the kernel's `agent_template_candidates`
/// already applies (#8239); 1 sits outside that ladder because the kernel
/// never reads the env var at all.
pub fn load_all_templates() -> Vec<AgentTemplate> {
    let mut templates = Vec::new();
    let mut seen_names = std::collections::HashSet::new();

    if let Some(dir) = env_template_dir() {
        collect_dir_per_type(&dir, &mut seen_names, &mut templates);
    }

    collect_agent_type_files(
        &librefang_types::agent_type_store::agent_types_dir_in(&librefang_home()),
        &mut seen_names,
        &mut templates,
    );

    if let Some(dir) = workspace_agents_dir() {
        collect_dir_per_type(&dir, &mut seen_names, &mut templates);
    }

    templates.sort_by(|a, b| a.name.cmp(&b.name));
    templates
}

/// Read a flat `agent-types/<name>.toml` store into `out`.
fn collect_agent_type_files(
    dir: &std::path::Path,
    seen_names: &mut std::collections::HashSet<String>,
    out: &mut Vec<AgentTemplate>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            // Skips the staging files `atomic_write` leaves behind if the
            // process dies mid-rename, as well as anything else an operator
            // dropped in the directory.
            continue;
        }
        // `to_str`, not `to_string_lossy`: a non-UTF-8 stem would become a
        // `U+FFFD` name that addresses no template and that every other such
        // file collides with.
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // The same gate the store's own writer applies in `create_agent_type`,
        // so `agent new` never offers a row `agent_type_path_in` cannot
        // address. `librefang-api`'s listing and the TUI's picker both filter
        // their copy of this enumeration too; this one used to be the
        // exception.
        if librefang_types::agent_type_store::validate_agent_type_name(name).is_err() {
            continue;
        }
        if name == "custom" || seen_names.contains(name) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            // Claiming the name before the read would suppress the
            // directory-per-type fallback below for a file that contributed
            // nothing — dropping a type that used to be listed.
            continue;
        };
        // `AgentManifest` is `#[serde(default)]` with no `deny_unknown_fields`,
        // so an empty file parses into a nameless default manifest and would
        // shadow a real `workspaces/agents/<name>/agent.toml` with an
        // unspawnable row. `create_agent_type` claims the name with
        // `File::create_new` before `atomic_write` renames the content in, so
        // a zero-byte file is exactly what a killed create leaves behind.
        if content.trim().is_empty() {
            continue;
        }
        seen_names.insert(name.to_string());
        out.push(AgentTemplate {
            name: name.to_string(),
            description: extract_description(&content),
            content,
        });
    }
}

/// Read a directory-per-type root (`<dir>/<name>/agent.toml`) into `out`.
fn collect_dir_per_type(
    dir: &std::path::Path,
    seen_names: &mut std::collections::HashSet<String>,
    out: &mut Vec<AgentTemplate>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest = path.join("agent.toml");
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "custom" || seen_names.contains(&name) {
            continue;
        }
        // Same ordering rule as the flat store: claim the name only once the
        // entry is actually going to be pushed.
        let Ok(content) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        seen_names.insert(name.clone());
        out.push(AgentTemplate {
            name,
            description: extract_description(&content),
            content,
        });
    }
}

/// Extract the `description` field from raw TOML without full parsing.
fn extract_description(toml_str: &str) -> String {
    for line in toml_str.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("description") {
            if let Some(rest) = rest.trim_start().strip_prefix('=') {
                let val = rest.trim().trim_matches('"');
                return val.to_string();
            }
        }
    }
    String::new()
}

/// Format a template description as a hint for cliclack select items.
pub fn template_display_hint(t: &AgentTemplate) -> String {
    if t.description.is_empty() {
        String::new()
    } else if t.description.chars().count() > 60 {
        let truncated: String = t.description.chars().take(57).collect();
        format!("{truncated}...")
    } else {
        t.description.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::agent::AgentManifest;
    use librefang_types::config::DefaultModelConfig;

    // -----------------------------------------------------------------------
    // extract_description — TOML scanner without full parser dependency.
    // -----------------------------------------------------------------------

    #[test]
    fn extract_description_finds_quoted_value() {
        let toml = r#"
name = "demo"
description = "A demo agent"
"#;
        assert_eq!(extract_description(toml), "A demo agent");
    }

    #[test]
    fn extract_description_returns_empty_when_missing() {
        let toml = r#"
name = "demo"
"#;
        assert_eq!(extract_description(toml), "");
    }

    #[test]
    fn extract_description_handles_unquoted_value() {
        // The scanner trims surrounding double-quotes; an unquoted value
        // should still come through verbatim (with its surrounding whitespace
        // trimmed).
        let toml = "description = bare-value\n";
        assert_eq!(extract_description(toml), "bare-value");
    }

    #[test]
    fn extract_description_ignores_lines_with_description_substring() {
        // Only lines whose first non-whitespace token is `description` count;
        // a line like `# description of the agent` must not be picked up.
        let toml = r#"
# description of the agent
name = "demo"
description = "real one"
"#;
        assert_eq!(extract_description(toml), "real one");
    }

    #[test]
    fn extract_description_first_match_wins() {
        // Real TOML would not have two top-level `description` keys, but the
        // scanner is line-based — pin the first-match behaviour so refactors
        // don't silently flip it.
        let toml = r#"
description = "first"
description = "second"
"#;
        assert_eq!(extract_description(toml), "first");
    }

    // -----------------------------------------------------------------------
    // template_display_hint — UI hint formatter with 60-char ellipsis.
    // -----------------------------------------------------------------------

    fn make_template(description: &str) -> AgentTemplate {
        AgentTemplate {
            name: "t".to_string(),
            description: description.to_string(),
            content: String::new(),
        }
    }

    #[test]
    fn display_hint_passes_short_description_through() {
        let t = make_template("short and sweet");
        assert_eq!(template_display_hint(&t), "short and sweet");
    }

    #[test]
    fn display_hint_returns_empty_for_no_description() {
        let t = make_template("");
        assert_eq!(template_display_hint(&t), "");
    }

    #[test]
    fn display_hint_truncates_with_ellipsis_above_60_chars() {
        // 70 'a's → must be truncated to 57 chars + "..." = 60 chars total.
        let long = "a".repeat(70);
        let t = make_template(&long);
        let hint = template_display_hint(&t);
        assert_eq!(hint.chars().count(), 60);
        assert!(hint.ends_with("..."));
        assert!(hint.starts_with(&"a".repeat(57)));
    }

    #[test]
    fn display_hint_keeps_exactly_60_char_description_intact() {
        // Boundary: cutoff is `> 60`, so exactly 60 chars must NOT be
        // truncated.
        let s = "a".repeat(60);
        let t = make_template(&s);
        assert_eq!(template_display_hint(&t), s);
    }

    #[test]
    fn display_hint_counts_chars_not_bytes_for_unicode() {
        // 70 multi-byte characters: must trigger truncation by char count
        // (not by byte length) and must not panic on a non-char-boundary
        // byte slice.
        let s = "汉".repeat(70);
        let t = make_template(&s);
        let hint = template_display_hint(&t);
        assert_eq!(hint.chars().count(), 60);
        assert!(hint.ends_with("..."));
    }

    // -----------------------------------------------------------------------
    // env_template_dir / load_all_templates — env-var-driven path
    // discovery. `LIBREFANG_HOME` is also mutated by `launcher.rs`'s tests,
    // so these serialize on the crate-wide `crate::test_env::env_lock`
    // rather than a module-private mutex (#8239) — a private one only
    // protects tests within this file from each other, not from a
    // `launcher.rs` test flipping the same var mid-assertion.
    // -----------------------------------------------------------------------
    use crate::test_env::env_lock;

    /// Regression for #8239: a type written only into the canonical
    /// `agent-types/<name>.toml` store (the destination `POST /api/templates`
    /// and `agent_type_create` both use) must show up in `load_all_templates`,
    /// not just be visible to the kernel's own `agent_template_candidates`.
    #[test]
    fn load_all_templates_finds_type_written_only_to_agent_types_dir() {
        let _guard = env_lock();

        let home = tempfile::tempdir().expect("tempdir");
        let agent_types_dir = librefang_types::agent_type_store::agent_types_dir_in(home.path());
        std::fs::create_dir_all(&agent_types_dir).unwrap();
        std::fs::write(
            agent_types_dir.join("regression8239.toml"),
            "name = \"regression8239\"\ndescription = \"created via the dashboard\"\n",
        )
        .unwrap();

        let prev_home = std::env::var("LIBREFANG_HOME").ok();
        let prev_agents = std::env::var("LIBREFANG_AGENTS_DIR").ok();
        // SAFETY: serialized on env_lock.
        unsafe {
            std::env::set_var("LIBREFANG_HOME", home.path());
            std::env::remove_var("LIBREFANG_AGENTS_DIR");
        }

        let templates = load_all_templates();

        // SAFETY: see above — still under env_lock.
        unsafe {
            match prev_home {
                Some(v) => std::env::set_var("LIBREFANG_HOME", v),
                None => std::env::remove_var("LIBREFANG_HOME"),
            }
            match prev_agents {
                Some(v) => std::env::set_var("LIBREFANG_AGENTS_DIR", v),
                None => std::env::remove_var("LIBREFANG_AGENTS_DIR"),
            }
        }

        let names: Vec<&str> = templates.iter().map(|t| t.name.as_str()).collect();
        let found = templates
            .iter()
            .find(|t| t.name == "regression8239")
            .unwrap_or_else(|| panic!("regression8239 type not found in {names:?}"));
        assert_eq!(found.description, "created via the dashboard");
    }

    /// The flat `agent-types/` store must win over a same-named directory
    /// under `workspaces/agents/`, matching the precedence the kernel's
    /// `agent_template_candidates` already applies — otherwise editing a
    /// type from the dashboard could appear to have no effect if a stale
    /// live-agent workspace of the same name still exists.
    #[test]
    fn load_all_templates_prefers_agent_types_over_workspace_agent_of_same_name() {
        let _guard = env_lock();

        let home = tempfile::tempdir().expect("tempdir");

        let agent_types_dir = librefang_types::agent_type_store::agent_types_dir_in(home.path());
        std::fs::create_dir_all(&agent_types_dir).unwrap();
        std::fs::write(
            agent_types_dir.join("shared-name.toml"),
            "name = \"shared-name\"\ndescription = \"from agent-types\"\n",
        )
        .unwrap();

        let workspace_dir = home
            .path()
            .join("workspaces")
            .join("agents")
            .join("shared-name");
        std::fs::create_dir_all(&workspace_dir).unwrap();
        std::fs::write(
            workspace_dir.join("agent.toml"),
            "name = \"shared-name\"\ndescription = \"from workspaces/agents\"\n",
        )
        .unwrap();

        let prev_home = std::env::var("LIBREFANG_HOME").ok();
        let prev_agents = std::env::var("LIBREFANG_AGENTS_DIR").ok();
        // SAFETY: serialized on env_lock.
        unsafe {
            std::env::set_var("LIBREFANG_HOME", home.path());
            std::env::remove_var("LIBREFANG_AGENTS_DIR");
        }

        let templates = load_all_templates();

        // SAFETY: see above — still under env_lock.
        unsafe {
            match prev_home {
                Some(v) => std::env::set_var("LIBREFANG_HOME", v),
                None => std::env::remove_var("LIBREFANG_HOME"),
            }
            match prev_agents {
                Some(v) => std::env::set_var("LIBREFANG_AGENTS_DIR", v),
                None => std::env::remove_var("LIBREFANG_AGENTS_DIR"),
            }
        }

        let matches: Vec<&AgentTemplate> = templates
            .iter()
            .filter(|t| t.name == "shared-name")
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "expected exactly one 'shared-name' entry, found {}",
            matches.len()
        );
        assert_eq!(matches[0].description, "from agent-types");
    }

    /// Run `f` with both env vars forced to the given values, restoring them
    /// afterwards. `f` must not assert — return what you want to check and
    /// assert on it after the vars are back, or a failing assertion unwinds
    /// past the restore and leaks a temp `LIBREFANG_HOME` into every later
    /// test in the process.
    fn with_env<T>(
        home: Option<&std::path::Path>,
        agents_dir: Option<&std::path::Path>,
        f: impl FnOnce() -> T,
    ) -> T {
        let _guard = env_lock();
        let prev_home = std::env::var("LIBREFANG_HOME").ok();
        let prev_agents = std::env::var("LIBREFANG_AGENTS_DIR").ok();
        // SAFETY: serialized on the crate-wide env lock.
        unsafe {
            match home {
                Some(p) => std::env::set_var("LIBREFANG_HOME", p),
                None => std::env::remove_var("LIBREFANG_HOME"),
            }
            match agents_dir {
                Some(p) => std::env::set_var("LIBREFANG_AGENTS_DIR", p),
                None => std::env::remove_var("LIBREFANG_AGENTS_DIR"),
            }
        }
        let out = f();
        // SAFETY: see above — still under env_lock.
        unsafe {
            match prev_home {
                Some(v) => std::env::set_var("LIBREFANG_HOME", v),
                None => std::env::remove_var("LIBREFANG_HOME"),
            }
            match prev_agents {
                Some(v) => std::env::set_var("LIBREFANG_AGENTS_DIR", v),
                None => std::env::remove_var("LIBREFANG_AGENTS_DIR"),
            }
        }
        out
    }

    fn write_type(dir: &std::path::Path, file: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(file), body).unwrap();
    }

    fn write_workspace_agent(home: &std::path::Path, name: &str, description: &str) {
        write_type(
            &home.join("workspaces").join("agents").join(name),
            "agent.toml",
            &format!("name = \"{name}\"\ndescription = \"{description}\"\n"),
        );
    }

    /// A flat-store entry that cannot be read must not consume the name and
    /// suppress the directory-per-type fallback.
    ///
    /// The unreadable entry here is a *directory* whose name ends in `.toml`,
    /// which is one of the shapes the review named (alongside a file being
    /// renamed into place by the registry fan-out at the moment the CLI
    /// enumerates). Claiming the name before the read made `assistant`-style
    /// types vanish from the picker entirely — strictly worse than the bug
    /// #8239 fixed.
    #[test]
    fn unreadable_flat_store_entry_does_not_hide_workspace_template() {
        let home = tempfile::tempdir().expect("tempdir");
        let agent_types_dir = librefang_types::agent_type_store::agent_types_dir_in(home.path());
        // A directory, not a file: the extension filter admits it, the read fails.
        std::fs::create_dir_all(agent_types_dir.join("shadowed.toml")).unwrap();
        write_workspace_agent(home.path(), "shadowed", "from workspaces");

        let templates = with_env(Some(home.path()), None, load_all_templates);

        let names: Vec<&str> = templates.iter().map(|t| t.name.as_str()).collect();
        let found = templates
            .iter()
            .find(|t| t.name == "shadowed")
            .unwrap_or_else(|| panic!("workspace template was suppressed; got {names:?}"));
        assert_eq!(found.description, "from workspaces");
    }

    /// A stem the store's own writer would refuse must not be advertised.
    ///
    /// `librefang-api`'s listing and the TUI's picker both filter here; this
    /// enumeration was the one that did not, so it offered rows that
    /// `agent_type_path_in` cannot address.
    #[test]
    fn flat_store_skips_stem_the_store_would_reject() {
        let home = tempfile::tempdir().expect("tempdir");
        let agent_types_dir = librefang_types::agent_type_store::agent_types_dir_in(home.path());
        write_type(
            &agent_types_dir,
            "bad name!.toml",
            "name = \"whatever\"\ndescription = \"unaddressable\"\n",
        );
        write_type(
            &agent_types_dir,
            "good-name.toml",
            "name = \"good-name\"\ndescription = \"fine\"\n",
        );

        let templates = with_env(Some(home.path()), None, load_all_templates);

        let names: Vec<&str> = templates.iter().map(|t| t.name.as_str()).collect();
        assert!(
            names.contains(&"good-name"),
            "valid stem must still be listed: {names:?}"
        );
        assert!(
            !names.contains(&"bad name!"),
            "stem rejected by validate_agent_type_name must not be listed: {names:?}"
        );
    }

    /// A zero-byte `<name>.toml` — what a `create_agent_type` killed between
    /// its `File::create_new` claim and `atomic_write`'s rename leaves behind
    /// — parses into a nameless default manifest, so it must not shadow a real
    /// workspace template with an unspawnable row.
    #[test]
    fn empty_flat_store_file_does_not_shadow_workspace_template() {
        let home = tempfile::tempdir().expect("tempdir");
        let agent_types_dir = librefang_types::agent_type_store::agent_types_dir_in(home.path());
        write_type(&agent_types_dir, "halfwritten.toml", "");
        write_workspace_agent(home.path(), "halfwritten", "from workspaces");

        let templates = with_env(Some(home.path()), None, load_all_templates);

        let matches: Vec<&AgentTemplate> = templates
            .iter()
            .filter(|t| t.name == "halfwritten")
            .collect();
        assert_eq!(matches.len(), 1, "expected exactly one 'halfwritten' entry");
        assert_eq!(
            matches[0].description, "from workspaces",
            "the zero-byte flat-store file shadowed the real workspace manifest"
        );
    }

    /// `LIBREFANG_AGENTS_DIR` is documented as an override
    /// (`docs/src/app/integrations/cli/page.mdx`), so it has to beat both
    /// on-disk stores. Once `librefang init` fans the registry out into
    /// `agent-types/`, a developer pointing the CLI at a locally modified
    /// template would otherwise silently get the registry copy.
    #[test]
    fn env_agents_dir_overrides_the_flat_agent_types_store() {
        let home = tempfile::tempdir().expect("tempdir");
        let env_dir = tempfile::tempdir().expect("tempdir");

        let agent_types_dir = librefang_types::agent_type_store::agent_types_dir_in(home.path());
        write_type(
            &agent_types_dir,
            "assistant.toml",
            "name = \"assistant\"\ndescription = \"from agent-types\"\n",
        );
        write_type(
            &env_dir.path().join("assistant"),
            "agent.toml",
            "name = \"assistant\"\ndescription = \"from env override\"\n",
        );

        let templates = with_env(Some(home.path()), Some(env_dir.path()), load_all_templates);

        let matches: Vec<&AgentTemplate> =
            templates.iter().filter(|t| t.name == "assistant").collect();
        assert_eq!(matches.len(), 1, "expected exactly one 'assistant' entry");
        assert_eq!(
            matches[0].description, "from env override",
            "LIBREFANG_AGENTS_DIR must win over the agent-types/ store"
        );
    }

    /// A `LIBREFANG_AGENTS_DIR` pointing at nothing must be ignored rather
    /// than becoming an empty highest-precedence source.
    #[test]
    fn env_template_dir_skips_nonexistent_path() {
        let bogus = std::env::temp_dir().join("librefang-cli-templates-test-3582-does-not-exist");
        let _ = std::fs::remove_dir_all(&bogus);
        let empty_home = tempfile::tempdir().expect("tempdir");

        let resolved = with_env(Some(empty_home.path()), Some(&bogus), env_template_dir);

        assert_eq!(
            resolved, None,
            "non-existent AGENTS_DIR must be filtered out"
        );
    }

    /// The override is only picked up when it names a real directory.
    #[test]
    fn env_template_dir_picks_up_existing_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let empty_home = tempfile::tempdir().expect("tempdir");

        let resolved = with_env(Some(empty_home.path()), Some(dir.path()), env_template_dir);

        assert_eq!(resolved.as_deref(), Some(dir.path()));
    }

    /// Mirror the kernel's spawn-time + execute-time default_model overlay so
    /// we can verify a manifest with empty/"default" provider+model resolves
    /// to the configured default_model — not to any hardcoded vendor value.
    fn resolve_effective_model(
        manifest: &AgentManifest,
        default_model: &DefaultModelConfig,
    ) -> (String, String) {
        let provider_is_default =
            manifest.model.provider.is_empty() || manifest.model.provider == "default";
        let model_is_default = manifest.model.model.is_empty() || manifest.model.model == "default";
        let effective_provider = if provider_is_default {
            default_model.provider.clone()
        } else {
            manifest.model.provider.clone()
        };
        let effective_model = if model_is_default {
            default_model.model.clone()
        } else {
            manifest.model.model.clone()
        };
        (effective_provider, effective_model)
    }

    /// Bundled example template must not hardcode a provider; it should defer
    /// to the user's configured default_model (regression: openfang #967).
    #[test]
    fn example_custom_agent_template_does_not_hardcode_provider() {
        let toml_str = include_str!("../../../examples/custom-agent/agent.toml");
        let manifest: AgentManifest =
            toml::from_str(toml_str).expect("example agent.toml must parse");

        // Must not pin any specific vendor — otherwise switching default_model
        // in config.toml would have no effect on agents spawned from this template.
        assert_ne!(manifest.model.provider, "groq");
        assert_ne!(manifest.model.model, "llama-3.3-70b-versatile");

        // Must be either empty or the explicit "default" sentinel so the
        // kernel's default_model overlay applies.
        let provider_defers =
            manifest.model.provider.is_empty() || manifest.model.provider == "default";
        let model_defers = manifest.model.model.is_empty() || manifest.model.model == "default";
        assert!(
            provider_defers && model_defers,
            "example template must defer to default_model, got provider={:?} model={:?}",
            manifest.model.provider,
            manifest.model.model
        );
    }

    /// End-to-end: a manifest deferring to default_model resolves to whatever
    /// the user has configured — not to the legacy groq fallback.
    #[test]
    fn manifest_with_default_provider_resolves_to_configured_default_model() {
        let toml_str = include_str!("../../../examples/custom-agent/agent.toml");
        let manifest: AgentManifest =
            toml::from_str(toml_str).expect("example agent.toml must parse");

        // Simulate a user who switched their default to OpenAI.
        let user_default = DefaultModelConfig {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            ..Default::default()
        };

        let (provider, model) = resolve_effective_model(&manifest, &user_default);
        assert_eq!(provider, "openai");
        assert_eq!(model, "gpt-4o");
        assert_ne!(provider, "groq");
        assert_ne!(model, "llama-3.3-70b-versatile");
    }
}
