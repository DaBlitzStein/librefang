# Changelog

All notable changes to LibreFang will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Calendar Versioning](https://calver.org/) (YYYY.M.DD).

## [Unreleased]

### Fixed

- Escape every TOML control character when the dashboard serializes agent manifest strings, preserving carriage returns, tabs, and other control bytes as valid round-trippable TOML instead of producing a manifest the daemon cannot parse. (@TechWizard9999)

## [2026.7.31] - 2026-07-31

_58 PRs from 4 contributors since v2026.7.27._

### Highlights

- **API key security** — Keys now support env/vault indirection and a hashed form, closing a hash-only WebSocket/terminal auth bypass; three additional authorization boundaries around plugin execution, MCP env values, and cross-user token refresh were also closed.
- **Speech-to-text improvements** — Language and prompt parameters now thread through STT, and video containers are accepted as input for transcription.
- **Browser CDP attachment** — Agents can now attach to browser-level Chrome DevTools Protocol endpoints via `Target.createTarget`, enabling richer browser automation.
- **EveryAPI integration** — Auto-detection of EveryAPI CLI credentials and new partner surfaces make connecting to EveryAPI faster and require no manual setup.
- **Kubernetes deployment** — A single-replica baseline with readiness contract and rootless restricted-PSS container support makes LibreFang deployable on Kubernetes out of the box.

### Added

- Add `exec_policy.full_mode_skips_approval`, which decouples the two properties `mode = "full"` has always fused, so an operator can run unrestricted shell commands that still prompt for approval.
  `Full` waived the global `approval.require_approval` list for `shell_exec` as well as skipping allowlist validation, so an operator who deliberately set `Full` for one agent silently lost their `require_approval = ["shell_exec"]` for it, with nothing on any surface saying so.
  That coupling was deliberate rather than accidental, so the flag makes a documented decision overridable instead of fixing a bug: with the flag off, `Full` waives only command validation and `[approval]` decides who must confirm, exactly as under `allowlist`.
  The default is `true` and preserves today's behaviour on every existing install, and it is deliberately not flipped for two reasons that are load-bearing together: `ApprovalPolicy::default()` ships with `shell_exec` in `require_approval`, and `Kernel::spawn` promotes any standalone agent whose `capabilities.tools` contains `shell_exec` or `*` and which declares no `exec_policy` to `mode = "full"`.
  On a stock install this waiver is therefore the only reason ordinary agents run shell commands unattended, and a flipped default would prompt on every command rather than expressing a new operator intent.
  The `safe_bins_skip_approval` waiver from #6000 is intentionally left unconditional: it is by its own name an explicit approval opt-out, whereas `Full` is a command-validation mode that never claimed to speak for `[approval]`.
  A per-user RBAC `NeedsApproval` still forces the approval queue in either position of the flag, and the field is readable on `GET /api/config` alongside its neighbour.
  Like the rest of `exec_policy` it is baked into each agent's manifest at spawn / restore time, so `POST /api/config/reload` does not retrofit it onto already-running agents — kill the agent and let it respawn, or restart the daemon (#6594) (@houko)

- Add an EveryAPI connect action to the dashboard's Providers page, so registering the gateway no longer requires dropping to `librefang models connect everyapi` in a terminal.
  EveryAPI is not a built-in provider: until a registry entry exists it is absent from `GET /api/providers` altogether rather than merely unconfigured, so the Add picker — which lists what that endpoint already returns — could never surface it, and the dashboard had no path to it at all.
  The picker footer now offers a connect action while no `everyapi` entry exists, opening a two-field drawer for the relay key and an optional gateway root.
  It keys off the entry *existing* rather than being usable, because an entry with a missing key is already reachable through the normal configure flow and re-offering connect there would overwrite it.
  The entry is registered with an empty `models` array on purpose: `catalog_needs_initial_refresh` in `crates/librefang-api/src/everyapi_catalog.rs` is true exactly when the provider is configured but has no live models, so the daemon's `refresh_if_missing_in_background` fetches `/v1/models` and `/api/pricing` and synthesises the catalog itself.
  Duplicating that synthesis in the browser would mean a second copy of the pricing rules to keep in sync, and the gateway is not CORS-open to the dashboard in any case.
  The provider id, display name, key env var and default gateway root are exported as `EVERYAPI_PROVIDER` alongside the mutation and documented as a cross-language contract with the CLI's constants, since a drift there would register a provider the daemon never refreshes (#6586) (@houko)
- Add `librefang models connect everyapi [--set-default]`, which registers an [EveryAPI](https://github.com/everyapi-ai/everyapi) aggregating gateway as a custom LLM provider in one command, plus an `EveryApiWiringCheck` in `librefang doctor` that reports whether such a gateway is present and which route is actually in effect.
  EveryAPI's own `everyapi use <tool>` injects environment variables and execs a child process, which does nothing for a long-lived daemon whose HTTP drivers read `base_url` from config rather than the environment — so the wiring belongs on the LibreFang side.
  The command reads `api_base` and `relay_key` from `~/.config/everyapi/credentials.json` (honouring `$XDG_CONFIG_HOME`), fetches the gateway's live `/v1/models` listing, resolves model metadata from the builtin catalog (the gateway publishes ids but no pricing and mostly no context window), stores the key in `~/.librefang/.env`, and persists the provider through the running daemon when one is up or by writing `~/.librefang/providers/everyapi.toml` when it is not; the key never enters the provider TOML, the terminal, or the logs.
  A text model whose context window and output limit cannot both be resolved is skipped and named in the output rather than registered, because such an entry is discarded when the catalog loads and would otherwise vanish silently; entries with unresolved pricing are marked `pricing_known = false` so budget math does not treat them as free.
  Synthesised entries deliberately avoid `ModelTier::Custom`: `find_model` returns the first `Custom` match immediately (#983) and `merge_catalog_file` dedupes on `(id, provider)`, so a `Custom` gateway copy of a colliding id (`claude-sonnet-5`, `claude-opus-5`, `gemini-3.5-flash`) would have hijacked every provider-blind lookup and silently re-priced agents that never opted into the gateway.
  `--set-default` will not pick an `openai-response`-only model, since those reject the non-streaming calls that compaction, proactive memory, the skill workshop, and web augmentation all issue.
  The doctor check warns when an env route and a provider entry are live at once, naming both, because the effective gateway then differs per driver and neither surface mentions the other (#6583) (@houko)
- Source EveryAPI model metadata from the gateway's own public `/api/pricing` feed rather than reverse-looking-up ids in the compiled-in OpenRouter snapshot, and refresh the catalog on a TTL so the model list does not go stale.
  The snapshot lookup guessed a vendor prefix from `owned_by`, resolved only 7 of 18 models, and copied the upstream vendor's list price — which is not what the gateway charges.
  `/api/pricing` is served with optional auth, publishes `context_window` plus the gateway's real per-token ratios, and converts as `model_ratio * 2.0` for input and `model_ratio * completion_ratio * 2.0` for output; that conversion is corroborated by both the gateway's own docs and its `billing/quota.go` settlement path against `QuotaPerUnit = 500000`.
  Models billed per call carry no per-token price and are emitted with `pricing_known = false` instead of a bare `0.0`, which would have asserted they are free.
  Output-token limits are the one figure neither gateway endpoint publishes, so the snapshot is kept solely as their fallback and the command reports which models borrowed one.
  A new `everyapi_catalog` module in `librefang-api` mirrors the existing `openrouter_catalog` shape — TTL staleness check, background backfill, and a shared retry window — so a gateway that adds or removes models is picked up without re-running the connect command (#6583) (@houko)
- Add `librefang service install --system` on macOS, which registers a boot-time LaunchDaemon instead of the login-time LaunchAgent the command has always written.
  The existing behaviour was that every platform got a *per-user* service, so a Mac that rebooted and stopped at the login window — or at the FileVault unlock screen — never started the daemon at all, which makes an always-on install impossible without hand-writing a plist.
  `--system` writes `/Library/LaunchDaemons/ai.librefang.daemon.plist`, the directory launchd loads at boot before any login.
  It requires root and specifically requires `sudo` rather than a root login, because the job has to run as a real account and `SUDO_USER` is the only signal identifying which one; a missing `SUDO_USER` is an error rather than a silent default to root.
  The generated job sets `UserName` to that account, `HOME` to its home directory and `LIBREFANG_HOME` to `~/.librefang`, resolved from the passwd database rather than through `dirs::home_dir()` — under `sudo` the latter returns root's home and the daemon would serve a state directory the invoking user cannot read.
  The install also creates the state directory and `daemon.log` and hands the whole tree to the target account, because launchd opens `StandardOutPath` before dropping privileges and the daemon would otherwise be unable to write its own log.
  The handover is recursive on purpose: `~/.librefang` almost always exists already by the time anyone reaches for `--system`, so chowning only the directory node would leave its contents on whatever uid created them, and a state directory previously written by a `sudo librefang start` would leave the LaunchDaemon hitting EACCES at an arbitrary depth long after `service status` reported everything registered.
  The walk records symlinks but never descends them (`DirEntry::file_type` describes the entry, not its target, and the chown uses `lchown`), so a link pointing out of the state directory cannot redirect the handover onto an unrelated tree.
  The plist is chmodded to 0644, since launchd refuses to load a LaunchDaemon writable by anyone but its owner.
  The pre-existing root guard on the per-user path is unchanged: plain `service install` still refuses to run under `sudo`, and now points at `--system` when it does.
  `service uninstall --system` removes it, `service status` reports both the LaunchAgent and the LaunchDaemon with no flag needed, and `--system` on Linux or Windows is a clear error pointing at the `services.librefang` NixOS module or a hand-written system unit rather than a silent no-op (@houko)
- Promote NixOS from a package-only target to a first-class deployment surface, and give deepin / Debian-family hosts the distro awareness they previously lacked entirely.
  The flake gains system-agnostic `nixosModules.default` / `nixosModules.librefang` (backed by the new `nix/nixos-module.nix`) exposing `services.librefang` with `port`, `openFirewall`, `user`, `group`, `stateDir`, `environmentFile`, and `extraEnvironment`, plus `overlays.default` so nixpkgs consumers can pull `librefang-cli` / `librefang-desktop` into their own package set.
  `ExecStart` uses `start --foreground` deliberately: the default `librefang start` takes the `!spawned && !foreground` branch into `spawn_detached_daemon`, which `setsid`s a child and returns from the parent, so a unit without the flag would have its service torn down moments after `nixos-rebuild` reported success.
  Evaluation-time assertions reject an `environmentFile` inside the Nix store (which would make provider keys world-readable), a `stateDir` whose final component is empty or whose parent is a shared FHS directory, an `extraEnvironment.LIBREFANG_HOME` that would desynchronise from `StateDirectory`, and an `openFirewall` paired with a non-loopback bind that has no auth source.
  `checks` now covers `librefang-desktop` on Linux (previously reachable only through `packages`, so a `wrapGAppsHook3` or desktop-item regression passed `nix flake check`) and a `nixos-module-eval` check that instantiates a throwaway `nixosSystem` and asserts thirteen properties of the generated unit, which is how CI validates the module without a NixOS host.
  A `nixos-vm-test` check boots a real NixOS guest and asserts the unit reaches active, port 4545 opens and `/api/health` answers, covering the one thing evaluation cannot prove; it is opt-in via `nix build .#checks.x86_64-linux.nixos-vm-test` because building it compiles the CLI and boots a VM, while `nix flake check --no-build` still instantiates the expression so a module rename that broke the test is caught (measured: the full `--no-build --all-systems` pass instantiates both new checks and performs zero builds).
  `nix-build.yml` gains a `pull_request` job that runs evaluation only, so the Nix path is gated before merge without paying the 80-95 minute cold compile the push-to-main matrix still runs.
  The installer learns `detect_distro` from `/etc/os-release` with `/etc/NIXOS` as the authoritative NixOS marker, degrading silently to unknown where neither exists, and on NixOS it now suppresses the glibc fallback at both decision sites — that binary can never execute there for want of `/lib64/ld-linux-x86-64.so.2`, so the previous behaviour downloaded it, failed the post-install run check, rolled back, and printed a generic failure that told a NixOS user nothing.
  `librefang doctor` gains a Linux-only soft check that probes pkg-config for the desktop webview stack and maps `/etc/os-release` to a per-family remediation hint, deliberately suggesting a package *search* rather than concrete package names because the package shipping a given module differs between distributions and between releases of one distribution.
  Documented in `docs/operations/nixos.md`, plus new README and docs-site sections for NixOS and Debian / Ubuntu / deepin in both English and Chinese (@houko)
- Implement the MCP `resources` primitive in the `librefang-runtime-mcp` client, which was previously tools-only, so an agent can now consume MCP servers that expose their data as resources rather than tools.
  `McpConnection` gains `list_resources` (`resources/list`), `read_resource` (`resources/read`), and `list_resource_templates` (`resources/templates/list`) on both the rmcp (stdio + streamable-HTTP) and hand-rolled SSE transports; HttpCompat has no resources concept and returns a clear error.
  When a server advertises the `resources` capability (read live from the rmcp handshake `peer_info`, or captured from the SSE `initialize` response) the client registers two synthetic tools — `list_resources` and `read_resource` — that flow through the normal tool-call loop and are intercepted before the transport `tools/call`, so a real server tool literally named `read_resource` is unaffected.
  A `resource_link` in a tool result is now surfaced as a first-class `[resource_link] name — uri (mime)` line instead of being flattened into an opaque JSON string, and an embedded resource contributes its text (binary blobs are elided, never inlined into the prompt); resource lists are sorted by URI for prompt-cache stability.
  No `resources` client capability is declared because the MCP `resources` capability is server-side and rmcp's `ClientCapabilities` has no such field (#6501) (@houko)
- Emit a `librefang_media_understanding_failures_total{kind,provider,model}` counter (with a matching structured `warn!`) whenever vision description or audio transcription fails, so a hosted model that a provider silently retires — e.g. Groq removing the hardcoded default `meta-llama/llama-4-scout-17b-16e-instruct` — surfaces as an actionable metric instead of a days-later user report.
  The counter is incremented at the point of failure inside `librefang-runtime-media` (both the image and audio provider-dispatch paths, plus the empty-result case), so it is captured regardless of caller, and its description is registered in `librefang-telemetry` for the Prometheus exporter.
  The hardcoded default models are intentionally left unchanged (choosing replacements is an operator call); a comment on `default_vision_model` now documents that these ids rot and points at the new metric (#6538) (@houko)
- Expand the scheduler delivery-target channel-type dropdown from 4 presets to all 25 first-party sidecar channel adapters, so operators picking a `channel` fan-out target can select WeChat, WeCom, Feishu / Lark, DingTalk, Microsoft Teams, Google Chat, Rocket.Chat, Matrix, Mattermost, WhatsApp, QQ, LINE, and the rest by name instead of typing the raw `channel_type` string. The two adapters that map to their own delivery-target tabs (`email`, `webhook`) are intentionally excluded to avoid two UI paths to the same target, and the existing "Custom…" escape hatch still passes through any channel_type the transport accepts (#6476) (@houko)
- Edit `HAND.toml` online from the Hands panel: the read-only manifest viewer now has an Edit / Save / Cancel affordance backed by a new authenticated `PUT /api/hands/{id}/manifest` that validates the submitted TOML by parsing it into a `HandDefinition` (rejecting invalid TOML or a changed `id` with a 400 and leaving the on-disk file untouched), runs the same supply-chain audit as the install path, persists the file to whichever on-disk copy the hand loads from, and hot-reloads the in-memory definitions — an already-active hand instance keeps its old manifest until it is deactivated and reactivated, and edits to a built-in (registry) hand are overwritten on the next registry sync (#6478) (@houko)
- Slack sidecar multi-step task-progress display: the generic AgentPhase lifecycle (Thinking → ToolUse{name} → Done/Error) now reaches the Slack adapter as `reaction` commands — Slack declares the existing `reaction` capability rather than a new one, since that is the capability `ChannelAdapter::send_reaction` already dispatches the phase lifecycle through — and is rendered as an updated-in-place Block Kit step list via `chat.update` for multi-step turns, while single-step turns keep the prior eyes → white_check_mark receipt reactions (both honour `SLACK_REACTIONS`); the `reaction` command gained optional `phase` / `tool_name` fields (backward-compatible, omitted when empty) carrying the lifecycle detail the emoji alone drops, and the bridge now also emits `ToolUse` phases on the non-streaming dispatch path so a non-streaming adapter that declares `reaction` sees the full `Thinking → ToolUse… → Done/Error` sequence (#6451) (@houko)
- Add opt-in completion notification for `process_start` background processes via the #4983 async-task tracker: a call with `notify_on_completion: true` registers a new `TaskKind::Process { pid }`, and when the process exits on its own or is killed the kernel injects a `TaskCompletionEvent` (exit code + tail of the captured output) back into the originating session, so a long-running process no longer has to be polled to learn it finished — the same delivery path (mid-turn injection / wake-idle) and `[async_tasks]` config that workflow / delegation tracking already use, with no new config surface (#6471) (@houko)
- Add opt-in completion notification for `process_start` background processes via the #4983 async-task tracker: a call with `notify_on_completion: true` registers a new `TaskKind::Process { pid }`, and when the process exits on its own or is killed the kernel injects a `TaskCompletionEvent` (exit code + tail of the captured output) back into the originating session, so a long-running process no longer has to be polled to learn it finished — reusing the same delivery path (mid-turn injection / wake-idle) as workflow / delegation tracking with no new config surface; note the `[async_tasks]` `default_timeout_secs` auto-kill is deliberately NOT applied to processes (a background server is meant to run indefinitely), only the completion-delivery machinery is shared (#6471) (@houko)
- Expose per-channel `dm_policy`, `group_policy`, `threading`, and `output_format` on `[[sidecar_channels]]`, restoring the channel-level override slot that was lost in the sidecar migration (#6445): each is `Option<_>`, so `overrides_from_sidecar_config` projects only the fields an operator explicitly set and an unset knob never materializes a policy they did not write. Precedence is unchanged — agent-level `[channel_overrides]` still wins over the per-channel value (#6468) (@houko)
- Per-user LLM provider credentials, end-to-end (#6460): a human user can store their own upstream provider API key encrypted in the existing credential vault (`CredentialVault`, AES-256-GCM, keyed by `LIBREFANG_VAULT_KEY` / OS keyring) under a per-user, per-provider namespace via `LibreFangKernel::{set,get,remove,list}_user_provider_key`, and their agent turns now bill that key. The authenticated `AuthenticatedApiUser.user_id` is read at the `/api/agents/{id}/message` and `/message/stream` handlers and threaded as `owner: Option<UserId>` through the kernel send/streaming/ephemeral entry points into `resolve_driver_for_owner`, whose precedence is (highest first) org allowlist (#6459) > agent-pinned `api_key_env` > user-scoped key > credential pool > operator `auth_profiles`/`provider_api_keys` rotation > catalog/convention env. The same owner-key preference is applied to every fallback-chain slot, so a provider failover cannot silently bill the operator's credential (a chargeback leak). Paths without a single authenticated initiator — channel messages, cron fires, agent-to-agent sends, and forks/sub-agents — pass `owner = None` and fall back to the daemon-global credential; the fork path additionally clears any inherited owner defensively so a sub-agent's spend is never mis-attributed to the parent turn's user. Global-only behaviour is byte-identical when no user key exists, the plaintext value is never returned through the API (listing surfaces provider names only), and per-owner spend is queryable via the existing `/api/budget/users` rollup. The HTTP/dashboard management surface for provider keys remains a follow-up (#6460) (@houko)
- Owner-gated HTTP surface for the per-user provider credentials above (#6460 Follow-up B): `PUT /api/users/{name}/provider-keys/{provider}` stores a user's upstream key, `DELETE /api/users/{name}/provider-keys/{provider}` removes it, and `GET /api/users/{name}/provider-keys` lists the provider NAMES a user has configured — never any secret value, since the kernel's plaintext getter stays `pub(crate)` and is deliberately absent from the `KernelApi` trait the HTTP layer calls through; writes are Owner-only via the existing `is_owner_only_write` `/api/users/` prefix gate (the same gate that guards create / delete / rotate-key) and the list GET is Owner-gated in `min_role_for_privileged_get` (mirroring `/api/config/export`) so an Admin cannot enumerate another user's provider layout, the `provider` segment is validated against the canonical `known_providers()` registry (rejecting empty / `/`-containing / unknown names with 400), and an unknown user name yields 404 so a typo never orphans a vault entry (#6460) (@houko)
- Let the master API credential live somewhere other than cleartext in `config.toml`, via a new `api_key_hash` and by routing `api_key` through the same env / `vault:` resolution the dashboard credentials already used.
  `config.toml` sits inside the daemon's own writable data dir and gets rewritten by the daemon, so it can never be a read-only Kubernetes Secret mount — `LIBREFANG_API_KEY` and `api_key = "vault:name"` are the two ways to keep the working secret out of the file entirely, and both are now resolved per auth snapshot rather than once at boot, so `POST /api/config/reload` no longer clobbers the override by re-reading the file from disk.
  `api_key_hash` holds `$sha256$…`, produced by the new `librefang hash-api-key` command, and `$argon2id$…` stays accepted for a hand-written value or a deliberately short human-memorable key.
  SHA-256 is the recommended form here and Argon2id remains the one for `dashboard_pass_hash`, which reads like an inconsistency and is not: a dashboard password is human-chosen, so the memory-hard KDF is what makes an offline dictionary attack uneconomic, while a master API key is a machine-generated bearer where there is no dictionary to enumerate — the KDF buys nothing against an offline attacker and instead charges ~50–100 ms of CPU to every request, including every wrong token from an unauthenticated caller on paths that have no login-attempt limiter.
  An `$argon2id$` master hash is therefore verified on a blocking thread rather than inline, so the format an operator chooses can never stall the async runtime.
  Existing plaintext deployments keep working and get a `$sha256$` hash written to a 0600 `api-key-hash.upgrade-hint` file on first authentication, mirroring the `dashboard_pass` → `dashboard_pass_hash` path; clients keep sending the same key, only the daemon's stored copy changes.
  The hash is never logged, because it is the verifier — anyone who could read it out of the log stream could paste it into their own config and authenticate.
  Reloading `api_key` or `api_key_hash` now reaches the HTTP middleware without a daemon restart: `api_key_lock` was previously written only at boot and on a dashboard credential change, so an edited master key kept authenticating with the old value (#6613) (@houko)
- Attach to browser-level CDP endpoints by creating a target and attaching to it, so a `cdp_endpoint` pointing at a browser-level WebSocket works instead of dying on the first command.
  `attach()` sent `Page.enable` immediately, which holds only for a page-level endpoint; against a browser-level one no page exists yet, so the session died at startup — Lightpanda reports this as `BrowserContextNotLoaded`, Chrome as `'Runtime.enable' wasn't found`.
  On a `ws://` endpoint librefang now asks `Target.getTargetInfo` which shape the endpoint is and, only when it answers `type: "browser"`, issues `Target.createTarget`, follows with `Target.attachToTarget` using `flatten: true`, and stamps the returned `sessionId` onto every later command so the browser routes it to that target.
  Reading the kind off the protocol rather than inferring it from a failed command is deliberate: Chrome accepts `Target.createTarget` on a page-level connection too and opens a second tab, so anything short of a definite answer would risk moving a configuration that points at a specific page onto a blank one.
  Anything other than `browser` — including an endpoint that does not implement `Target.getTargetInfo`, and any failure of the query itself — stays on the page-level path, reconnecting first so a server that drops the socket on an unknown method cannot leave the caller worse off than before the query was sent.
  A dropped CDP socket now fails the commands still waiting on it instead of leaving them to time out: the reader loop answers them with `CDP connection closed` when it exits, so a dead connection is reported as itself rather than as a 30-second stall per in-flight command.
  Both cleanup branches are pinned by tests that fail when the close call is neutered, since a leaked tab is invisible to an assertion on the returned error alone.
  A tab is now closed on a failed attach whichever way it was created — `Target.closeTarget` for one created over CDP, `/json/close/{id}` for one discovered over HTTP — since both leave a tab that no session will ever track.
  A target created this way is closed with `Target.closeTarget` rather than the `/json/close/{id}` route used for HTTP discovery, which does not exist on a `ws://` endpoint, and a handshake that fails partway closes the target it already created rather than abandoning a blank tab that nothing would reap (#6617) (@nevgenov)
- Make the page-extraction cap operator-configurable as `[browser] max_content_chars`, defaulting to the 50,000 characters it was hard-coded at, and report the pre-truncation length in the marker so the model can tell how much it is missing rather than only that something was lost.
  A mainstream Wikipedia article overruns the old cap, and the compile-time constant was the only extraction-adjacent limit `BrowserConfig` did not expose while carrying `timeout_secs`, `idle_timeout_secs`, `max_sessions` and the viewport dimensions as knobs — despite being the one most coupled to a deployment-specific fact the project cannot know, the context window of the model behind the agent.
  The marker now counts against the cap: the script cuts far enough back that content plus `... (truncated, N chars total)` lands within the limit, so an operator sizing the value to a context window gets a real ceiling rather than one the marker silently overshoots.
  `EXTRACT_CONTENT_JS` is no longer a `LazyLock` — a process-wide singleton over a config-dependent value would serve whichever cap the first extraction observed to every later one, so the script is built per call from a `str::replace` on a ~2 KB template, on a path that then makes a CDP WebSocket round trip.
  Like every other `[browser]` field the value takes effect on daemon restart, because `BrowserManager` captures `BrowserConfig` by value at boot (#6687) (@houko)
- Add per-PR changelog fragments under `changelog.d/`, so writing a changelog entry no longer means editing the one file every other open PR is also editing.
  Every PR appended its bullet to the single `## [Unreleased]` section of `CHANGELOG.md`, which made a merge conflict certain between any two concurrent PRs and carried no information when it happened — both sides were correct and the resolution was always "keep both".
  It bit hardest on fork PRs, where the maintainer cannot rebase the contributor's branch at all.
  A fragment is one file holding one bullet body without the leading `- `, in the section directory matching its `### ` heading (`added/`, `fixed/`, `changed/`, `security/`, `documentation/`), so two PRs never touch the same file and the conflict is structurally impossible rather than merely rarer.
  `cargo xtask collect-fragments` folds fragments into `## [Unreleased]` and deletes the files it consumed; `cargo xtask release` runs that step before cutting the dated release section, which is what keeps the `awk` extractors in `release.yml` and `release-notify.yml` slicing an unchanged file shape.
  Assembly **appends** to a `### ` subsection that already exists rather than replacing it, and creates a missing one in the repo's existing order, so editing `## [Unreleased]` by hand keeps working and the PRs already doing that are unaffected — the existing 160 bullets are deliberately not migrated, since converting them would rewrite the exact lines those PRs are conflicting on.
  Fragments are sorted by file name within each section rather than read in filesystem order, because an unsorted directory read would make the assembled file depend on the order the fragments happened to be created in.
  A fragment that cannot be deleted after the fold fails the command by name rather than propagating a bare `Permission denied`, because `CHANGELOG.md` is already written by then and a silent partial delete would make the next run fold the same entry in twice.
  `scripts/check-changelog-attribution.py` holds a fragment to the same standard as an `[Unreleased]` bullet in all four of its modes, reusing the one `bullet_block_has_attribution` predicate so there is a single copy of the `(@user)` rule, and additionally rejects a fragment in an unrecognised section directory — assembly has no heading to render such a fragment under, so it would be dropped without a word and the entry would vanish from the release notes.
  The `pre-commit` hook's attribution check now also fires when a commit stages only a fragment, which is the normal case and would otherwise have gone entirely unchecked.
  The release commit stages `changelog.d` as a directory so the deletions the fold performs land in the commit; a per-file stage is a no-op for a path that no longer exists, and leaving them unstaged would keep the consumed fragments on `main` and fold the same bullets in again at the next release (#6628) (@houko)
- Add `GET /api/ready`, a public readiness probe that returns 503 when a dependency required to accept work is unavailable.
  `GET /api/health` could not serve this purpose: it returns 200 even while its body reports `status: degraded`, so a Kubernetes probe — which sees only the status code — could never remove a degraded pod from Service endpoints.
  Changing `/api/health` itself would have conflated liveness with readiness and restart-looped pods through recoverable storage incidents, so the two contracts are now separate endpoints (#6633) (#6638) (@houko)
- Add an officially supported single-replica Kubernetes deployment under `deploy/kubernetes/`, as Kustomize manifests plus operator documentation.
  The repository previously shipped Docker Compose only, leaving every operator to invent their own StatefulSet — and to rediscover on their own that SQLite WAL on shared storage and `replicas: 2` both corrupt state.
  `scripts/check-k8s-manifests.py` asserts the properties that fail silently when they regress (`replicas: 1`, the liveness/readiness split, `ReadWriteOnce`, credentials from Secrets rather than literals), and a new CI workflow boots the manifests in kind under enforced `restricted` Pod Security and proves `/data` survives pod replacement (#6635) (#6638) (@houko)
- Add `LIBREFANG_API_KEY` as an environment override for the API bearer token.
  `config.toml` lives inside the daemon's own writable data dir and is rewritten at boot, so it cannot be mounted from a Kubernetes Secret — leaving no way to supply `api_key` without baking the literal into an image.
  An empty value is ignored with a warning rather than treated as "clear the key", because a Secret key that exists but is unset would otherwise disarm bearer authentication on a non-loopback bind (#6635) (#6638) (@houko)
- Auto-detect a locally installed and logged-in EveryAPI CLI and expose it as an LLM provider without ever copying its relay key into a LibreFang-owned file.
  Credentials are resolved per request through EveryAPI's own credential-process command and refreshed once after an HTTP 401, so EveryAPI remains the authority for key selection, OAuth refresh, and region resolution.
  Explicit provider keys, URLs, and user suppression still take precedence over auto-detection, and `librefang doctor` reports the detected wiring and any conflicting configuration (#6641) (@houko)
- Add official EveryAPI partner links and documentation across the website footer/CTA, README, dashboard sidebar, and docs navigation, in English and Chinese.
  Correct the EveryAPI provider guide to use `everyapi auth login` and the current credential-process discovery flow, and point EveryAPI CLI references at the public `everyapi-ai/everyapi-ai` repository (#6646) (@houko)
- Thread the `language` and `prompt` parameters from `media_transcribe` / `speech_to_text` through to the multipart form Whisper-compatible providers actually receive, instead of reading and discarding them.
  `language` was advertised in both tools' schema and silently dropped, so the provider always fell back to auto-detection regardless of what the caller asked for — a misdetected language does not error, it returns fluent, plausible, wrong text.
  `prompt` is genuinely additive: it supplies domain vocabulary and proper nouns the model would otherwise transcribe as phonetic near-misses, and improves punctuation and casing on long recordings.
  Both follow the precedent `tool_text_to_speech` already used for `language`: the per-call value wins, and `[media] audio_language` / `audio_prompt` are the new operator-configured fallback for calls that omit either field.
  Only the whisper-protocol provider arms (Groq, OpenAI, MiniMax, Fireworks, Together, SiliconFlow, and any `[media.custom_stt]` self-hosted endpoint) receive these — Gemini and ElevenLabs are separate provider contracts with no equivalent parameter.
  An install that sets neither field sees a byte-identical request to before either parameter existed (#6678, #6683) (@houko)

### Fixed

- Drive the dashboard's per-channel status indicator from the sidecar supervisor's real liveness instead of from message traffic, and surface the fields it reads on `GET /api/channels`.
  The indicator was `msgs_24h > 0 ? "running" : "idle"`, so a healthy-but-quiet channel rendered grey and a channel that died after handling messages rendered green; `ChannelStatus.connected` and `last_error` were maintained by the supervisor all along but were not present on the payload at all.
  Configured rows now carry `connected`, `started_at`, `last_message_at`, `messages_received`, `messages_sent`, `last_error`, and a `supervised` flag that says whether an adapter is registered for that instance name at all, all read per sidecar instance.
  The card maps those onto seven states — not started, starting, connected, active, degraded, stopped, failed — with the state spelled out as visible text and folded into the card's `aria-label`, since the card's own label otherwise overrides its contents for assistive tech and the colour would be the only carrier of meaning.
  `connected` with a `last_error` reads as degraded rather than healthy or dead, because the supervisor sets `last_error` on failure and never clears it, not even on the successful respawn that follows.
  A configured channel with no registered adapter reads as amber rather than grey: grey is what made a dead bot look benign in the first place, and the API layer genuinely cannot distinguish "start failed and the registration was rolled back" from "never started".
  The mapping lives in a shared `src/lib/channelLiveness.ts` so the Comms page's channel cards render the same verdict from the same payload rather than repainting config presence as an online badge.
  The `librefang channel list` table gains CONNECTED and IN/OUT columns plus a per-channel error footnote, fed the raw supervisor fields rather than a second copy of the state mapping (#6606) (@houko)
- Stop presenting the channels page's 24h message count as per-bot traffic when it is a per-channel-type aggregate.
  `usage_events.channel` stores the channel *type*, so the handler's `msgs_24h.get(channel_type).or_else(|| msgs_24h.get(name))` always hit on the first lookup and the per-instance fallback was unreachable: on a host running six Telegram sidecars every card reported the same number, the total across all six, and because the status dot was derived from it the whole page turned green whenever any one bot saw traffic.
  Re-keying that column per instance is not available: it is written from `SenderContext.channel`, which the bridge derives from the `ChannelType` on the inbound message (the instance name never reaches it) and which also feeds `SessionId::for_channel(agent, channel)` and the auth `identify(&channel, …)` binding, so re-pointing it at the instance name would silently re-derive every existing channel session.
  So the figure is now published as `msgs_24h_channel_type` alongside the `channel_type` it covers, the unreachable fallback is gone, and the dashboard shows it in the details drawer captioned with its actual scope instead of on the card where it read as this bot's traffic.
  Per-instance traffic comes from the supervisor's own `messages_received` / `messages_sent` counters, labelled as since-adapter-start because they survive supervised restarts and are not a 24h figure.
  `UsageStore::channels_msgs_24h_bulk` is renamed to `channel_type_msgs_24h_bulk` so the grouping is not misread again at the call site (#6606) (@houko)
- Render a failed `GET /api/channels` as an error on the channels page instead of the "no channels configured" empty state, which made an unreachable daemon look like a clean install on the page whose whole purpose is now health signalling (#6606) (@houko)
- Merge instead of replace in `PATCH /api/agents/{id}/identity`, so a partial body no longer nulls the identity fields it omits.
  The handler built a fresh `AgentIdentity` from the request alone with no read of the stored one, so `PATCH {"emoji": "X"}` silently discarded `avatar_url`, `color`, `archetype`, `vibe` and `greeting_style` and returned `200` — while the sibling `PATCH /api/agents/{id}/config`, which writes the same six fields, merged them correctly.
  Two PATCH endpoints on one resource with opposite semantics is the actual defect, so the six-field merge is now a single shared `merge_agent_identity` helper that both handlers call rather than a copy in each; an integration test asserts the two routes produce the same stored identity for the same partial body.
  Neither route can set a field back to `null`, since `null` already means "not provided"; sending an empty string stores an empty string, which is the closest thing to clearing one, and that is now stated in the endpoint's OpenAPI description (#6608) (@houko)
- Name the `tool_allowlist` entries that cannot take effect in the response of `PUT /api/agents/{id}/tools`, instead of accepting them silently.
  The kernel applies `tool_allowlist` after `capabilities.tools` as a `retain`, so it can only narrow: an entry naming a builtin or skill tool the declared set excludes grants nothing, yet the write returned `200`, a subsequent `GET` round-tripped the value, and the only way to discover the tool was never granted was to inspect a session export.
  The resolution semantics are deliberate and unchanged — the response now carries an optional `warnings` array naming each inert entry, and the OpenAPI description for the endpoint and for the field says that `tool_allowlist` narrows rather than grants.
  A request that submits only `capabilities_tools` is checked too, because narrowing the grant surface is itself a way to silence a stored allowlist entry, and the operator who just issued that request is the one who needs to hear about it; a request that touches neither field (blocklist only) stays quiet about whatever was already stored.
  Only provably inert entries are reported, because a false warning on a working configuration would be worse than the silence: the check is skipped entirely when `capabilities.tools` is unbounded (empty or `*`), and it never flags a glob (a later skill install or MCP connect can make it match), an `mcp_`-namespaced entry (MCP tools bypass `capabilities.tools`), or a self-evolution tool (injected regardless of what the manifest declares).
  The declared side is glob-evaluated rather than string-compared, so `capabilities_tools = ["file_*"]` with `tool_allowlist = ["file_read"]` is correctly left alone (#6609) (@houko)
- Render every `approval_audit.decision` value distinctly in the dashboard's Approvals History table instead of labelling four of the six values it can hold "Edited".
  `ApprovalsPage.tsx` branched on `approved` / `approve` and `rejected` / `reject` and let everything else fall through to a yellow pencil "Edited" badge — the same rendering as a genuine modify-then-approve — so on the reporter's host 46 of 56 audit rows (28 `pending`, 18 `timed_out`) claimed an operator edit that never happened.
  `timed_out` means nobody answered before the timeout expired and `pending` is the submission marker written before any decision exists; on an approval audit trail those are the opposite of the operator involvement the badge asserted.
  A `pending` row is not evidence that the request is still open — resolution inserts a second row instead of updating the first one — so every one of the reporter's 28 `pending` rows belongs to a request that has since closed; that data-model quirk is untouched here and raised on the PR.
  `denied` was mislabelled by the same fall-through, which the issue does not mention: `ApprovalDecision::as_str` writes `denied`, but the branch only matched the `rejected` / `reject` spellings that `routes/approvals.rs` uses on sibling shapes, so every genuinely-denied request rendered as an operator edit too — of the six values the daemon can write, only `approved` and (coincidentally) `modify_and_retry` were labelled correctly before this change.
  Each of the six values the daemon writes now carries its own label, icon and theme colour — `timed_out` neutral with a clock, `pending` in-progress, `modify_and_retry` the actual "Edited" case, `skipped` distinct — and every state pairs that colour with label text and an `aria-label` naming the decision, so the trail is readable without colour perception.
  An unrecognised value renders the raw string (or an explicit "unknown status" when the field is empty) rather than borrowing another decision's label, so a variant a newer daemon adds degrades visibly instead of becoming a false record.
  The frontend type is narrowed from `string` to a `KnownApprovalDecision` union plus an explicit escape for unknown values, the presentation table is a total `Record` over that union so adding a member without giving it a label is a compile error, and the entry interface picks up the `second_factor_used` field the Rust struct already carries.
  `ApprovalAuditEntry::decision` stays a `String` on the Rust side deliberately: the column is read back for rows written by any past version and `query_audit` drops rows it fails to deserialize, so a strict enum would silently shorten a security audit trail on one legacy value — the value set is documented on the field and pinned by a test instead (#6607) (@houko)
- Expose `external_auth.require_email_verified` and the six missing `OidcProvider` fields (`auth_url`, `token_url`, `userinfo_url`, `jwks_uri`, `audience`, and the per-provider `require_email_verified` override) in `GET /api/config`, while keeping every one of them non-writable.
  `require_email_verified` is the #3703 mitigation — it rejects a login whose ID token does not carry `email_verified = true`, which is what stops an unverified address in an `allowed_domains` domain from inheriting that domain's authorization — and it is deliberately absent from the `POST /api/config/set` allowlist so an Owner-role caller with a leaked API key cannot switch it off.
  Omitting it from the read side too was the wrong asymmetry: an operator had no way to confirm the protection was active without shell access to read `config.toml`, and with the provider endpoint overrides hidden as well, a non-OIDC provider's explicit `auth_url` / `token_url` / `jwks_uri` were invisible from any surface.
  Nothing newly exposed is secret-bearing: `client_secret_env` names the environment variable the secret is read from and `client_id` is the public half of the client registration, both of which were already emitted, and the secret itself never lives in config.
  The read/write parity guard added in #6604 cannot catch this class, because it enforces `writable ⊆ readable` and a field that is intentionally non-writable sits outside that invariant by construction (#6605) (@houko)
- Remove `ui.theme`, `ui.locale`, `ui.timezone`, and `ui.language` from the `POST /api/config/set` allowlist, where they had accepted writes that were silently thrown away since #4113.
  `KernelConfig` has no `ui` field and never had one, so the write path validated the dotted path against the allowlist, edited `config.toml` through `toml_edit` keyed by it, and dropped a `[ui]` table on disk that the next load discarded — `KernelConfig` does not set `deny_unknown_fields`, so neither the post-edit parse nor the reload could reject it.
  The caller received a success status for a change that was never applied and never read back, and `GET /api/config` reported `ui` as null.
  Nothing ever posted them: the dashboard keeps theme, language, and sidebar state in browser `localStorage` through zustand's `persist` middleware (key `librefang-ui-storage`), which is why four dead paths went unnoticed for three months.
  The unit test that pinned them as writable now pins them closed (#6605) (@houko)
- Add `every_writable_allowlist_entry_has_a_backing_config_field`, the mirror of #6604's read/write parity guard: every entry in the `POST /api/config/set` allowlist must name a field that actually exists on `KernelConfig`.
  The parity guard derives its candidate paths *from* a serialized config, so a path naming a field that does not exist is structurally invisible to it — which is why the four `ui.*` entries survived it.
  The oracle is the schemars-derived JSON Schema rather than a serialized config value, because a value walk cannot see a field whose `#[serde(skip_serializing_if = …)]` predicate holds for its default and 63 of those attributes exist in `config/types.rs`, several on writable paths (`exec_policy.allowed_env_vars`, `budget.providers`, `tool_invoke.allowlist`), so a value-based oracle would report real fields as dangling.
  The guard needs no exclusions — with the four `ui.*` entries gone, all 29 remaining exact paths and all 54 section prefixes resolve — and it carries the same sanity floor as its sibling plus negative controls asserting the resolver still rejects `ui.theme` and an invented section, since the failure mode of a path resolver is over-permissiveness rather than emptiness.
  The two allowlists moved from `const` items inside `is_writable_config_path` to module scope so the guard reads the real lists instead of restating them (#6605) (@houko)
- Share one tempfile sequence between the two `config.toml` writers in `routes/sidecar_toml.rs`, so a concurrent channel configure and remove can no longer clobber each other's write.
  `upsert_sidecar_block` and `remove_sidecar_block` each declared a function-local `static SEQ: AtomicU64 = AtomicU64::new(0)` while both formatted into the same `.config.toml.tmp.{pid}.{seq}` namespace, so the first call to either in a process minted the identical path and the two could write and rename each other's tempfile — landing one request's document at the other's target or losing it outright.
  The comment above the first counter already claimed it "guards against concurrent threads within this process (e.g. parallel tests, or two HTTP handlers racing on the same config file)", which is the guarantee two independent counters cannot provide.
  Both writers now go through one `atomic_write` helper backed by a module-level counter, and a test draws names concurrently from eight threads and asserts they are all distinct (#6605) (@houko)
- Store the EveryAPI gateway base URL with its `/v1` segment when connecting from the dashboard, matching what the CLI writes.
  `EVERYAPI_PROVIDER.defaultBaseUrl` was `https://api.everyapi.ai` on a doc comment claiming the drivers append the path themselves, which no driver does: the OpenAI-compatible driver builds `{base_url}/chat/completions` and the daemon's catalog refresh builds `{base_url}/models`.
  A dashboard connect with the gateway field left blank therefore registered a provider whose model fetch 404s, and because that failure is only `warn!`-logged and then throttled per base URL, the entry sat configured-with-zero-models indefinitely while the identical `librefang models connect everyapi` flow worked.
  The connect action landed in #6586 (#6602) (@houko)
- Refresh the EveryAPI catalog on both channel read paths, which #6583 left unwired.
  `list_models_by_provider` and `list_models_text` in `channel_bridge.rs` each refreshed for `openrouter` and fell through for `everyapi`, so a Telegram or Slack user listing or picking a model saw whatever snapshot the catalog last happened to hold while every `providers.rs` read path refreshed correctly.
  `list_models_text` spans every provider and so refreshes both catalogs unconditionally; the per-provider picker only refreshes the one being asked about (#6602) (@houko)
- Apply `tool_allowlist` when rendering MCP tool grants in the dashboard's Tools tab.
  The tab mirrored `tool_blocklist` only, but the kernel's Step 4 filter runs after MCP tools have joined the candidate set, so a non-empty allowlist naming no `mcp__*` glob strips the entire server (#6495).
  An allowlist-filtered server rendered as fully granted — the display asserted an agent could call tools the kernel had already removed.
  The Tools tab MCP display landed in #6578 (#6602) (@houko)
- Stop following symlinks in the two skill installers #6581 left untouched.
  That PR hardened `librefang_skills::marketplace::copy_dir_recursive` but the API route's copy helper and the CLI's `copy_dir_recursive` read the same registry checkout, the former dereferencing links through `std::fs::copy` and the latter branching on `Path::is_dir()`, which follows them.
  A symlink planted in a registry skill therefore still copied the target's real contents — or an entire external tree — into the installed skill (#6602) (@houko)
- Refuse `service install --system` on macOS while a per-user LaunchAgent is still installed.
  Both carry the `ai.librefang.daemon` label and the same state directory, so installing the LaunchDaemon over the agent produced a login-time respawn loop: the agent's `start --foreground` finds the daemon already holding the port, exits non-zero, and `KeepAlive` relaunches it.
  The check runs before the ownership handover and the plist write, so a refused install leaves nothing behind.
  `--system` landed in #6584 (#6602) (@houko)
- XML-escape the paths interpolated into both macOS plists.
  APFS permits every byte but `/` and NUL, so a volume named `Backup & Media` or a `LIBREFANG_HOME` containing `<` produced an ill-formed plist that launchd rejects wholesale, while the install path still reported it as written.
  The LaunchDaemon renderer landed in #6584 and the LaunchAgent one predates it (#6602) (@houko)
- Correct the NixOS module's non-loopback bind assertion, which accepted `environmentFile != null` as proof of authentication.
  No environment variable feeds `api_key` — it is read from `<stateDir>/config.toml` only — so the documented off-host recipe, whose environment file holds provider keys, passed `nixos-rebuild` and then deployed a unit the daemon refuses to start.
  The message now names the only credentials the unit environment can actually supply (`LIBREFANG_DASHBOARD_USER` / `LIBREFANG_DASHBOARD_PASS`), a new `authConfiguredExternally` option covers a `config.toml` maintained out-of-band, and the `LIBREFANG_ALLOW_NO_AUTH` disjunct now checks the value against the same five spellings `allow_no_auth_env()` accepts instead of merely testing for the key's presence.
  The module landed in #6582 (#6602) (@houko)
- Stop hand activation from fabricating a `mode = "full"` shell exec policy for every hand whose tool list contains `shell_exec`.
  `activate_hand_with_id` hardcoded `ExecSecurityMode::Full` whenever the hand's agent section declared no `[exec_policy]` of its own, so activating a marketplace hand silently granted unrestricted shell execution and — because `Full` also short-circuits the approval queue in `tool_runner::dispatch` — skipped the operator's `require_approval` globs entirely.
  The materialized policy is now inherited from the live global `[exec_policy]`, mode included; a hand that genuinely needs elevated exec still opts in by declaring its own `[exec_policy]`, which activation continues to respect verbatim.
  **This changes behaviour on a stock install, not only on a hardened one.** `ExecPolicy::default()` is `mode = "allowlist"` with an empty `allowed_commands` and 18 read-only `safe_bins` (`sleep`, `cat`, `head`, `wc`, `date`, `echo`, …), so a daemon with no `[exec_policy]` section at all now gives its hands that allowlist rather than unrestricted shell.
  A shell-using hand such as `devops` will therefore find `docker`, `kubectl`, `git`, and `cargo` rejected until the operator either adds them to `[exec_policy] allowed_commands` (or widens `mode`) or the hand declares its own `[exec_policy]` — the trade-off is deliberate: an operator who never wrote a shell policy has not consented to unrestricted third-party shell execution.
  The historically generous 300s / 120s timeouts survive as a floor rather than being replaced by the 30s `ExecPolicy::default()` values, because a timeout is not a security property and inheriting them wholesale would cut long-running hand commands short for reasons unrelated to the fix.
  The guard reads `capabilities.tools` and matches the `*` wildcard as well as `shell_exec`, because `spawn_agent_inner`'s own fallback promotes any manifest that still carries no `exec_policy` — leaving one unset on a manifest that fallback would match relocates the escalation rather than removing it.
  Activation also logs the resolved mode and where it came from, so an escalation is visible at activation time instead of only through the per-call warning much later.
  Scope: this covers the hand-activation path only, which leaves a divergence worth knowing about.
  `spawn_agent_inner` and the boot-time restore loop still promote a *standalone* agent to `Full` on the identical trigger, so post-fix the same `shell_exec` tool is unrestricted on a standalone agent and allowlisted on a hand agent; that promotion is deliberate today (it has its own regression test) and changing it needs an `allowed_commands` migration story, so it is left for a separate decision (#6594) (@houko)
- Require an explicit declaration before a hand's agent gets a wake-up cycle, instead of inferring autonomous ticking from `max_iterations`.
  `max_iterations` is the agent-loop iteration cap — `librefang-runtime` resolves it from `manifest.autonomous.max_iterations`, and the flat HAND.toml field exists only to carry it — but activation turned `autonomous.is_some()` into `ScheduleMode::Continuous` with `heartbeat_interval_secs` as the interval, so a hand asking for a loop-depth cap of 80 got a permanent 30-second wake-up cycle it never declared and could not find in any file.
  A role now leaves `reactive` only if its own section says so: an explicit `schedule`, honoured verbatim and still the only route to the `periodic` cron and `proactive` variants, or an explicit `[autonomous]` block, which schedules the role continuously at that block's own `heartbeat_interval_secs`.
  A role declaring nothing but `max_iterations` — at any `[metadata] frequency` — stays reactive and keeps its cap, and `[metadata] frequency` goes on being catalog-display metadata that the kernel does not read.
  Distinguishing an author-written `[autonomous]` block from the one synthesized to carry `max_iterations` is impossible after deserialization, since both land in the same `Option<AutonomousConfig>`, so the decision moved into `librefang-hands` where the raw TOML table is still available and an `autonomous` key means somebody typed one; the schedule-rewriting block in `hands_lifecycle.rs` is deleted outright, leaving hand roles to honour `schedule` exactly the way `spawn_agent_inner` already honours it for a standalone agent.
  **Hands that were implicitly ticking will stop, and that is a behaviour change, not only a bug fix.**
  Every role that wants to keep its loop needs an explicit `[autonomous]` block (or an explicit `schedule`) added to its HAND.toml; across the bundled registry that is 20 of the 59 roles — every role that carries `max_iterations`, since not one declares an `[autonomous]` block or a `schedule` today — including all four `wiki` roles and both loop-running `devops` roles (`main` and `implementer`).
  `devteam`'s `pm` / `engineer` / `qa` are the only roles that inherit through `base =`, from the `planner` / `coder` / `code-reviewer` templates in the registry repository; none of those three declares `max_iterations`, `[autonomous]`, or `schedule`, so those roles were already reactive and are unaffected either way.
  The shipped registry lives in its own repository, so those manifests are a follow-up there rather than something this change can carry, and `devops`'s own system prompt asserts "The Hand is already `frequency = \"continuous\"`, so this Phase fires once per turn" — prose that needs revisiting alongside the manifest.
  No role in the registry starts ticking: not one of the 59 declares an `[autonomous]` block or a `schedule` today, so the new condition is unsatisfied everywhere and the set of roles with a background loop only shrinks.
  The rule is not a subset of the old one in the abstract — a flat-format role that did write an explicit `[autonomous]` block would newly tick, since that block used to be dropped by the flat-format parse entirely (the bullet below) — but no such role exists to be affected.
  `frequency = "reactive"` also stops being a TOML parse error — #6595 reports an operator reaching for that spelling while hunting the ticks — though as catalog metadata it has no effect on scheduling either way (#6595) (@houko)
- Key the heartbeat monitor off an agent's `schedule` rather than the presence of `[autonomous]`, so an agent that nothing wakes is no longer flagged `BecameUnresponsive` for sitting idle.
  The comment above the check already said "skip passive agents"; `autonomous.is_some()` was only ever a proxy for that, and it held while activation derived `Continuous` from the mere presence of the field.
  Once `autonomous` means "carries a loop cap, and possibly guardrails" — which says nothing about whether the agent wakes itself — the proxy inverts: a role declaring `max_iterations` with no `[autonomous]` block and no `schedule` becomes the common shape, 20 of the 59 roles in the registry, and every one of them would be reported unresponsive for behaving exactly as configured.
  Four pre-existing heartbeat tests built their agents with a default reactive schedule; the two that expected the agent to be checked would have failed, and the two that expected it skipped would have started passing for the wrong reason, so all four now declare a continuous schedule and still exercise what their names claim (#6595) (@houko)
- Carry `schedule`, `[autonomous]`, and `[exec_policy]` through the flat HAND.toml agent format instead of dropping them.
  `parse_single_agent_section` tries `LegacyHandAgentConfig` first for any agent section without a `[model]` sub-table — the shape every hand in the registry uses — and that struct has no `deny_unknown_fields`, so all three keys were deserialized into a struct that did not declare them and vanished with no diagnostic.
  Each is a documented per-hand opt-in, so the opt-in was unreachable for exactly the hands that need it: such a hand could not pin a schedule of its own, could not declare the autonomy it wanted, and could neither tighten nor loosen its exec policy.
  This is what makes the explicit declarations above reachable at all for a flat-format hand, and an explicitly declared `[autonomous]` block now wins over the one synthesized from `max_iterations`.
  One consequence worth knowing: a malformed value under any of the three keys now fails the parse and the hand is skipped by `reload_from_disk`, where it used to be ignored silently — no hand in the registry declares any of them today, so nothing regresses now (#6594, #6595) (@houko)
- Expose every config field on `GET /api/config` that `POST /api/config/set` accepts, so a setting the dashboard just saved stops reading back as "not configured".
  The response body is hand-enumerated section by section, and a long tail of writable fields had never been added to it — among them `browser.enabled` / `browser.cdp_endpoint`, `media.image_model` / `media.custom_stt`, `tts.custom`, `approval.totp_grace_period_secs`, `web.timeout_secs`, `exec_policy.allowed_env_vars`, and the whole `[terminal]` section, which the dashboard declares a page for but could never populate.
  The `[channels]` file-transfer limits are added in the same pass without being part of that count: `channels.` accepts depth-2 paths only, so its depth-1 scalars are edit-on-disk, but the dashboard declares a `channels` section and had nothing to render in it.
  Deriving the body from `Serialize` instead was rejected on evidence rather than effort: the dashboard's chat page reads `media.stt_available` and `web.search_available`, keys no serializer would ever emit, and the body redacts secrets in place (`api_key`, `network.shared_secret`, `vertex_ai.credentials_path`) rather than dropping them, so a wholesale switch would have broken working UI and needed an exhaustive deny-list to stay safe.
  The hand-written shape is kept and a guard added instead: `config_read_write_parity_tests` walks a serialized `KernelConfig`, filters the paths through the real `is_writable_config_path` — not a copy of its allowlists, which would become the next thing to drift — and fails the build for any writable leaf the read path omits.
  `redacted_config_json` is split out of the handler so that guard runs without booting a kernel (#6596) (@nevgenov)
- Emit the serde spelling rather than the Rust variant name for enum-valued config fields on `GET /api/config`, so their dashboard dropdowns show the configured value instead of appearing unset.
  `mode`, `reload.mode`, `exec_policy.mode`, `broadcast.strategy`, `docker.mode`, `docker.scope`, and `web.search_provider` were rendered with `format!("{:?}", …)`, which yields `Allowlist` and `DuckDuckGo` where `config.toml`, the write path, and the schema's own `select` options all say `allowlist` and `duck_duck_go`.
  The dashboard matched the value against the options it had just been handed, found nothing, and rendered an empty control for a field that was set.
  Four sibling fields in the same handler already went through `serde_json::to_value`, so this restores the house style rather than introducing one; an integration test now rejects any upper-case character in those values, which catches a new field reintroducing `Debug` formatting and not merely the seven that were fixed (#6596) (@nevgenov)
- Correct the API reference for `GET /api/config` and `POST /api/config/set` in both locales.
  The documented response example shared not one key with the real body — it advertised `default_provider`, `listen_addr`, `api_key_set`, and `channels_configured`, none of which the endpoint has ever returned — and the `config/set` example named its field `key` where the handler reads `path`, so a copied request failed with `missing 'path' field`.
  The same example also used a bare table name as the path, which the write allowlist rejects with `403`; it now uses a leaf, and the page states the three-segment limit and the edit-on-disk exclusions (#6596) (@nevgenov)
- Stop `[approval] trusted_senders` from waiving human approval on every tool a connected MCP server exposes.
  `classify_risk` matched a closed list of built-in names, so any `mcp_*` tool fell into the `Low` default and the trusted-sender branch of `requires_approval_with_context` returned before the operator's `require_approval` globs were consulted — the globs were dead config for exactly the tools least able to be audited, since server-side tool code is third-party and its effects are not enumerable from a name.
  Real MCP servers ship tools that move money or take a plaintext credential as an argument, so `mcp_*` now grades as high risk and the bypass stays scoped to the built-ins it was written for; an operator who wants an MCP tool unattended can still allow it per-channel or per-user (#6592) (@houko)
- Refuse to replace a populated EveryAPI provider entry when the gateway's model list cannot be fetched.
  Both persistence paths rewrite `providers/everyapi.toml` wholesale, so a transient outage on a re-run downgraded a working catalog to zero models and still reported success.
  A first run, which has nothing to lose, still proceeds and fills the models in later (#6583) (@houko)
- Distinguish a rejected relay key from an unreachable gateway when fetching the model list.
  Both collapsed into the same outcome, so a revoked key printed "check that the gateway is reachable" and was then saved as if valid; a 401 or 403 now stops before anything is written and points at `everyapi login` (#6583) (@houko)
- Stop the daemon-write path from aborting the process on the failure its own fallback exists for.
  `write_provider_via_daemon` went through a helper whose transport-error arm calls `std::process::exit(1)`, so an unreachable daemon killed the command before the direct file write could run.
  A daemon that parses the payload and *rejects* it is now terminal rather than falling back, because the registry route deletes the file it refused and rewriting it would leave a definition that fails to parse on every boot (#6583) (@houko)
- Pin the gateway base URL into `[provider_urls]` when `--set-default` switches the default provider.
  The default route persists only provider, model and `api_key_env`, while the daemon resolves the boot-time driver from `default_model.base_url` or `provider_urls` several hundred lines before the model catalog that holds the address is built — so the next boot produced a default driver with no endpoint (#6583) (@houko)
- Declare the media capabilities the synthesised EveryAPI provider actually implies.
  `media_capabilities` was hardcoded empty while the same run registered image, audio and video entries, leaving all of them unreachable through the media paths despite appearing in the catalog (#6583) (@houko)
- Rank the `--set-default` choice by capability instead of taking the first model by id.
  Entries are id-sorted for output determinism and ASCII orders uppercase first, so the CLI picked `MiniMax-M3` over `claude-opus-5` while its own help text promised "this gateway's best model" (#6583) (@houko)
- Treat a published context window as evidence of a text model when a gateway row omits `supported_endpoint_types`.
  The empty case defaults to video because every observed empty row is a video model, but the field is optional — a gateway that stopped sending it would have had its chat models registered as video, exempt from the token-limit validation and unusable for chat (#6583) (@houko)
- Point the no-daemon `--set-default` message at re-running connect rather than `librefang models set`.
  That command writes only `default_model.model`, leaving the previous provider and `api_key_env` in place — a combination that resolves the wrong driver for the chosen model (#6583) (@houko)
- Classify an OpenAI-compatible `insufficient_quota` response as a billing error rather than a malformed request.
  OpenAI signals an exhausted account with that code on a 403, so every OpenAI-compatible endpoint does too, but it was not in `BILLING_PATTERNS` and fell through to the generic 4xx arm.
  The operator was told to check their request format while the real cause was an empty account, and because `is_billing` stayed false the long billing cooldown never applied, so a provider with no funds left kept being retried.
  Affects every OpenAI-compatible provider, not just gateways (#6583) (@houko)
- Stop `librefang doctor` reporting a healthy EveryAPI wiring when the relay key is gone.
  The check returned `Pass` as soon as the provider file existed, so `credentials_usable` was never consulted — after `everyapi logout`, a key rotation, or a revocation it stayed green while every request through the gateway failed authentication.
  It now warns and names the remediation, since a provider entry is a file that persists while the credential behind it is not (#6583) (@houko)
- Point skill search and install at the synced registry checkout so the FangHub surface works on a stock install: every remote path targeted `github.com/librefang-skills`, an organization that does not exist, so `librefang skill search` got `422 Unprocessable Entity` (GitHub's answer to an `org:` qualifier naming a missing org) and `librefang skill install <name>` got a 404 for every skill.
  The real source is `~/.librefang/registry/skills/`, synced by `registry_sync` from `librefang/librefang-registry` and forge-agnostic (it honours `registry.registry_host`, so a Codeberg mirror works) — 61 skills, `web-search` among them.
  `MarketplaceConfig` gains a `registry_dir`; `search_registry` scans that checkout (name and description, case-insensitive, sorted) and `install` copies from it before falling back to the GitHub-releases path, normalizing `SKILL.md` into a native `skill.toml` and running the same supply-chain audit as a remote bundle.
  `librefang skill install ./dir` now accepts a `SKILL.md`-only directory, which the registry loader already auto-converted but the CLI rejected with "No skill.toml found", making every registry skill uninstallable by path.
  `librefang skill install <git-url>` is implemented rather than advertised-only: the CLI's own help text documents the form, but a URL fell through to the name-based install, which pasted it into `{org}/{name}` and requested a nonsense URL.
  `GET /api/marketplace/search` reads `SKILL.md` as well as `skill.toml` — it looked only for the latter, so it returned an empty list on every install, unlike the sibling `GET /api/skills/registry` which had already solved this (#6569) (@houko)
- Fix the dashboard's agent Clone button, which failed on every click: `POST /api/agents/{id}/clone` requires `new_name` (no serde default, and the request struct is `deny_unknown_fields`), but the client posted `{}` and nothing in the click handler ever collected a name, so the backend answered 422 and the UI showed a generic "Failed to clone agent" toast.
  Clone now opens a small dialog that pre-fills `<source>-copy`, lets the operator edit the name, and carries the `include_skills` / `include_tools` toggles the endpoint already accepted; `cloneAgent()` and `useCloneAgent()` take the payload instead of posting an empty body (#6566) (@houko)
- Make the agent Tools tab reflect MCP servers granted through the `mcp_servers` allowlist, which it previously reported as "AVAILABLE / click to assign" with 0 tools recognised even while the agent was actively calling those tools.
  The tab derived every group's assigned/available state from `capabilities_tools` alone, but the kernel grants MCP tools through `mcp_servers` and explicitly skips the declared-tools filter for them (`available_tools`, Step 3) — `capabilities_tools` governs builtin tools only, and `tool_blocklist` is the per-tool exception mechanism.
  MCP group state now comes from `mcp_servers` / `mcp_servers_mode` (already emitted by `GET /api/agents/{id}`, just missing from the dashboard's `AgentDetail` type) minus `tool_blocklist`, with the blocklisted tools labelled inside the expanded group.
  MCP groups are also read-only on this tab now: `PUT /api/agents/{id}/tools` carries no `mcp_servers` field, so the old group toggle wrote MCP tool names into `capabilities_tools` where the kernel ignores them — it looked like it worked and changed nothing.
  The grant semantics are mirrored into a tested `src/lib/toolGrants.ts` (glob matching, MCP name normalization, grant-mode resolution) rather than re-derived inline (#6565) (@houko)
- Fix goal creation from the dashboard always failing: the create form seeds `parent_id` and `agent_id` as empty strings and posted them verbatim, so `POST /api/goals` read `Some("")` for the parent, looked for a goal literally named `""`, and answered `404 Parent goal '' not found` for every goal a user tried to create.
  The blank `agent_id` was persisted too, and later made `POST /api/goals/{id}/start` reject the goal with "Assign an agent to this goal before starting a run" on a goal the user never assigned.
  Both layers are fixed: the API now normalizes a blank (or whitespace-only) `parent_id` / `agent_id` to "absent" on create and treats it as the same clear-this-link signal as `null` on update, and the dashboard stops sending the empty fields at all.
  A non-blank `agent_id` that is not a valid UUID is now rejected up front with `400 Invalid agent_id` on both create and update, instead of being silently stored and only surfacing later as the same misleading "Assign an agent to this goal before starting a run" message on a goal that was in fact assigned.
  The six built-in goal templates rendering on every visit is by design — they are a static catalog, and `handleApplyTemplate` already posts `{title, description, status}` with no `parent_id`, so applying one was never affected (#6562) (@houko)
- Fix interactive menu button callbacks (`/models` provider and model pickers, "back to providers") being silently dropped on Telegram, which made every press a no-op with no reply and no error.
  The bridge's menu interceptor read the pressed menu's id from `metadata["message_id"]` with `as_str()` only, so it resolved `None` and hit an early `return` on both sidecars: the Rust adapter wrote the id as a JSON number (`serde_json::Value::as_str` never matches a number), and the Python adapter never puts the id in callback metadata at all — it sets the canonical top-level `message_id`, which the daemon stores as `ChannelMessage.platform_message_id`.
  The resolver now accepts a string or a number from metadata and falls back to `platform_message_id`, so both adapters work; the Rust adapter also emits the id as a string in both slots, matching the wire contract already used by `message_event`, by `SidecarMessageParams.message_id: Option<String>`, and by the Python adapter.
  Populating the top-level slot additionally stops the daemon from synthesising a random UUID for a callback's `platform_message_id`, which could never address a real Telegram message.
  The unresolvable case is now a `warn!` carrying both candidate values instead of a `debug!`, so a future regression of this class is visible rather than silent (#6564) (@houko)
- Honour `agent.toml: [model] context_window` on the paths that actually run a turn, so the field the runtime's own warning tells operators to set is no longer inert.
  All three execution paths — the non-streaming `execute_llm_agent`, the streaming turn, and the ephemeral `/btw` turn — resolved the window from the model catalog only, so an agent on a model the catalog does not know (a custom or proxied model reached through a `provider_urls` override) stayed pinned to the 8192-token `UNKNOWN_MODEL_CONTEXT_WINDOW` fallback no matter what the manifest said, and trimmed history aggressively on every turn.
  Only the read-only context report honoured the override, and its comment claimed to mirror "the same precedence chain the agent loop uses" — so the dashboard showed the operator's value while every real turn ignored it.
  The chain (manifest override, then catalog, then persisted session value) now lives in one `manifest_helpers::resolve_context_window` used by all four call sites; the compaction gate uses the same helper but deliberately skips the session level, whose stale value would otherwise rank above its 200K default window.
  A session `/model` switch clears `model.context_window`, since that number annotates the manifest's model and would otherwise size the budget from the wrong model; the streaming path also resolves the window *after* the session override is applied, where it previously read the pre-override manifest (#6568) (@houko)
- Stop `agent kill` / agent removal (the `purge_identity=true` / canonical-UUID purge path) from deleting the agent's rows in `audit_entries`, which broke the append-only WORM Merkle chain that `security verify` walks and made routine test-agent purges report a chain break indistinguishable from tampering.
  `execute_structured_agent_deletes` — the single canonical agent-scoped delete cascade — listed `audit_entries` alongside genuinely per-agent tables (memories, kv_store, sessions, …), so purging an agent opened a `seq` gap whose downstream `prev_hash` no longer resolved.
  The audit trail is now excluded from the cascade: its rows survive with their now-orphaned `agent_id` (an audit trail is supposed to record what happened, including to agents later removed), while the rest of the per-agent purge is unchanged; `approval_audit` (a separate flat table with its own time-based retention, not the Merkle chain) stays in the cascade. Remediating a pre-existing gap left by the old behavior — a non-destructive re-anchor / attestation path instead of the fully-destructive `security audit-reset` — is tracked separately as a follow-up feature (#6553) (@houko)
- Make `POST /api/skills/reload` report an honest result in Stable mode instead of always returning `{"status":"reloaded"}` on a no-op.
  When the skill registry is frozen (Stable mode) the kernel `reload_skills` previously bailed with only a `warn!`, so new skill directories added after boot were silently invisible and the HTTP handler still reported success — indistinguishable from a real reload without daemon-log access.
  The kernel now returns a `SkillReloadOutcome`: while frozen it refreshes the on-disk content of already-loaded skills via the freeze-safe `SkillRegistry::reload_skill` and reports any brand-new skill directories it is deliberately not loading (via a new `SkillRegistry::unloaded_on_disk_dirs`), and the handler surfaces `{"status":"partial"|"refreshed","frozen":true,"refreshed":[...],"skipped_new":[...],"detail":...}` so an operator can see that a restart is needed to pick up new skills; the intentional Stable-mode freeze boundary is unchanged (#6540) (@houko)
- Stop losing tool results in the 10–16 KB band, where a result was too small to spill to a recoverable artifact yet large enough to be lossily truncated by the sanitizer.
  Artifact spill (`spill_fresh_result`, default `spill_threshold_bytes = 16_384`) preserves an oversized result's full bytes as a recoverable stub, but the sanitizer's `strip_tool_result_details` independently hard-cut at a hardcoded `10_000` with a non-recoverable `...[truncated from N chars]` marker, so a result in `[10_000, 16_384)` never spilled and was truncated irrecoverably — defeating spill's documented intent to run "before `sanitize_tool_result_content` truncates".
  The sanitizer's size cut is now tied to `DEFAULT_SPILL_THRESHOLD_BYTES` (a new exported constant that `default_spill_threshold_bytes` also returns) so the two can't drift apart, making the cut a genuine last-resort fallback that only fires when spill did not run (spill disabled or the artifact write failed) rather than a dead band below the spill threshold; the size-independent base64-blob and injection-marker stripping still runs unconditionally (#6545) (@houko)
- Stop the skill/tool-result security scanner from flagging ordinary words as hardcoded secrets: the credential-prefix patterns (`sk-`, `ghp_`, `gho_`, `github_pat_`, `xoxb-`, `xoxp-`, `akia`) matched as free lowercased substrings via Aho-Corasick, so any word merely containing those characters (`task-`, `risk-`, `disk-`, `ask-`, `desk-`, `mask-`) tripped a false "Possible hardcoded secret" warning that `injection_guard::scan_tool_result` then escalated into a `[SECURITY WARNING …]` banner prepended to clean tool output.
  These seven prefixes are now token-anchored (the preceding character must be start-of-text or a non-token separator) and require a plausible key body after the prefix (a run of at least 12 key-ish `[a-z0-9_-]` characters containing a digit), mirroring the `starts_with` prefix check in `tool_runner::taint`; real leaked keys (`sk-` + high-entropy body, `ghp_…`, `AKIA…`, `xoxb-…`) still fire, while the non-prefix members of the group (`api_key`, `-----begin rsa`, …) keep matching as plain substrings (#6541) (@houko)
- Apply the `[approval] auto_approve = true` shorthand when the policy is installed into the `ApprovalManager`, fixing a silent no-op where the flag cleared nothing and every tool stayed gated.
  `ApprovalPolicy::apply_shorthands` (which clears `require_approval` when `auto_approve` is set) was only ever called from a unit test — the daemon-boot path (`ApprovalManager::new_with_db`) and the hot-reload path (`update_policy`) both installed `config.approval` verbatim, so an operator who set `auto_approve = true` to disable gating got no effect and the field's own doc comment ("clears the require list at boot") was false.
  The shorthand is now applied at all three policy-install entry points (`new`, `new_with_db`, `update_policy`), so `auto_approve` takes effect at boot and on `POST /api/config/reload`; the separate `trusted_senders` / `[[users]]` RBAC layering is documented above and unchanged (#6492) (@houko)
- Return a deterministic `409 Conflict` when a client resolves an approval that was already resolved, instead of a non-deterministic `400`-or-`404` that depended on whether the in-memory `recent` ring still held the entry.
  `ApprovalManager::resolve` reported "Already {decision} by {who}" (mapped to `400`) only while the resolution sat in the 100-slot buffer, and degraded to "not found" (`404`) once the buffer evicted it or the daemon restarted, so the same double-resolve returned different statuses depending on load and uptime.
  `resolve` now falls back to the durable `approval_audit` log to recognize an already-terminal request, and the api boundary maps the "Already …" verdict to a new typed `LibreFangError::Conflict` (`409`, code `conflict`) a client can act on, while a genuinely-unknown id still returns `404` and a missing second factor still returns `400` (#6492) (@houko)
- Stop `KnowledgeStore::delete_by_agent` from silently orphaning another agent's knowledge when a shared entity's first-writer agent is deleted (#6521).
  Entities are keyed on `(id, peer_id)` — not `agent_id`, which is only first-writer provenance — so a deterministic-id entity (a well-known org/person name) first written by agent A can be referenced by agent B's live relations; deleting every `agent_id = A` entity on A's deletion removed that shared row, and B's relations quietly stopped resolving (the JOIN just stopped matching — no error, data vanished from future reads).
  `delete_by_agent` now deletes A's relations wholesale (they are strictly per-agent) but only removes A's entities that NO surviving relation still references by id or name, keeping shared, still-referenced entities in place.
  Surfaced in review of #6519 (#6521) (@houko)
- Scope entities/relations created by the MCP `knowledge_add_entity` / `knowledge_add_relation` tools to the calling agent instead of the empty-string sentinel, so tool-created knowledge is no longer split-brained from the rest of the graph; `MemorySubstrate::add_entity` / `add_relation` hardcoded `agent_id = ""`, while the proactive-extraction write path and the agent-scoped read (`GET /api/memory/agents/{id}/relations` → `query_graph_scoped(.., Some(agent_id), ..)`) both key on the real agent uuid — so anything an agent stored via its own `knowledge_add_*` tools could never be returned by the agent-scoped relations endpoint or the dashboard relations view, and `delete_by_agent` never removed it (it orphaned on agent deletion); the caller agent id (already available at the tool dispatcher as `caller_agent_id`) is now threaded through the `KnowledgeGraph` handle trait and the `Memory` trait into the store, and an absent caller id keeps the historical shared/unscoped (`""`) write; existing rows already written under `""` stay shared/unscoped (they cannot be retroactively attributed to an agent), while new writes are agent-scoped; entity rows remain shared across agents by their composite `(id, peer_id)` key (which does not include `agent_id`), so an entity's `agent_id` is first-writer provenance and the relation carries the load-bearing scoping — the pre-existing `delete_by_agent` / `ON CONFLICT` interaction for a shared entity whose first-writer agent is later deleted is unchanged here and tracked as a follow-up (#6519) (@houko)
- Serialize an explicit `session_id_override` that targets the agent's canonical session on the SAME lock as the no-override persistent path, closing a lost-update race on session history.
  The message-dispatch lock was selected purely on whether an override was supplied — `Some(sid)` → `session_msg_locks[sid]`, `None` → `agent_msg_locks[agent_id]` — so a REST `POST /message` passing `session_id = <canonical entry.session_id>` and a concurrent no-override persistent dispatch to the same agent (plain `send_message` / a workflow step / a trigger fire on an agent with no home channel) took two different mutexes, ran concurrently, and both loaded → appended → blind-`save_session`'d the one canonical session, losing one write.
  An override equal to the canonical session now collapses to the per-agent lock namespace (with matching re-entrancy / held-lock tracking) at both the non-streaming (`send_message_full_inner`) and streaming (`send_message_streaming_with_sender_and_opts`) sites; the raw override still flows to `resolve_dispatch_session_id`, so the resolved session id is unchanged.
  A no-override channel dispatch keeps the per-agent lock (narrow fix — the channel-derived override variant, which the cron-prune path already hand-guards, is unchanged) (#6518) (@houko)
- Stop the streaming LLM paths from silently swallowing a provider error delivered mid-stream over an already-HTTP-200 SSE body, which turned a recoverable overload / rate-limit into a truncated-or-empty "successful" turn with no retry or failover.
  The Anthropic driver now handles an `event: error` frame (e.g. `overloaded_error` under load) by returning the typed `LlmError::Overloaded` / `RateLimited` / `Api` it maps from `error.type` instead of letting the frame fall into the catch-all match arm and ending the stream `Ok` with partial content.
  The OpenAI-compatible driver (which OpenRouter and Groq route through) now surfaces a terminal `data: {"error": …}` frame as an `LlmError::Api` classified via a new `openai_stream_error_code` helper — so a rate-limit still retries the same provider rather than skipping it — but only when the frame carries a real error signal (a non-null `message` / `code` / `type`, a non-empty string, or a bare number/array), so a benign `"error": null` or an all-null error object riding alongside valid `choices` on a normal content chunk no longer aborts the stream and discards that content; the classifier also accepts the numeric HTTP-status `code` OpenRouter sends (`429`) in addition to the symbolic strings, and the surfaced `LlmError::Api.status` is derived from the typed code (429 / 503 / 402 / …) instead of a blanket 502.
  The same non-null check also filters empty strings, not just null, on the nested `error.message` / `error.code` / `error.type` fields — mirroring the existing bare `"error": ""` exclusion — so a provider that pads a normal content chunk with an all-empty-string (rather than all-null) placeholder object no longer has that chunk's content discarded (#6512) (@houko)
- Apply a content-emitted guard at BOTH streaming layers that can re-run a provider, so surfacing a mid-stream error (above) cannot corrupt the caller's output by concatenating a second full response onto content already delivered.
  `FallbackDriver::stream` (the failover layer) now wraps the inner stream in the same content-emitted intercept relay `FallbackChain::stream` already uses and propagates the error instead of failing over once any observable content (text / tool / thinking deltas) has reached the caller — and no longer health-penalizes or exhaustion-marks a provider that served content before erroring, matching `FallbackChain`.
  `stream_with_retry` (the per-provider retry layer in librefang-runtime), which previously re-streamed a full second response onto the same caller channel after a mid-stream `Overloaded` / `RateLimited` / transient error, now tracks whether content has reached the caller and surfaces the error instead of retrying once it has (retry is still applied when the error precedes any content) (#6512) (@houko)
- Classify a `[browser]` config change as restart-required instead of reporting a false hot-reload success: `POST /api/config/reload` queued a `HotAction::ReloadBrowserConfig` whose apply arm only logged "new sessions will use updated config", but the live `BrowserManager` captures `BrowserConfig` by value at boot with no rebuild path, so every new session kept the boot-time `max_sessions` / `cdp_endpoint` / `headless` until a full daemon restart while the reload reported the change applied.
  `build_reload_plan` now marks browser changes `restart_required` (honest reload report), the misleading hot action and its now-dead `ReloadBrowserConfig` variant are removed, and the ops reference table (`docs/operations/config-reload.md`) is updated `H` → `R` (#6516) (@houko)
- Honor `CompletionRequest.response_format` (JSON / JSON-Schema structured output) in the Gemini and Vertex AI drivers, which silently dropped it — the `GenerationConfig` had no `responseMimeType` / `responseSchema` field and neither `build_request` (used by both Vertex call sites) nor Gemini's own inline `complete()` / `stream()` request builders consulted the field, so a caller that set `response_format = Json` / `JsonSchema` and routed to Gemini/Vertex got free-form prose and a downstream JSON parse failure with no signal, while the OpenAI, Anthropic, and Ollama drivers already honored it.
  `GenerationConfig` gains `response_mime_type` + `response_schema`, a `gemini_response_format` mapper sets `responseMimeType = "application/json"` for `Json` / `JsonSchema` and passes the schema through verbatim for `JsonSchema`, and `complete()` / `stream()` now route through `build_request` so all four call sites (2 Gemini inline + 2 Vertex) share one mapping (Vertex inherits the fix).
  Gemini's `responseSchema` is a restricted OpenAPI-subset, so an invalid schema is rejected by the API like any other bad request (#6515) (@houko)
- Stop a superseded/aborted streaming turn and a timed-out trigger fire from permanently leaking the scheduler's per-agent token reservation, which could lock an agent out of its own quota until the hourly window rolled over (a self-inflicted quota DoS).
  `AgentScheduler::check_quota_and_reserve` pre-charges `total_tokens` and returned a bare `u64`, so the pre-charge was rolled back only by an explicit `settle_reservation` / `release_reservation` call; when the owning future was dropped mid-flight — a follow-up message superseding the in-flight streaming turn (`abort()`, #3739), a `stop`/`kill`, or a trigger fire dropped by its `tokio::time::timeout` — neither call ran and the estimate (the agent's `model.max_tokens`) leaked, while the USD sibling (`MeteringReservation`) was already released on drop.
  Reservations now go through a new `AgentScheduler::reserve_tokens` that returns a `#[must_use]` `TokenReservation` RAII guard whose `Drop` releases the pre-charge unless it was explicitly settled/released, bringing the token side to parity with the USD side; a no-quota reservation carries `estimated_tokens == 0` so its drop is a no-op, preserving the zero-reservation contract.
  Only affects agents with a non-zero effective token quota configured (#6513) (@houko)
- Make the CHANGELOG `(@user)` attribution check recognize attribution anywhere in a bullet's block, not only on the `- ` marker line, so it stays compatible with the repo's own one-sentence-per-line prose rule: a long multi-sentence bullet wraps across lines and carries its trailing `(@houko)` on the final continuation line, which `check-changelog-attribution.py` (and the pre-commit hook + `CHANGELOG Attribution` CI gate that call it) previously flagged as missing attribution, so wrapping a bullet to satisfy the prose rule broke the attribution gate. The three scan modes (diff / `--all-unreleased` / `--staged`) now check the whole bullet block (marker line plus indented continuation lines up to the next blank line, bullet, or heading); a no-attribution bullet is still caught (@houko)
- Route the post-approval agent reply through the account-qualified outbound path so multi-account installs deliver to the correct bot/chat: `wake_agent_after_approval` previously looked the channel adapter up by the bare channel key (`channel_adapters.get(channel)`), ignoring `deferred.account_id`, but adapters are registered under both the bare key and the account-qualified key (`"<channel>:<account_id>"`), so a resumed reply for a non-first account was delivered to the wrong account's adapter or missed entirely; it now reuses `ChannelSender::send_channel_message`, which builds the same account-qualified lookup key the canonical outbound path uses (#6492) (@houko)
- Pin `crates/librefang-api/src/login_page.html` to LF in `.gitattributes`: the file is embedded verbatim via `include_str!` and its inline `<script>` is authorised by an exact SHA-256 baked into the CSP (`middleware.rs`), so a Windows checkout with `core.autocrlf=true` rewrote it to CRLF, shifted the computed hash, and failed `dashboard_login_page_script_is_allowed_by_csp_hash` on the Windows shard only — turning `main` red on every merge since #6486 touched the page (#6481) (@houko)
- Honour glob patterns in the per-agent `tool_allowlist` / `tool_blocklist`, matching the Step 1 builtin filter in the same function: Step 4 of `available_tools` used exact string equality, but MCP tool names are runtime-generated and namespaced (`mcp__<server>__<tool>`), so they can never be enumerated as static literals — a non-empty `tool_allowlist` therefore retained only exactly-named native tools and silently dropped every `mcp__*` tool, and a `mcp__*` glob entry itself matched nothing because `*` was treated literally; both retains now use `glob_matches` (already used by the builtin filter above), so `["file_read", "mcp__notion__*"]` works and a plain exact name still matches unchanged via `glob_matches`' `pattern == value` fast path (#6495) (@houko)
- Fix `librefang approvals approve` failing with a 415 that printed a false `✔ approved`, and hide resolved entries in `approvals list` (#6492): the approve request was a bodyless POST, so it carried no `Content-Type` and axum's `Json<ApproveRequestBody>` extractor rejected it with 415 before the handler ran — it now sends an empty JSON object so the header is present. The CLI also gated success only on a `body["error"]` field, so a 415 (whose body deserializes to `{}`) printed success; success is now gated on the real HTTP status via a new `daemon_json_checked` helper, with the failure reason falling back to the status when the body carries no error. Separately, `approvals list` rendered every entry the API returns — which includes recently-resolved (`approved` / `rejected` / `expired`) requests alongside pending ones — with no status column, so a terminal request looked actionable; it now shows a Status column. Server-side resolution and the re-resolve 400 were already correct (the latter via #6441). (@houko)
- Treat `auto_dream` (and other system-internal) fork tool calls as system-internal so RBAC no longer gates them: dream turns run through `run_forked_agent_streaming` with a `None` sender context and no synthetic channel, so once `[[users]]` was configured their `memory_store` / `memory_recall` / `memory_list` calls fell through `resolve_user_tool_decision` to `guest_gate` — whose allowlist lacks the `memory_*` tools — and hit `NeedsApproval` on every dream cycle; `LoopOptions` now carries a `system_call` flag that the fork path sets and the runtime dispatch forwards to `ApprovalGate::resolve_user_tool_decision` (mirroring the existing cron / autonomous channel escape hatch), so internal forks bypass the per-user gate while an unattributed user call with no sender still fails closed through the guest gate (#6463) (@houko)
- Allow the standalone `/dashboard` login page's inline submit handler under the global Content Security Policy: the CSP's `script-src 'self'` (no `unsafe-inline`, no hash) blocked the page's inline script that posts credentials to `/api/auth/dashboard-login`, while the bundled React SPA at `/` uses only external scripts and was unaffected — making the failure look language-dependent; `script-src` now allows only the exact SHA-256 hash of that static script, `unsafe-inline` stays forbidden for scripts, and a new response-level regression test hashes the served script and asserts the CSP source allows it (#6480) (@houko)
- Stop a partial `[channel_overrides]` table from silently gating group messages to mention-only: `dm_policy` / `group_policy` were bare enums that materialized to their `#[default]` (`GroupPolicy::MentionOnly`) whenever the struct was constructed, so writing one unrelated field (e.g. `threading = true` for Slack) flipped an unset group policy from the absent-`None` "process everything" behaviour to "mention-only" and dropped all non-mention group traffic on every channel the agent served — the reporter lost all Matrix inbound this way (#6444 fixed only the Matrix symptom). Both fields are now `Option<_>`, so an unset policy stays `None` (no gating, matching the historical whole-struct-absent path) and is distinct from an explicitly written `Some(MentionOnly)`; the two bridge gating paths (text + media) treat `None` identically to the old no-overrides case, and an unset policy that still carries `group_trigger_patterns` resolves to mention-only so the patterns keep gating. The reply-intent precheck gate on both paths was carried along too: it previously fired only for the literal `Some(GroupPolicy::All)`, so the common unset-policy "process all" config silently lost its precheck filter and replied unconditionally, while the semantically-identical explicit `group_policy = "all"` still got it; both gates now route through a shared `group_reply_precheck_applies` helper that resolves `None` (no trigger patterns) the same as explicit `all`, so the two paths cannot drift again (#6468) (@houko)
- Stop the `Security` CI job from failing red on an audit-infrastructure outage: npm retired the legacy `pnpm audit` endpoints (`/-/npm/v1/security/audits[/quick]`, now HTTP 410 "endpoint is being retired"), and `xtask deps --web` classified every resulting non-zero `pnpm audit` exit across `web` / `dashboard` / `docs` as a dependency issue — so all three tripped and `main` went red on every push that touched Rust or CI, unrelated to the change under test; `run_pnpm_audit` now captures the audit output and, on the retired-endpoint signature, warns and skips (a `Skipped` outcome not counted against the build) while a genuine advisory still fails as before (#6466) (@houko)
- Stop `sandbox_command` from silently dropping secret-shaped names the operator explicitly listed in `exec_policy.allowed_env_vars` (#6439 regression): the credential-word heuristic (`KEY`/`SECRET`/`TOKEN`/`PASSWORD`/…) meant the allowlist could never carry a credential, breaking every credentialed CLI driven through `shell_exec` / `process_start` (e.g. a keyring-unlock password env var); the two passthrough sources are now filtered by trust level — the operator's own allowlist is refused only the daemon's reserved secrets (`LIBREFANG_VAULT_KEY`, provider API keys, via the new `is_reserved_env_var`), while hand-assembled untrusted lists keep the full heuristic at both assembly and spawn time, and refused names are now surfaced in the tool output instead of only a daemon-log WARN (#6458) (@houko)
- Resolve the model context window provider-aware so an OpenRouter agent stops showing the 8192 unknown-model denominator in the chat context bar (and no longer mis-sizes the live `ContextBudget` / compaction math): live catalog entries are keyed `openrouter/{raw_id}` but the dashboard model-picker (`set_agent_model`) persists the manifest model without the prefix, and the kernel's eight `find_model(&manifest.model.model)` lookup sites were provider-blind, so a bare id (`tencent/hy3:free`) missed the prefixed catalog entry and fell back to `UNKNOWN_MODEL_CONTEXT_WINDOW`; a new `ModelCatalog::find_model_for_manifest(provider, model)` re-qualifies the bare id against the `{provider}/` prefix (the issue's suggested `find_model_for_provider` alone would still have missed), and every lookup site — context report, live budget, compaction, tool-support detection, and the #6398 thinking-backfill gate — now routes through it (#6423) (@houko)
- Fix Matrix sidecar inbound being silently dropped under the default `GroupPolicy::MentionOnly`: the adapter hardcoded `is_group=True` for every room and never set `metadata["was_mentioned"]` (the only mention signal the group gate reads), so both 1:1 DMs and explicit @-mentions of the bot were dropped with `OB-06 group_gating_skip` and the bot appeared dead; the adapter now flags a room non-group when it is listed in `m.direct` or its `/sync` summary reports two joined members, and sets `was_mentioned` when the bot's own user id is in the MSC3952 `m.mentions` list, restoring parity with the Discord / Feishu adapters (#6444) (@houko)
- Derive the fallback `agent.toml` path from the agent's UUID instead of the literal `"agent"` when the DB row has no `source_toml_path`: `safe_path_component` strips every non-ASCII character, so an agent whose display name is entirely Cyrillic / CJK / accented-Latin sanitized to the empty string and every such agent collapsed onto the shared path `workspaces/agents/agent/agent.toml` — TOML edits were silently ignored and two such agents overwrote each other; the three fallback sites (boot re-sync, `persist_manifest_to_disk`, `reload_agent_from_disk`) now pass the UUID, matching the spawn-time `resolve_workspace_dir` derivation so distinct agents never collide and auto-discovery finds the real directory (#6442) (@houko)
- Cap the `FallbackChain` non-streaming rate-limit backoff: a provider's `Retry-After` (or the shared rate guard's persisted ~1h RPH lockout) was slept verbatim before failover, stalling the turn for minutes-to-an-hour and defeating the chain's purpose; a backoff over a 10 s cap now fails over to the next provider immediately, matching the streaming path and the sibling `FallbackDriver` (#6446) (@houko)
- Stream attachment-URL fetches with a running size cap instead of buffering the whole body first: `resolve_url_attachments` read `resp.bytes()` and only checked the 20 MB limit afterward, so a server streaming an unbounded body (bounded only by the 30 s timeout) could allocate multiple GB and OOM the daemon; it now pre-rejects on Content-Length and aborts mid-stream once the cap is exceeded, bounding peak memory (#6446) (@houko)
- Mark a workflow run `Failed` when a Conditional / Loop / DAG step references a deleted agent: those paths returned `Err` via a bare `?` without touching run state (only the Sequential arm set `Failed`), so the run hung in `Running` until the next stale-recovery sweep at daemon boot; all agent-resolution failure paths now share a `mark_run_failed` helper, and the DAG concurrent layer skips resolving the agent of a step whose dependency already failed (#6446) (@houko)
- Bound ClawHub/Skillhub skill-zip extraction with the same decompression-bomb guards as the marketplace bundle path (entry count, per-entry uncompressed size, compression ratio, total uncompressed size) via a shared `write_zip_entry_capped` helper: the install path streamed every entry with an unbounded `std::io::copy`, so a malicious skill zip could exhaust disk / inodes (#6446) (@houko)
- Fix the per-IP WebSocket connection tracker's guard-drop TOCTOU: the `Drop` impl decided to `remove` the entry from a stale read, so a concurrent `try_acquire_ws_slot` could re-increment it between the read-guard release and the `remove`, deleting the entry backing a live connection and letting a single IP exceed `max_ws_per_ip`; it now uses `remove_if` with a re-checked counter under the write lock (covers both the agent and terminal WS paths) (#6446) (@houko)
- Compare canonicalized `serde_json::Value`s (not raw JSON strings) in the config-reload `field_changed` diff: `HashMap`-bearing fields (users, channel_role_mapping, broadcast routes, audit retention, per-skill env, …) serialized in per-instance iteration order, so a content-identical reload spuriously reported a change — emitting a needless `ReloadAuth` and a `restart_required` signal on every reload for any multi-entry deployment; `to_value` normalizes map key order so equal content compares equal (#6446) (@houko)
- Evaluate the session-reset policy on persistent channel sessions even when the agent manifest sets `session_mode = "new"`: the reset skip was keyed on the requested mode, but the channel branch resolves to a persistent `for_sender_scope` session regardless of `session_mode`, so such a session was silently excluded from `config.session.reset`; the skip is now keyed on whether the session id was freshly minted this call (a genuinely ephemeral `SessionId::new()`) (#6446) (@houko)
- Record cached prompt tokens in the ChatGPT / Codex Responses-API driver: the streaming usage parse read only `input_tokens` / `output_tokens` and left `cache_read_input_tokens` at zero, so the metering layer billed the cached prefix at the full input rate (the 10% cache-read discount was lost); it now reads `input_tokens_details.cached_tokens`, mirroring the chat-completions driver (#6446) (@houko)
- Surface a mid-stream error on the OpenAI-compatible streaming endpoint as an in-band error frame instead of a truncated `finish_reason:"stop"` + `[DONE]`: the forwarder discarded the agent-loop `JoinHandle` (the only carrier of a mid-stream failure) and always emitted a clean finish, so clients received partial output framed as a successful completion; also keep the streamed `tool_call` index monotonic across agent-loop iterations and thread image / conversation-history input through to vision models (a streaming request carrying an image now returns an explicit 400 rather than silently dropping it) (#6441) (@houko)
- Key the channel message debouncer and typing-flush buckets on the per-user sender id rather than the group chat id, so within the debounce window one group member's message (and any injected instructions) is no longer coalesced into another member's identity, RBAC resolution, and billing (#6441) (@houko)
- Write memories through to an attached HTTP vector backend on both insert and re-embed so embedding recall is not silently empty against it, and send the OAuth bearer token / configured auth headers to SSE-transport MCP servers so OAuth for the SSE transport is no longer inert (#6441) (@houko)
- Tolerate `redacted_thinking` / server-tool content blocks in the Anthropic non-streaming `complete()` path (the streaming path already did), clear the stale past `expires_at` on an MCP token refresh that omits `expires_in` (which otherwise forced a network refresh every request and burned rotating refresh tokens), and re-rank SQLite semantic recall over a similarity-neutral candidate window so relevant older memories are not dropped (#6441) (@houko)
- Make `GoalRunner::start` atomic so concurrent starts for the same goal cannot leave a second orphaned, unstoppable loop, and make the cron `SummarizeTrim` generation-CAS exclude the concurrent message send so an appended cron turn is not silently lost (#6441) (@houko)
- Map `approve_request`'s typed kernel errors to the correct HTTP status — 404 for a missing / expired id, 400 for a missing second factor or an already-resolved request, and 500 only for a genuine internal failure — instead of collapsing every resolve failure to 400 (#6441) (@houko)
- Gate the `cli_pypi` release job to stable and LTS tags only, in both `release.yml` (the tag-push pipeline) and `release-cli.yml` (the manual `workflow_dispatch` CLI re-run path): every beta/rc pre-release tag was also uploading ~250 MB of per-platform CLI binary wheels to the `librefang` PyPI project, and 46 accumulated pre-releases consumed 10.35 GB of its 10 GB total-size quota, so the `v2026.7.10` stable publish started failing with `400 Project size too large`; pre-release CLI binaries remain attached to the GitHub Release, only the PyPI upload is now skipped for them (#6433) (@houko)
- Stop the desktop release job's asset cleanup from deleting the CLI binary tarballs: its "Delete existing assets for this target" step matched `*<rust_target>*`, the same target triple the `cli_*` jobs embed in `librefang-<target>.tar.gz` (+ `.sha256`), so a desktop job re-running after its CLI counterpart had already uploaded — exactly what the macOS desktop jobs do when they re-run for notarization — deleted the freshly-uploaded `librefang-{x86_64,aarch64}-apple-darwin.tar.gz` as collateral, which then made `cli_pypi` fail with a 404 when it downloaded them to build the wheels; the cleanup now skips CLI-owned `librefang-*` / `SHA256SUMS*` assets (desktop bundles use Tauri's own `LibreFang_<ver>_<x64|aarch64|…>` naming, which never contains the triple, so the guard costs the desktop nothing) (#6433) (@houko)
- Clear 13 more Rust 1.97 stable clippy `useless_borrows_in_formatting` errors (`redundant reference in format! argument`) across the CLI TUI screens (`comms.rs`, `hands.rs`, `settings.rs`, `skills.rs`, `templates.rs`): stable's tightened lint turned the `Quality / Build + clippy` gate red on every PR regardless of which one triggered the `main` run, the same lint class #6421 cleared elsewhere in the workspace; `format!` captures its arguments by reference either way, so dropping the borrow is a pure lint fix with no behavior change, and the genuine `widgets::truncate(&x, n)` function-argument borrows are untouched (#6427) (@houko)
- Stop the channel media gate from spending a billed reply-precheck LLM call on captionless media in a `group_policy=all` group: #6141 ran `classify_reply_intent` on an empty string (`extracted_user_text(...).unwrap_or_default()`) for a caption-less image / voice / video, so the precheck now runs only when there is text to classify (captioned media keeps parity with the text path) and captionless media proceeds without the call, which also reconciles #6141's PR notes that claimed the precheck was not replicated onto the media path when the merged code did replicate it (#6426) (@houko)
- Seed test-booted kernels from a pinned in-repo registry snapshot (`crates/librefang-runtime/tests/fixtures/registry/`, librefang-registry@89d0e4c8) instead of the network: #6410's `LIBREFANG_REGISTRY_OFFLINE=1` export made `sync_registry` a no-op on fresh test homes, turning main red with ~111 registry-content unit tests (model catalog, hand routing, hands registry, MCP catalog/installer, hand activation, metering) plus 3 `librefang-api` integration tests; a new `registry_sync::seed_registry_fixture_for_tests` copies the fixture into the test home and fans it out through the real sync path, `resolve_home_dir_for_tests` and the 27 API-test harnesses use it, `MockKernelBuilder` gains an opt-in `.with_registry_fixture()`, and the env-var test locks are promoted to a crate-wide `test_env::ENV_LOCK` so `tts::test_synthesize_no_provider` no longer races `model_catalog`'s `GOOGLE_API_KEY` setter under threaded `cargo test` (#6421) (@houko)
- Clear the Rust 1.97.0 stable clippy errors (`question_mark`, `useless_borrows_in_formatting`, `for_kv_map`) that turned `cargo clippy -- -D warnings` into a hard failure on the Quality lane of every PR touching a Rust crate — `main` itself stayed green only because its recent commits were docs-only, so CI's per-crate change-detection skipped the Rust lane and never exercised the lint — across `librefang-skills` (`config_injection.rs`), `librefang-runtime` (`a2a.rs`), `librefang-kernel` (`tools_and_skills.rs`), and four `librefang-api` sites (`channel_bridge.rs`, `routes/agents/sessions.rs`, `routes/workflows/workflow.rs`, `tests/totp_flow_test.rs`); folded into this PR because its own Quality lane cannot go green until the lints are fixed, and the registry fixture this PR adds is what keeps the test lanes green alongside the clippy fix (#6421) (@houko)
- Uppercase the character after each separator when deriving the version-pinned Homebrew tap formula's class name, matching Homebrew's `Formulary.class_s`, so a pre-release tag yields `class LibrefangAT2026710Beta1` — the exact casing the `golden_gate` audit requires; #6416's `tr -cd '[:alnum:]'` removed the hyphen but left the channel word lowercase (`...beta1`), which `golden_gate` still rejects, so the next `-beta` / `-rc` release would have generated a versioned formula that fails `brew readall` / install (verified by running the real generation step for a stable and a beta tag → `brew readall` reports zero errors) (#6418) (@houko)
- Correlate dashboard chat turns with the WebSocket terminal frames that own them: the client now sends a `message_id` on every chat message and the daemon echoes it on `response` / `silent_complete` / `error`, so a previous turn's late terminal frame — delayed past the next user send by post-turn proactive-memory extraction — lands on its own bubble instead of overwriting the newer message's bubble and dropping the newer answer (#6419) (@houko)
- Generate valid Ruby class names for the version-pinned Homebrew tap formulae (`librefang@<ver>`): the release pipeline stripped only `.` from the version (`tr -d '.'`), leaving the `-` in pre-release versions so `librefang@2026.6.29-beta.14` produced `class LibrefangAT2026629-beta14 < Formula` — a syntax error that made every beta/rc pin fail `brew readall` / install; it now derives the class name with Homebrew's own `Formulary.class_s` rules (`LibrefangAT2026629Beta14`), and the 40 already-published broken pins are corrected in `librefang/homebrew-tap` (#6416) (@houko)
- Emit a single `conflicts_with` cask stanza in the Homebrew tap cask generator (`release.yml` and `release-desktop.yml`): casks allow only one `conflicts_with`, but the generator wrote one stanza per other channel, so `librefang-beta` / `librefang-rc` casks were rejected as invalid; the other channels are now collected into a single array literal (`conflicts_with cask: ["librefang", "librefang-rc"]`), with the two already-published casks fixed in `librefang/homebrew-tap` (#6416) (@houko)
- Export `LIBREFANG_REGISTRY_OFFLINE=1` in the four CI test lanes (unit, Ubuntu shards, Windows, macOS) so test-booted kernels stop fetching the content registry: each fresh temp home triggered a real git clone per boot, and whether the fetch succeeded changed test outcomes — a registry-pre-installed `assistant` `agent.toml` made restore treat the disk template as authoritative and clobber explicit DB model config, deterministically failing PR #6384's restart test in CI while passing locally under `just test` (#6410) (@houko)
- Request extended thinking through the OpenAI-compatible driver: a configured `CompletionRequest.thinking` budget now emits the OpenAI-style `reasoning_effort` parameter (`low` ≤ 4096 tokens, `medium` ≤ 16384, `high` above; budgets under 1024 stay off, matching the anthropic driver's gate), closing the asymmetry where the stream path parsed `reasoning_content` deltas but the request side never asked for them — emitted only under the default echo policy (Kimi's `EmptyString` disable and the DeepSeek `Strip` / `Echo` families are excluded), an explicit `extra_body.reasoning_effort` override takes precedence, and the global `[thinking]` backfill now skips models the catalog marks `supports_thinking = false` so a mixed fleet does not start sending the parameter to known non-reasoning models (#6407) (@houko)
- Correct the channel docs' claim that all five sidecars read `*_DM_POLICY` / `*_GROUP_POLICY` env vars: only the WhatsApp sidecar reads `WHATSAPP_DM_POLICY` (Cloud API webhook mode only; `WHATSAPP_GROUP_POLICY` is read but inert), the Telegram / Discord / Slack / Teams sidecars read neither, and the working migration target is `agent.toml [channel_overrides]` `dm_policy` / `group_policy` enforced by the channel bridge (with `allowed_only` user gating backed by `[[users]]` channel bindings); also replace invalid `dm_policy = "always"` / `group_policy = "trigger_only"` example values that fail manifest deserialization (#6405) (@houko)
- Exempt fresh `read_artifact` results from every re-spill gate — the post-tool chokepoint, the per-result tool budget (Layer 2), and, except as a last resort, the per-turn aggregate budget (Layer 3) — so paging in a spilled tool result larger than `spill_threshold_bytes` returns real bytes (still subject to the pre-existing ~10 KB sanitize truncation cap) instead of minting a fresh artifact stub that again points at `read_artifact`; previously any read in the 16–64 KiB range could never make progress, and a `spill_threshold_bytes` below the sanitize cap recreated the loop at Layer 2 (#6406) (@houko)
- Add a `LIBREFANG_REGISTRY_OFFLINE` switch that skips the registry network refresh (git clone / tarball fallback) while keeping local pre-install copies, and export it from `just test`: every test-booted kernel previously attempted a real registry fetch from its fresh temp home, and the resulting git fork storm exhausted pid limits in constrained containers (`Resource temporarily unavailable` failures in `just test`) (#6408) (@houko)
- Clear the `cargo-deny` / security-audit advisory RUSTSEC-2026-0204 failing CI on every branch by bumping the transitive `crossbeam-epoch` to 0.9.20 (invalid pointer dereference in the `fmt::Pointer` / `fmt::Display` impls for null `Atomic` / `Shared` pointers; pulled in via `rayon-core` → `crossbeam-deque`, lockfile-only change) (#6400) (@houko)
- Prefer OpenRouter's live `/models` catalog over the checked-in fallback snapshot when selecting or validating a model, refresh stale data when the user opens the model picker, narrow default model synchronization to target only the delisted model during intra-provider free-model migrations, align catalog freshness checks across route handlers, make restart tests hermetic, and treat missing pricing as unknown instead of free (#6384) (@pavver)
- Preserve `provider = "default"` and `model = "default"` as an explicit agent setting that follows future global model changes, expose a dashboard action to restore that mode, and treat every concrete agent model as an explicit pin (#6384) (@pavver)
- Surface the model each CLI passthrough provider (`codex-cli`, `claude-code`, `gemini-cli`, `qwen-code`) is configured to run, read live from the tool's own config, so a custom model — DeepSeek via `~/.codex/config.toml`, a Kimi/Moonshot id via Claude Code's `ANTHROPIC_MODEL` / `~/.claude/settings.json`, a Gemini preview via `GEMINI_MODEL` / `~/.gemini/settings.json`, or an OpenAI-compatible id via `~/.qwen/settings.json` — is recognised on the Providers page and in the agent model picker instead of only the catalog's default models (#6365) (@houko)
- Stop CLI providers (`codex-cli`, `gemini-cli`, `claude-code`, `qwen-code`) from forcing a placeholder `--model <provider-id>` onto their CLI for a bare provider id, so each CLI defers to its own configured default model (#6365) (@houko)
- Clear `cargo-deny` advisory failures on `main` by bumping `anyhow` to 1.0.103 (RUSTSEC-2026-0190) and ignoring the unmaintained `ttf-parser` advisory (RUSTSEC-2026-0192 — transitive via `pdf-extract` → `lopdf`, no safe upgrade available) (#6366) (@houko)
- Clear the `quick-xml` advisories RUSTSEC-2026-0194 / RUSTSEC-2026-0195 by bumping `plist` to 1.10.0 (pulls the patched `quick-xml` 0.41.0) and `tauri-winrt-notification` to 0.7.3 (drops its `quick-xml` dependency), removing both vulnerable versions from the lockfile (#6387) (@houko)
- Raise the Nix Build job timeout to 120 minutes so the now-routine cold builds — the Rust CI lanes churn the repo's 10 GB Actions cache quota daily, evicting the `/nix/store` cache between runs — complete instead of being cancelled at 60 minutes, unbreaking the workflow that had been red on `main` since June 11 (#6389) (@houko)
- Route every dashboard copy button through the clipboard helper, so copying works on a daemon reached over plain HTTP at a LAN or VPN address.
  `navigator.clipboard` is only defined in a secure context, so on `http://<lan-ip>:4545` the property is `undefined` and a bare `navigator.clipboard.writeText(...)` throws before the promise exists — the button produced no clipboard content, no error, and no visual feedback.
  `lib/clipboard.ts` has fixed this since it was written, falling back to `document.execCommand('copy')` through a detached textarea, but only the chat page imported it; the config page, audit detail, agents page, users page, skill install command, and TOML viewer had all reintroduced the raw API.
  The helper reports failure by resolving to `false` rather than throwing, so each call site now branches on the result instead of on a `catch` that could never fire — the users page in particular gated the close button of a one-time-visible rotated API key on a copy it never verified, and two call sites showed a "Copied" toast unconditionally.
  An eslint `no-restricted-properties` rule now rejects `navigator.clipboard` everywhere in the dashboard except the helper itself, because the helper's own comment shows this was already fixed once and regressed (#6668) (@houko)
- Expose `peer_id`, `session_mode`, and `delivery` on `/api/schedules`, which `/api/cron/jobs` already reported for the identical job.
  The two routes are deliberate alternate views over one `CronJob` store — the cron view serializes the struct whole, the schedules view renders a flattened presentation — and the flattened one had fallen behind on all three fields that decide how a fire behaves: which peer's memory it resolves against, whether every fire shares one session or gets an isolated one, and where its output goes.
  The reporter could confirm only `peer_id` because `CronJob::session_mode` carries `skip_serializing_if = "Option::is_none"`, so an unset value is absent from the cron view rather than null and looked the same as a field that does not exist.
  The schedules view therefore emits both as explicit nulls when unset, matching how it already renders `tz`, `last_run`, and `next_run`: a read surface with a stable key set lets a client tell "not configured" apart from "the server is too old to report it".
  `POST /api/schedules` now sets all three as well, where it previously hardcoded `peer_id` to null, forced `delivery` to the fire-and-forget variant, and parsed `session_mode` through an `.ok()` that turned a misspelling into "use the agent's default" behind a `201`; a malformed value on any of them is now a 400 that names the field.
  `PUT /api/schedules/{id}` patches `delivery`, which the kernel has always supported and this route simply never forwarded.
  It cannot patch `peer_id` or `session_mode`, because `CronScheduler::update_job` has no branch for either and its omitted-or-null-means-untouched convention cannot express clearing an optional field, so a request that tries to change one is refused with a 400 explaining that a recreate is needed rather than answered with a 200 for a patch that never applied — echoing the stored value back stays a no-op so the natural read-modify-write round trip still works (#6668) (@houko)
- Report `cost_usd`, `total_tokens`, `duration_ms`, and the derived `label` on `GET /api/sessions/{id}`, which the session list has always computed and the detail endpoint did not.
  The list derives cost and tokens from a `usage_events` join, the duration from the first-to-last stamped message, and a label snippet from the first user message when the column is empty, while the detail handler hand-built a fixed object carrying none of that — so the same session answered differently depending on which route you asked, and an unnamed session that reads as "hello…" in the list read as `null` in the detail.
  The same root cause as #6596, so both views now share one helper per value rather than a second copy of the derivation, since a copy is what let them diverge in the first place.
  The shapes match the list exactly: cost and tokens are numeric zeros for an unmetered session rather than null, the duration is null below two stamped messages, and an explicit label still wins over the snippet.
  A failed usage aggregate now returns 500 rather than a zero, matching the session load beside it — reporting no spend for a session that spent money is the same silent-wrong-value failure this issue is about (#6668) (@houko)
- Render `approval.trusted_senders` as a read-only card on the Approvals page, so the approval-bypass roster is auditable from the dashboard and not just from the API.
  A sender on that list skips the approval prompt for every tool the risk classifier does not rank high, and it reached no operator-facing surface at all — #6637 exposed it on `GET /api/config` along with the rest of the non-writable `ApprovalPolicy` fields, but nothing rendered it, so auditing who holds the waiver still meant shell access to read `config.toml`.
  It stays out of the config write allowlist for the reason it needed exposing in the first place: adding yourself to an approval-bypass list over HTTP is precisely the escalation the approval gate exists to prevent, so holding an API key must not be enough to do it.
  The card sits above the pending queue because it explains the requests that never arrive, and an empty list is presented as the reassuring state it is — every sender goes through the gate (#6668) (@houko)
- Reject unknown keys inside `[mcp_servers.transport]` instead of silently discarding them, closing a gap between an MCP server entry and the transport table nested one level inside it.
  `McpServerConfigEntry` has carried `deny_unknown_fields` since #5130, because the `detect_unknown_nested_fields` walker bails on array-of-table paths and serde is the only layer that can see a typo in a `[[mcp_servers]]` element at all — but `McpTransportEntry` and the two structs under its `http_compat` variant did not, so the guard stopped exactly one level above where operators hand-write the most config.
  A reporter's `[mcp_servers.transport.env]` table was dropped whole at load: `env` is real, but it belongs to the parent entry as a `Vec<String>` of variable *names*, not to the transport as a key/value table, and nothing said so.
  The subprocess ran with neither variable set and `GET /api/mcp/servers/{name}` reported `"env": []`, while the server's own script fell back to hardcoded defaults that happened to match — so the misconfiguration was invisible until a credential rotation, at which point the rotated secret would have been written to an inert table and the stale default kept working.
  `HttpCompatHeaderConfig` and `HttpCompatToolConfig` had the same gap and are guarded too; both sit under `[[mcp_servers.transport.headers]]` / `[[mcp_servers.transport.tools]]`, arrays of tables nested inside an array of tables, and every field on them except `name` and `path` has a `serde(default)`, so a misspelled `responce_mode` left the tool wired to the default JSON response mode rather than the operator's choice.
  On an internally-tagged enum serde applies the attribute per variant against the buffered content, which is not obvious from the attribute alone and is not true of adjacently- or untagged-tagged containers, so the behaviour is pinned by tests over the reporter's exact TOML rather than assumed from the fact that it compiles.
  Read this before upgrading, because the failure is loud and total rather than local: a stray key under `[mcp_servers.transport]` now makes `librefang start` **exit non-zero without starting the daemon** — the deserialize error propagates out of `load_config`, and `cmd_start` short-circuits on it precisely so the diagnostic naming the offending key reaches stderr instead of being swallowed by a tolerant default.
  It is not "that one server is skipped"; nothing boots until the key is removed.
  Because the rejection happens inside serde it applies whether or not `strict_config` is set, the same way the parent entry has already behaved for a misspelled scalar such as `timout_secs`.
  The hard stop is boot-only: `POST /api/config/reload` maps the same error to a `400` and leaves the running config in place, so a live daemon surfaces the typo without dropping its MCP connections — worth knowing if you edit `config.toml` on a running host, since reload tells you about the mistake at no cost while a restart on the same file will not come back up.
  Two read-side keys the API synthesised into the `transport` object, a derived `source` discriminator on each `http_compat` header and a `tools_count` duplicating the length of the array beside it, are gone: neither is a field of the guarded types, so with the guard in place they turned any `GET` → `PUT` of an `http_compat` server into a `400`, and serde short-circuits at the first unknown key so both had to go.
  The same read route also omitted `input_schema`, which has a `serde(default)`, so that round trip had been quietly overwriting a hand-authored JSON Schema with `{"type":"object"}` — it is now emitted, and a static `http_compat` header `value`, which stays redacted because it is a credential, is merged back from the stored entry on write exactly as an inline `env` value already was.
  Writing an `http_compat` server through the default `config.toml`-backed store also corrupted its headers, which the tests for the round trip above found: that path goes `serde_json` → `json_to_toml_value` → TOML, TOML has no null, and the converter maps an absent `Option` to an *empty string* rather than dropping the key, so an env-sourced header came back from disk as `value = ""` instead of unset.
  The runtime checks `value` before `value_env`, so it then sent an empty header and never resolved the variable — a silent credential failure with nothing logged.
  `value` and `value_env` now carry `skip_serializing_if`, the same fix already documented as load-bearing on the entry's own `template_id` and `oauth` fields for the identical reason.
  A header that carries *both* a static `value` and a `value_env` lost the static one on every read-modify-write, which the merge above initially reproduced rather than fixed: the read route redacts `value` and emits `value_env`, and the merge treated that returned `value_env` as "nothing to restore here".
  Since the runtime resolves `value` first, the header silently stopped sending the operator's static credential and started resolving the variable instead — a `200` with nothing logged and a different request on the wire.
  The merge now keys the decision on `value` alone, so the presence of a `value_env` beside it no longer suppresses the restore (#6666) (@houko)
- Request `/json/new` with `PUT` when discovering a target over HTTP, so `cdp_endpoint` works against Chrome 111 and newer.
  Chrome moved the endpoint to `PUT` as CSRF hardening; librefang still sent `GET` and got back a `405` page whose HTML then failed to parse, so the operator saw `Invalid JSON from /json/new: expected value at line 1 column 1` — a parser error that says nothing about the verb that caused it.
  The request now tries `PUT` first and falls back to `GET` on `405`, which keeps older builds and proxies that only route `GET` working, and the response status is checked before the body is parsed so a non-2xx reply is reported as itself rather than as malformed JSON.
  The two verb-negotiation tests pin the call counts rather than only the returned target, since a GET-first implementation reaches the same result and would otherwise pass both (#6619) (@nevgenov)
- Substitute `MAX_CONTENT_CHARS` into the page-extraction script instead of leaving the cap hard-coded in the JavaScript, so the constant that documents the limit is the one that enforces it.
  `MAX_CONTENT_CHARS` was declared, marked `#[allow(dead_code)]`, and read by nothing; the script truncated at a literal `50000` written twice inside the JS string.
  Editing the constant to change how much page text reaches the model therefore did nothing at all, and the `#[allow]` suppressed the one warning that would have said so.
  The script is now a template with a `__MAX_CONTENT_CHARS__` placeholder substituted once through a `LazyLock`, and a test asserts both that the placeholder is gone from the built script and that it is still present in the template — the second half is what keeps the two from silently drifting apart again (#6623) (@nevgenov)
- Carry the curated `## [Unreleased]` section into the dated release section, which nothing had ever read.
  `cargo xtask changelog` built the whole `## [VERSION]` section from PR metadata — `git log` for the numbers, `gh pr view` for the titles — and inserted it *below* `## [Unreleased]` without touching it, and the `awk` extractors in `release.yml` and `release-notify.yml` slice only that dated section for the release notes, the announcement article, and the social post.
  So the section was write-only: 160 hand-written bullets, the part that explains why a change was made rather than restating its title, and the part a pre-commit hook and two CI jobs enforce `(@user)` attribution on, reached nothing at all.
  The release cut now lifts that body out — subsections and their order verbatim, because a human chose them — composes the dated section as stats, breaking changes, highlights, the curated prose, then the generated entries, and leaves the `## [Unreleased]` heading behind and empty, since in-flight PRs append under it and the `changelog.d/` fold errors outright without it.
  Generated entries fill only the gaps: a PR whose number appears in the trailing `(#N)` group of a curated bullet gets no title line, so every PR is described exactly once.
  That group is read from the bullet's last non-empty line only, which is what keeps a mid-bullet cross-reference (`the latter via #6441`) from being mistaken for the PR the bullet documents and suppressing a real entry, and it accepts the `(#6594, #6595)` form one bullet already uses for two PRs.
  A bullet carrying no reference at all cannot be matched to anything, so that PR keeps its generated line and the bullet is named in a warning — a duplicated entry is cosmetic, a silently dropped PR is not.
  The fallback is per bullet, not global: one unreferenced bullet out of 160 leaves the other 159 suppressing their own entries, where discarding the whole set would have turned three such bullets into no deduplication at all.
  Draining is destructive, and a lossy CHANGELOG surfaces only after the tag exists, so the composed section is checked against the section as it stood on disk before anything is written and a missing bullet aborts the release naming it.
  The check compares whole bullets rather than `- ` marker lines, because the sentence-per-line prose rule makes multi-line bullets the norm — 67 of the 160 — and a marker-line comparison would call a bullet preserved while every sentence after its first had been dropped.
  It parses the section to the next *dated* heading while the drain stops at the next `## [` of any kind, so the two disagree exactly where a drain would truncate — on a bullet continuation line that starts in column 0 with a release heading — rather than restating what the drain happened to take.
  A second release run for one version aborts as well, and needs its own check: regenerating an existing `## [VERSION]` section rebuilds it from PR titles, and by then the first run has already emptied `## [Unreleased]`, so the prose is in no section of the file and the primary guard is comparing against nothing.
  It fires on attribution found anywhere in the bullet, not just the marker line, and names the two ways out — move the prose back under `## [Unreleased]`, or delete the dated section and cut it again.
  Nothing prunes `## [Unreleased]`, so the cut now also reports how many curated bullets reference a PR in the release's own commit range and how many reference only older ones, and warns on the latter: the section accumulates across releases, and carrying an already-shipped bullet into a dated section announces it as new alongside a `Full diff` link that contradicts it (#6628) (@houko)
- Stop the daemon from aborting on macOS when a child process exits while the WASM sandbox is live.
  Wasmtime defaults to Mach exception ports on macOS, which parks a handler thread in a blocking `mach_msg` receive flagged `MACH_RCV_INTERRUPT`; the kernel then returns `MACH_RCV_INTERRUPTED` instead of restarting the call when a signal lands on that thread, and wasmtime treats anything but "port closed" as fatal — it prints one `mach_msg failed with ...` line and calls `abort()`.
  The daemon delivers such signals constantly (SIGCHLD from MCP stdio servers, sidecar channels, exec-tool subprocesses, and provider CLI probes), and a process-directed signal can be routed to any thread, so the whole process could die with no panic, no unwind, and no log beyond that line.
  The sandbox now selects POSIX signal handling, which needs no thread parked in an interruptible syscall and is inherited across `fork()`; Mach ports only buy coexistence with Mach-port-based crash reporters, and nothing in the dependency tree uses one.
  It surfaced as a hard SIGABRT partway through `cargo test -p librefang-kernel --lib`, and CI could not have caught it because nextest runs each test in its own process — a `cargo test` guard step on the macOS lane now covers that gap (#6635) (#6638) (@houko)
- Keep an edit to a registry-shipped hand by writing it to a new operator override directory, `<home>/hands/`, instead of into the registry checkout that the next sync erases.
  `update_manifest_persisted` wrote the edit back to `registry/hands/<id>/HAND.toml` because that was the only way to win the load-order race, but that path is inside a git checkout the registry sync fast-forwards with `git reset --hard origin/main` — so the supported way to customise a built-in hand erased itself on the next daemon start, which is what the #6636 reporter hit when even marking the file read-only did not help.
  The override directory sits outside both `registry/` (upstream's copy, and only upstream's) and `workspaces/` (which already means "installed here" rather than "customised here"), and nothing writes into it but an explicit edit, so an override exists only if someone made one.
  `scan_hands_dir` reads it first and the kernel router follows the same precedence, because routing that resolved against upstream's definition while the registry served the edited one would route a renamed hand by rules nobody could see in the UI.
  An id now counts as claimed only once a manifest has actually been read, so a half-written override directory shadows nothing — before, it would have dropped the registry's hand from the scan entirely rather than overriding it.
  Minting an override seeds the shadowed copy's `SKILL.md` and `SKILL-{role}.md` alongside the manifest, since the override replaces the whole directory the scan reads and a manifest-only override would silently strip the skill content that becomes the agents' system prompts; a skill file the operator has since edited is never overwritten.
  Uninstall reaches an override-only hand as well, which is otherwise un-uninstallable once upstream drops an id someone had customised — the hand reports as built-in while the override resurrects it on every reload — and it still refuses, without touching the override, while a registry copy exists (#6669) (@houko)
- Report every `[approval]` field on `GET /api/config`, not the seven of fourteen the response builder happened to enumerate.
  The approval section declares no explicit field list, so the dashboard renders a control for whatever the derived schema says `ApprovalPolicy` has, and the missing seven rendered blank and read back as their JSON zero value.
  `cache_approvals_per_session` is the one an operator noticed: it defaults to `true`, so the box showed unchecked even against a `config.toml` that said `true`, leaving no way to confirm whether per-session approval caching was on.
  `trusted_senders`, `channel_rules`, `timeout_fallback`, `routing`, `totp_tools`, and `audit_retention_days` had the same gap.
  All seven stay non-writable — approval policy is deliberately not adjustable over HTTP, so an Owner-role caller with a leaked API key cannot relax it — which is exactly why the existing `writable ⊆ readable` guard was blind to them, and the new check uses the serialized struct as its oracle so a field added later fails a test instead of quietly joining the gap (#6637) (@houko)
- Make a hand's `[[settings]]` values reach the agent that is already running, not just the one that boots next.
  Settings only reach an LLM as the rendered `## User Configuration` tail on each role's system prompt, and that tail is materialized when the hand is activated.
  Saving from the dashboard wrote the instance config and persisted `hand_state.json` but never touched the live agents, so the change took effect on the next daemon restart — boot replays every persisted hand through the activation path, which does re-render — while the running agent kept answering from the HAND.toml defaults with no error to explain the discrepancy.
  Reported against the Trading Hand, where the prompt branches on `trading_mode` and `approval_mode`: selecting Live Trading and disabling approval silently got neither.
  The save path now persists the config and rewrites each role's prompt under the same per-instance lock the runtime-override path uses, rolling the config back if either step fails so the persisted file and the live registry cannot disagree.
  Re-rendering a live prompt needed its own helper: the three rendered tails are appended in a fixed order and each one strips from its own marker, so re-applying them in sequence to a prompt that already carries all three drops the reference and team blocks on the way past.
  The env-var passthrough allowlist is re-resolved on the same path, since changing a select changes which `provider_env` the subprocess sandbox should admit, and it now narrows as well as widens.
  Two `librefang hand` subcommands that were broken in a way that made the CLI no help in diagnosing any of this are fixed too: `hand set` posted a wrapped `{"config": …}` body at a handler expecting a flat map and never touched the named setting, and `hand settings` read a response field that does not exist and reported "no configurable settings" for every hand.
  A stored value that is not a JSON string — `false` for a toggle, `100` for a numeric field, both valid from any API client — also reverted the setting to its schema default, and scalars are now coerced (#6637) (@houko)
- Stop an explicitly configured EveryAPI gateway from being repointed at whatever account the EveryAPI CLI happens to be logged into — including mid-way through the dashboard's own "Connect EveryAPI gateway" flow, which registers the provider entry before storing its relay key.
  Credential provenance was inferred from `is_custom` and `auth_status`, neither of which carries that meaning — the catalog loader leaves `is_custom` false for every provider when the registry cache is unreadable, and an explicit configuration whose key env var is simply unset looks exactly like an unresolvable CLI login — so a provider file installed before its key was set had its `base_url` rewritten from the CLI account until the next daemon restart.
  Provenance is now recorded explicitly and cleared by every explicit source, so an entry with no reachable credential stays inert instead of falling through to the CLI credential process (#6647) (@houko)
- Show the official EveryAPI square logo in the dashboard sidebar instead of a plain "E" letter placeholder.
  The asset is served from the dashboard's public path and a partner logo asset contract test pins the URL EveryAPI links resolve against (#6648) (@houko)
- Stop reporting an untrackable `agent_send` as a delegation, which handed the model the answer in a field named `task_id` and told it to wait for a callback that would never fire.
  `send_to_agent_async_tracked` has two legitimate outcomes and they meant opposite things: a task id on the tracked path, the callee's whole response body when no parseable caller session made tracking possible.
  Both arrived through one undiscriminated `Result<String, _>`, so the tool that consumes it labelled the response body `task_id`, set `status: "delegated"`, and instructed the model to end its turn and wait — for a reply it was already holding, and which no registered task would ever deliver again.
  The blocking fallback itself is correct and stays; with nowhere to route a completion event, an inline answer beats an orphaned delegation.
  It now returns an `AsyncSendOutcome` the caller must branch on, so a tracked delegation still renders its task id and an inline one returns the reply exactly as the blocking path does.
  The path is reached in production by the MCP HTTP bridge and the REST tool bridge, both of which pass no session by construction, and its log moves from `debug!` to `warn!` with the caller fields attached — an operator whose agents lose async delegation should not have to raise the log level to find out (#6662) (@houko)
- Call `set_self_handle` in the CLI's in-process ACP and MCP backends, which boot a kernel of their own and previously aborted the process the moment they needed a kernel handle.
  `LibreFangKernel::boot` returns a bare kernel and leaves the `self_handle` slot empty — filling it is the caller's job, discharged by seven other production sites but not by these two.
  ACP failed at startup: `KernelAdapter::new` reads `kernel_handle()` as its first action, and that accessor is an `.expect`, so `librefang acp` aborted before serving a request.
  MCP failed later and less visibly, on the first `librefang_agent_*` tool call, because `send_message` resolves the same handle to plumb kernel tools into the agent turn.
  A comment on `KernelAdapter::new` asserting that `boot` wires the handle up before returning is corrected — that claim is what made both omissions look deliberate, and `boot.rs` contains no such call.
  Three tests in `crates/librefang-kernel/tests/self_handle_bootstrap_test.rs` pin the kernel-side contract from both directions, including the idempotence that makes a defensive call safe for any future surface that boots its own kernel, and a source-scanning guard in `librefang-cli` asserts the call sites themselves so deleting either line fails rather than silently restoring the abort.
  Found while triaging the unrelated concern reported in #6651, which does not reproduce on `main` (#6686) (@houko)
- Salt the delegation dedupe hash with the caller and conversation, so two sessions delegating the same message to the same agent no longer share one async task and one reply.
  `register_async_task` dedupes delegations on `(target agent, prompt_hash)` and deliberately ignores the caller — a #5033 decision its own docstring records, along with the instruction that callers needing per-session isolation must salt `prompt_hash` themselves.
  The kernel's only production caller did not: it hashed the message text alone, making the hash a pure function of the prompt.
  Two independent agents asking the same agent the same question therefore collided on one registry entry, the second received the first's handle, and the completion event was delivered only to the first caller's session — while the second, already told "delegation started asynchronously; do not wait", waited for a reply that would never arrive.
  The hash now covers the caller agent, the caller session and the conversation key as well as the message, which is a superset of the registry's `(agent, session)` delivery key plus the field that selects a different callee session.
  The fix is caller-side by design: the kind-only match key is the documented contract and stays, pinned by `register_dedupe_is_cross_session_for_delegation_kind`, and what still dedupes is exactly the intended idempotency case of one caller re-sending the same message on the same conversation while the first is in flight (#6662) (@houko)
- Report a goals storage failure as a 500 instead of an empty page, a leaked error string, or a missing goal.
  `GET /api/goals` folded a substrate read error into the same empty array it returns when nothing has been created yet, so a corrupt blob or an unreadable SQLite file reached the operator as `200 {"items": [], "total": 0}` — indistinguishable on the wire from a daemon with no goals, and the dashboard drew its template-picker empty state over live data it had simply failed to read.
  `GET /api/goals/{id}/children` was worse in both directions at once: a 200 carrying an empty list *and* a raw `format!("{e}")`, so a client checking the status saw success while the body handed out the SQLite path and error chain.
  `POST /api/goals/{id}/start` had the same swallow on a write path — its catch-all `_ => Vec::new()` turned an unreadable store into `404 Goal not found`, sending the operator to re-create a goal that exists.
  All three now log the full error and return the scrubbed 500 envelope that `GET /api/goals/{id}` next to them already used; an absent or non-array key stays a 200 empty result, because that is the genuine not-yet-created shape (#6662) (@houko)
- Add `[registry] auto_sync` so the daemon can be told to stop overwriting the registry checkout.
  `~/.librefang/registry/` is a git clone the sync fast-forwards with `git reset --hard origin/main`, so every local modification under it is destroyed — including the ones `PUT /api/hands/{id}/manifest` writes, which land in `registry/hands/<id>/HAND.toml` whenever the hand shipped with the registry, making the supported way to customise a built-in hand self-erasing.
  Reported as "local edits are overwritten on daemon start, even with the file set read-only, and no config option disables the registry sync".
  The culprit was not boot: `boot_with_config` honours `cache_ttl_secs` (86400 by default) and normally skips, while the background catalog task passes a hard-coded TTL of `0` into `refresh_registry_checkout`, which makes `should_refresh` true for any marker older than a second and forces the reset on its first tick and every 24 h after — so gating boot alone would have changed nothing.
  Setting `auto_sync = false` freezes the checkout on both automatic paths and leaves the explicit ones (`librefang init`, `POST /api/catalog/update`) fetching as before, so freezing does not strand an operator who wants an update.
  `POST /api/hands/reload` never fetched from upstream to begin with — it only reloads hand definitions already on disk into memory — so it is unaffected either way.
  The catalog rebuild is deliberately not gated with the fetch: an operator who froze the registry still gets the models already on disk rather than an empty catalog (#6661) (@houko)
- Stop the config page offering edits the write endpoint refuses, and stop it captioning every `mode` field with the wrong description.
  `GET /api/config/schema` now reports `x-non-writable`, the resolved list of paths `POST /api/config/set` rejects, and the dashboard renders those fields visible but not editable with a line pointing at `config.toml`.
  Reported against `approval.require_approval`, which is deliberately excluded from the write allowlist so a leaked Owner-role API key cannot relax approval policy — the defect was that the UI offered the edit anyway and the save came back as a bare 403, which reads as "saved but not applied".
  The server sends the verdict rather than the allowlists because writability is decided by an exact-path list, section prefixes, a depth-2-only rule and a secret-suffix scrub; re-deriving that in the SPA would make it a third place to keep in sync, and an integration test cross-checks the emitted set against the write endpoint itself.
  Field descriptions are now looked up section-first (`desc_<section>_<field>`, falling back to the bare leaf name), so `exec_policy.mode`, `reload.mode`, `docker.mode`, `privacy.mode` and `sanitize.mode` stop showing the root-level "Kernel operating mode" text while genuinely section-neutral names like `enabled` and `timeout_secs` keep sharing one string.
  Also fixes an edit-loss path in the hand Agent tab: the prompt and tool drafts were overwritten when `useAgentDetail` resolved, so anything typed between opening the tab and the query landing vanished — a new saved value is now adopted only while the draft is untouched (#6663) (@houko)
- Re-enable the `KernelConfig` JSON Schema golden guard, which had been `#[ignore]`d long enough for real schema changes to land unreviewed.
  The attribute was added as a temporary measure over a 490-byte drift, with a comment saying to drop it once the fixture was regenerated; the fixture was never regenerated, so the guard reported "ignored" rather than failing and three subsequent schema changes reached `GET /api/config/schema` without the reviewable diff the fixture exists to force.
  Regenerating shows what slipped through: the `providers` property and its `ProvidersConfig` definition (the org-wide provider allowlist), `ExecPolicy.full_mode_skips_approval` (also reflected in `exec_policy`'s reshaped `default` value), and a reworded `ApprovalPolicy.auto_approve` description.
  Everything else in the diff is key-ordering churn from the schemars output, which is why it is 13,716 lines for three semantic changes and why it is landing on its own rather than buried inside a feature PR (#6664) (@houko)
- Fixed the `#6631` plugin route-classification guard panicking on a Windows checkout, where `include_str!` reflects `routes/plugins.rs` back with CRLF line endings and the guard's `\n}\n` terminator search matched nothing.
  Route extraction now normalises CRLF to LF before parsing and is factored into a shared helper covered by its own regression test that builds the CRLF fixture directly, so the platform-divergent case is provable without a Windows runner (#6665) (@houko)
- Make flipping `[registry] auto_sync` take effect on `POST /api/config/reload`, which #6661 documented but did not deliver.
  `registry` was classified restart-required as a whole section, and `should_store_config` swaps the reloaded config only when a plan carries a hot action or a live-read change — so a registry-only reload was reported as needing a restart and then discarded, and the 24 h catalog task kept reading the old value out of the previous snapshot and clobbering the checkout the operator had just asked it to leave alone.
  `auto_sync` is now classified on its own as a live-read field, matching how the task consumes it, while `cache_ttl_secs` / `registry_mirror` / `registry_host` stay restart-required because they are read when the checkout is set up.
  Boot's `auto_sync` gate also gets its first test, and it asserts on the fan-out copy `sync_registry` performs with no network rather than only on the checkout's contents — an offline runner leaves the checkout alone by accident, which would have made the obvious version of the test pass whether or not the gate existed (#6671) (@houko)
- Reject a commit whose *author identity* attributes it to Claude / Anthropic, not just one whose message does.
  The `commit-msg` hook read only the message, so `GIT_AUTHOR_NAME=Claude git commit -m "fix: …"` passed a spotless message check and still put `Claude <noreply@anthropic.com>` into `git log`, `git blame`, and the GitHub commit list — the same attribution the message rule exists to keep out of the history, arriving through the one field nobody reads while reviewing a diff.
  Nine such commits reached `main` across six PRs merged on the same day before this was noticed.
  Separators are squashed before matching so `Claude Code`, `Claude-Code` and `ClaudeCode` are one case rather than three, whole names are matched rather than substrings so a contributor called Claudia or Claude Dubois is unaffected, and the address test is pinned to the bot mailbox so an Anthropic employee committing under their own address is not blocked either.
  The three `scripts/tests/` corpora for the git-side hooks now run in CI, which they never did — a regression in either attribution predicate was previously catchable only by running them by hand, which is not what anyone does before pushing a hook edit.
  That wiring also exposed `pre-commit-sha-fallback.sh` passing only where `gitleaks` was absent: it built its throwaway repo without a `.gitleaks.toml`, so wherever the tool was installed the hook aborted on a config-load error and the test reported failure (#6672) (@houko)
- Warn instead of silently failing when `api_key_hash` is set with no transmittable key, which is the posture the hash-only documentation itself recommends.
  `build_mcp_bridge_cfg` sends the master key to the daemon's own `/mcp` endpoint on behalf of CLI-based drivers, and a hash cannot stand in for it — a verifier does not yield the secret it verifies.
  So a daemon configured the way the `api_key_hash` doc, the upgrade hint, and `librefang hash-api-key` all advise — hash set, plaintext removed — built the bridge with no bearer, and its own middleware answered 401 on every driver tool call, precisely because #6613 made a hash count as configured auth.
  Nothing reported it: the driver surfaced a failed tool call, and the "nothing transmittable configured" case fell through to an empty string with no log line anywhere on the path.
  The field doc and the CLI hint now name the exception and the two workarounds that keep the secret out of `config.toml` — `api_key = "vault:NAME"` or `LIBREFANG_API_KEY` — and the kernel logs a `WARN` naming the same at every driver rebuild (#6673) (@houko)
- Regenerate the `KernelConfig` schema golden fixture, which #6664 landed one merge behind the source it is generated from.
  #6664 re-enabled the guard and regenerated the fixture, but its branch was cut after #6667 added `api_key_hash` and before #6661 added `registry.auto_sync`, so the fixture it committed described a `KernelConfig` that no longer existed by the time it merged.
  The two PRs never touched the same lines, so nothing flagged the staleness — a regeneration and a new config field conflict semantically without conflicting textually, and the fixture is only correct relative to the commit it was generated at.
  `main` went red on the first push after that, which is the guard doing exactly the job #6664 restored it for: it was off long enough for four schema changes to reach `GET /api/config/schema` unreviewed, and the first thing it caught once re-enabled was real drift.
  The regenerated fixture is reviewed rather than taken on trust, by the same order-insensitive comparison #6664 used, and every one of the sixteen semantic changes traces to a merged PR: `registry.auto_sync` and its default (#6661), `api_key_hash` and the rewritten `api_key` description (#6667), and `additionalProperties: false` on the four `McpTransportEntry` variants plus both `HttpCompat` structs (#6666), which also drops the rendered `default: null` from the two `Option` fields on `HttpCompatHeaderConfig` because schemars stops emitting it for a `deny_unknown_fields` container (#6676) (@houko)
- Accept `mp4` / `mov` / `mkv` / `avi` video containers in `media_transcribe` and `speech_to_text`, extracting and transcribing the audio track server-side instead of rejecting the file on its extension alone.
  The audio track behind these containers already transcribed fine — renaming a `.mp4` to `.m4a` was enough to get a correct transcript — because the only obstacle was an extension allowlist that happened to admit `.webm` (a dual-purpose audio/video container) through a side door while turning away containers that are exclusively video.
  The video-only extensions are budgeted against the existing `MAX_VIDEO_BYTES` limit rather than the tighter `MAX_AUDIO_BYTES` audio uses, and the extraction reuses the ffmpeg piping infrastructure the `.oga` re-mux path already established, factored into a shared helper so both transcodes share one spawn / timeout / kill-on-timeout implementation.
  Unlike the `.oga` re-mux, this always re-encodes to Ogg/Opus rather than copying the source codec: a video container's audio track can be AAC, PCM, Opus, AC3, or anything else ffmpeg can decode, and one deterministic target format is what lets the same Whisper-upload path handle all of them without per-codec branching.
  `.webm` is unaffected — it already reaches the provider unchanged through the existing audio path, and does not need the extraction hop (#6679, #6683) (@houko)

### Changed

- Upgrade `agent-client-protocol` from 0.11.1 to 1.3.0 in the `librefang-acp` crate, migrating the ACP adapter to the 1.x API (supersedes the version-only dependabot bump that left the crate failing to compile).
  The 1.x SDK moved the wire-schema types under a versioned namespace, so every `agent_client_protocol::schema::X` import is now `agent_client_protocol::schema::v1::X` (with `ProtocolVersion` re-exported at the `schema` root); the connection, router, and JSON-RPC surface (`Agent`, `Client`, `ConnectionTo`, `Builder`, `ByteStreams`, `Responder`, `on_receive_*`, `util`) is unchanged at the crate root.
  The companion `agent-client-protocol-tokio` crate has no 1.x release and was pulling a second, older copy of `agent-client-protocol` (and a stale `rmcp` 1.8) into the tree, so it is dropped entirely: its sole use, `agent_client_protocol_tokio::Stdio`, is replaced by `agent_client_protocol::Stdio`, which 1.x exposes at its own crate root with the same `Stdio::new()` constructor (#6526) (@houko)
- Bump `agent-client-protocol` from 1.3.0 to 2.0.0 and update the one call site the rename broke.
  2.0.0 renames `ResponseRouter::respond_with_result` to `route_with_result` (the request-side `Responder::respond_with_result` is untouched and easy to confuse with it), so the catch-all `on_receive_dispatch` handler in `crates/librefang-acp/src/server.rs` that routes `Dispatch::Response` failed to compile against the bumped pin until updated; `cargo check -p librefang-acp --lib` and `cargo check -p librefang-api --lib` both pass clean against the new version (#6600) (@houko)
- Normalize on-disk upload naming so every producer that writes into the shared upload directory names the file `<uuid>.<ext>` instead of today's three divergent schemes (bare `<uuid>`, `image_<uuid>.png`, `<uuid>.<ext>`), keeping the file's type at rest for extension-sniffing tools and for any flow that persists or re-dispatches the bytes.
  A single deterministic `librefang_types::media::on_disk_name(file_id, content_type, filename)` helper (extension from `ext_for_content_type`, then a safe filename extension, else a bare UUID) is now the one naming authority: the API upload / media / session-image / generated-image / browser-screenshot producers all route through it, and the client-facing `file_id` stays a bare UUID so the path-traversal and #3361 owner guards still `uuid::Uuid::parse_str` it.
  `serve_upload` and `resolve_attachments` reconstruct the name through a shared resolver that also tolerates legacy bare-`<uuid>` files and probes `<uuid>.*` for generated images not in the upload registry, so existing uploads keep serving (#6530) (@houko)
- Thread the RBAC-resolved user identity (#3054) into the audit trail for human-initiated API actions that previously wrote un-attributed rows: direct `POST /api/tools/{name}/invoke` (ToolInvoke), backup create / restore, MCP-server add / update / taint-patch / delete, all user & RBAC management mutations (create / update / delete / bulk-import / rotate-key / policy, routed through the shared `persist_users` helper, plus per-user budget set / clear), and dashboard skill-evolution (`skill_evolve:*`); each handler now reads the `AuthenticatedApiUser` from request extensions and passes `user_id` + `channel = "api"` through `record_with_context`, so `/api/audit/query?user=…` (which already filtered) now returns these events attributed rather than only the auth / budget / config paths that were already threaded — the survey confirmed daemon-internal (`DreamConsolidation`, `RetentionTrim`), cron, and agent-to-agent events are inherently userless and stay `user_id = None`, and the new `docs/architecture/audit-user-attribution.md` documents the attributed vs. userless classes and how they are labeled (#6461) (@houko)
- Thread `CanvasConfig.max_html_bytes` / `allowed_tags` into the canvas tool: the `CANVAS_MAX_BYTES` task-local was never `.scope()`d and `allowed_tags` was read nowhere, so both operator knobs were dead config and the tool always used the hardcoded 512 KiB cap + built-in tag allowlist; the agent loop now scopes both task-locals from the resolved config (kernel-populated `LoopOptions.canvas_config`), and an empty `allowed_tags` still falls back to the built-in list (#6446) (@houko)
- Consume `ParallelToolsConfig.mcp_default_safety` / `mcp_readonly_allowlist` in the parallel-tool dispatcher: an unannotated `mcp__server__tool` fell through to `Unknown` → `WriteShared`, so both knobs were inert and read-only MCP calls never parallelised; `plan_batch` now consults them (allowlist → `ReadOnly`, else the `mcp__`-prefix default), with an explicit schema annotation still winning (#6446) (@houko)
- Correct the misleading `channels.file_upload_max_bytes` doc and warn at boot when it is tuned away from the default: the in-process Matrix / Telegram adapters it configured were replaced by out-of-process sidecars (#5317–#5459) and it is no longer threaded into the sidecar send path, so the cap silently did nothing; the doc now states it is unenforced pending a sidecar-crate change and the kernel logs a WARN so an operator is not misled (#6446) (@houko)
- Correct the `prompt_intelligence.hash_prompts` doc and warn at boot when set to `false`: the `content_hash` is load-bearing (it drives prompt-version dedup and is stored NOT NULL) so it is always computed, and the flag was silently ignored; setting it `false` now logs a WARN and the doc reflects that it is effectively always-on (#6446) (@houko)
- Consolidate the four duplicated invisible/format code-point lists that #6141 added — the skills prompt-injection scanner, the runtime injection guard, the prompt-builder sanitizer, and the kernel prompt-context sanitizer — into a single source of truth `librefang_types::text::INVISIBLE_FORMAT_CHARS`, so the set can no longer silently drift between crates and reopen the scanner bypass in the un-updated copy: the three char-only copies now alias the shared const directly and the skills labeled `(char, &str)` table (it needs a per-code-point label for its warning) is guarded by an equality test that fails the build on divergence (#6426) (@houko)
- Stop the release pipeline from generating a stable `librefang` formula in `librefang/homebrew-tap`: the stable CLI now ships from homebrew-core (Homebrew/homebrew-core#290413), so the tap keeping its own copy shadowed the core formula and duplicated maintenance on every release; a stable tag now cascades the newest build into the `beta` / `rc` tap channels only, the keg-only `librefang@<ver>` versioned formula and the desktop cask sync are unaffected, and the release-notes install block now installs the stable CLI directly from core (#6416) (@houko)
- Migrate the Linux system tray implementation in `librefang-desktop` from Tauri's default `tray-icon` to a pure D-Bus implementation using `ksni` 0.3.6.
  This removes `libappindicator` / `libappindicator-sys` from the Linux build graph, eliminating the runtime `dlopen` of `libayatana-appindicator3` and the corresponding CI/Docker package install.
  It does not remove the GTK3 dependency tree (`gtk`, `gdk`, `atk`, …) or resolve the advisories tracked in `deny.toml`'s `ignore` list (RUSTSEC-2024-0411..0420) — those stay transitive via `tauri-runtime-wry` on Linux regardless of the tray implementation, and RUSTSEC-2024-0429 was never in that list to begin with.
  The new implementation re-registers with the `StatusNotifierWatcher` via `ksni` on D-Bus reconnect, updates status properties every second, and supports toggling window visibility on left-click activation (#6572) (@pavver)
- `deploy/docker-entrypoint.sh` now detects whether it started as root and adapts, so the published image satisfies Kubernetes `restricted` Pod Security without a second rootless tag to maintain.
  Under Docker it still starts as root, chowns a bind-mounted `/data`, and drops to uid 1001 through `gosu` — unchanged, so existing Compose deployments keep working.
  Under a pod that pins `runAsUser: 1001` it skips both the chown and `gosu` (neither is possible unprivileged) and verifies `/data` is writable up front, failing with a message that names the `fsGroup` requirement instead of letting SQLite die later on an opaque `EACCES`.
  The image `HEALTHCHECK` now probes `/api/ready` rather than `/api/health`: its consumer is Compose's `depends_on: service_healthy` gate, which is readiness semantics, and `/api/health` can never fail that check (#6632) (#6638) (@houko)
- Refresh the checked-in OpenRouter model snapshot used as the offline fallback catalog.
  The runtime's live catalog remains authoritative whenever OpenRouter is configured, so this update only affects lookups made before the first live fetch completes (#6642) (@houko)
- Move every CI and release workflow off Node 20 onto Node 24, which is what the dashboard's own test runner now requires.
  Node 20 left active LTS support in April 2026, and the dependabot bump of jsdom 29 → 30 (#6658) surfaced the consequence: jsdom 30 pulls an undici that calls `webidl.util.markAsUncloneable`, an API that only exists from Node 21, so every one of the 91 dashboard test files failed to start its vitest worker with `TypeError: webidl.util.markAsUncloneable is not a function`.
  That is a runtime incompatibility rather than a test failure — the same suite passes on a Node 24 host — so pinning jsdom back would have deferred the upgrade rather than fixed anything.
  All eleven `node-version: 20` pins move together instead of only the one that runs the tests: `dashboard-build`, `mobile-smoke`, `release-cli`, `release-desktop` and four jobs in `release` all build the same dashboard, so leaving any of them on 20 would have turned one red check into a release-time failure discovered later.
  Six pins in the repository were already on 24, so this makes the runtime uniform rather than introducing a new one (#6675) (@houko)
- Ship debug symbols as a separate release asset, so a crash report from a released build can actually be symbolized.
  `[profile.release]` carried `strip = "symbols"` and set no `debug` key at all, so nothing about a shipped binary was recoverable — which is where #6659 stalled: its crash report shows a six-function cycle repeating inside a 52-frame window, an unbounded recursion whose culprit is one `atos` invocation away, and that invocation could not be run.
  Offsets from an existing crash report cannot be mapped onto a later rebuild, and `lto = "fat"` with `codegen-units = 1` makes a local symbolized rebuild expensive enough that the reproduction window closed first.
  The profile now also sets `debug = "line-tables-only"` and `split-debuginfo = "packed"`, which puts function names and `file:line` for every frame into a `.dSYM` bundle on macOS and a `.dwp` file on Linux, beside the executable rather than inside it.
  `strip = "symbols"` still applies to the executable, so the binary users download is unchanged; the release workflow uploads the split file as its own `librefang-<target>-debug-symbols.tar.gz` asset, which only someone symbolizing a crash needs to fetch.
  `line-tables-only` matches what `[profile.dev]` has used since #1805 and is what keeps the cost small — it omits the variable-inspection DWARF that dominates debug-info size while keeping every frame nameable.
  The macOS job fails if the bundle is missing, since its absence there means the profile change silently regressed; the Linux job warns instead, because those targets are cross-compiled and an absent split file must not take a release down over a diagnostic aid (#6677) (@houko)
- Refresh the checked-in OpenRouter model snapshot used as the offline fallback catalog.
  The runtime's live catalog remains authoritative whenever OpenRouter is configured, so this update only affects lookups made before the first live fetch completes (#6682) (@houko)

### Security

- Add LibreFang daemon lifecycle commands and mutating writes against the daemon's own SQLite database to the dangerous-command denylist, which screens every `shell_exec` under all exec modes including `full`.
  The list already blocked `python[23]? -c`, so a diagnostic one-liner scoped to the calling agent was refused while `librefang stop` — which bounces every other agent and channel adapter sharing the daemon — passed unimpeded; #6594 reports an agent taking exactly that route ten times in one morning, leaving some channel adapters with a broken outbound path afterwards.
  The lifecycle entry matches `start` / `stop` / `restart` by bare name, by path (`target/release/librefang`, `/usr/local/bin/librefang`), with the Windows `.exe` suffix, through the `gateway` alias subcommand, and behind the CLI's one global option and its separate value token (`librefang --config <path> stop`); `librefang status` and every other read-only subcommand are deliberately not matched.
  The database entry matches mutating SQL statement forms (`insert into`, `replace into`, `update <table> set`, `delete from`, `drop table|index|view|trigger`, `alter table`) against `librefang.db`, plus output redirection that would truncate the file.
  It is written against statement forms rather than bare verbs, and scoped to the daemon's own database rather than any `.db`, so that read-only inspection stays allowed: a match is a hard block in `manual` mode, and blocking `sqlite3 librefang.db "select … from usage_events …"` would have blocked the investigation that produced #6606.
  `select`, `.schema`, `.dump`, a redirect into a separate backup file, and a `select` that merely mentions `'delete'` as a value are all covered by regression tests as staying safe (#6594) (@houko)
- Attribute an authenticated API caller's own-key spend to that caller and enforce their per-user budget, closing a bypass the per-user provider-credential feature (#6460 / #6483) opened: the owner threaded into `resolve_driver_for_owner` selects and bills the user's own vault key upstream, but usage attribution and the per-user budget gate keyed only on the request-body `sender_context`, which is absent on a plain authenticated POST (`identify("api","")` → `None`). So an authenticated user with a stored key and a configured `[budget]` could POST `/api/agents/{id}/message`, `/message/stream`, or an ephemeral `/btw` with no `sender_id`, spend on their own key past their cap indefinitely, and show zero on `/api/budget/users`; worse, because attribution was taken from the unauthenticated request body, a caller could pass a `channel_type` + `sender_id` mapping to another user's binding and have their own-key spend booked against that user. The three post-call write sites (`execute_llm_agent`, `send_message_ephemeral`, and the streaming task) now record `UsageRecord.user_id` and run the per-user budget check on `owner.or(attribution_user_id)` — the authenticated owner wins when present, sender-derived attribution stays the fallback for owner-less paths (channel / cron / agent_send). The streaming path keys on the fork-nulled `effective_owner` (not the raw owner) so a sub-agent's spend is never mis-attributed to the parent turn's user, and a plain authenticated POST that spoofs a `sender_id` can no longer book spend against someone else (#6514) (@houko)
- Canonicalize the provider name before the per-user key lookup so an alias-configured agent bills the user's own key instead of silently falling back to the operator's global credential (a chargeback leak). `resolve_driver_for_owner` looked the owner's stored key up by the raw manifest provider string (`get_user_provider_key(uid, "google")`), but the Owner CRUD's `validate_provider` only accepts canonical `known_providers()` names, so a user can only store the key under the canonical `gemini` — the lookup for an agent with `provider = "google"` (an alias of `gemini`) missed, `user_scoped_key` was `None`, and the operator's global `GEMINI_API_KEY` was billed for the user's turn while per-user attribution/budget for that turn was wrong. A new `librefang_llm_drivers::drivers::canonical_provider_name` (aliases → canonical registry name, unknown names pass through) is applied to the manifest provider before the vault lookup, so the read shares the one namespace the write surface stores under. Reads and writes were already canonical for non-alias providers, so this is a no-op there; only alias-configured agents change behavior (#6517) (@houko)
- Stop the outbound taint/DLP heuristic from false-positive-blocking legitimate MCP tool arguments that contain a long unformatted numeric id (#6499). Two rules over-blocked (fail-closed): (1) a bare export-attachment filename like `note-<id>-<id>.pdf` (no directory separator) tripped `OpaqueToken` because `looks_like_path` only recognized strings with a `/` as structured — `looks_like_path` now also excludes a bare basename with a letter-initial extension (`^[\w.\-]+\.[A-Za-z][A-Za-z0-9]{0,7}$`), so a numeric-suffixed token like `abc123.9876543` is still scanned; (2) the phone regex makes every separator optional, so any contiguous digit run matched its digit-group floor and was flagged `PiiPhone` — a match is now rejected only when the full contiguous digit run it sits in exceeds 16 digits (E.164 caps a real number at 15), leaving 10-15 digit numbers to the phone rule and 13-16 digit runs to the card rule. Genuine opaque tokens, real phone numbers, and card numbers still fire (counter-tests included); the card rule's own handling of >16-digit runs is out of scope here. The runtime `McpTaintPolicy` per-path skip-rules remain, but the default heuristic no longer requires operators to allowlist their own filenames and ids. (#6499) (@houko)
- Scope the knowledge graph (entities/relations) per user on multi-user agents, closing a cross-user data leak that mirrored the memories one #6493 fixed: the `entities`/`relations` tables were keyed on `agent_id` only (no per-user column, unlike `memories.peer_id`), so every user routed through one agent could read every other user's KG facts. A new migration (v47) adds a `peer_id` column to both tables — the shared/unscoped peer is the empty-string sentinel `''`, not SQL NULL, because NULL is distinct-from-NULL in a UNIQUE/PRIMARY KEY and would let the same shared entity duplicate — and rebuilds `entities` with a composite `PRIMARY KEY (id, peer_id)` so two users' same-named entities (which normalize to the same deterministic id) coexist as distinct rows instead of one silently overwriting the other. `peer_id` threads from the tool dispatcher (using the turn's `sender_id`, exactly as the memory tools do) through the `Memory` / `KnowledgeGraph` traits into the store, where the read query filters `r.peer_id` and the entity JOIN ties a matched entity to the relation's peer so a shared id never resolves across users. An unset peer is an unscoped read returning every peer's rows (shared semantics, matching memories); existing rows migrate to `''` and single-user agents are unaffected. The dashboard/admin relations endpoints stay agent-wide, with an optional `?peer_id=` filter on the read. (#6494) (@houko)
- Stop a compacted-session summary from leaking across users on a multi-user agent: `SessionStore::canonical_context` — the reader on the prompt-assembly path — returned the agent-scoped `compacted_summary` unconditionally, ignoring the `compacted_summary_session_id` owner column that #6225 added, so when several `[[users]]` shared one agent, a summary built from user A's conversation was injected verbatim into user B's turn (up to a 4000-char excerpt of A's private messages at prompt index 0). The #6225 fix had scoped only the dashboard `/session` banner (`compacted_summary_for_session`) and left this injection reader ungated; `canonical_context` now applies the same owner check — a summary is surfaced only to the session that owns it, while a legacy ownerless row or an unscoped (`None`) agent-wide read is unchanged. Exposed by default (the leak path runs whenever `stable_prefix_mode` is false, the default). The default heuristic summary builder in `append_canonical` (used when no LLM summary is configured) had the same class of leak from the write side: it folded every compacted message — across all sessions — into one summary and then stamped it with the single session that happened to cross the threshold, so on a busy multi-user agent user A's private messages could be folded into a summary stamped `owner = B`, which the read gate would then hand to B. The fold now includes only the triggering session's own entries (plus legacy untagged ones), mirroring the read-path filter, and only carries forward an existing summary that the same session owns — so the summary's content matches its owner stamp and the gate is honest. When the compacted batch contains none of the triggering session's messages the prior summary and owner are left untouched rather than an empty summary being stamped; the message trim is unchanged (#6493) (@houko)
- Extend the #6443 cross-account `channel_send` guard to the `/mcp` bridge: the subprocess `claude-code` driver now forwards the turn's originating bot account on the bridge connection as `X-LibreFang-Current-Account-Id` (a new `CompletionRequest.sender_account_id`, populated from the same kernel metadata stamp as the in-process path) and `mcp_http` rehydrates it into the tool execution context, so an explicit `account_id` targeting another tenant's bot account is rejected on the bridge path exactly as in-process — closing the parallel surface the #6449 fix left unchanged (#6443) (@houko)
- Reject a cross-account (cross-tenant) `channel_send`: the tool's `account_id` parameter — which selects the registered bot instance a message routes through — was passed straight from the model to the kernel send path with no validation, so on a multi-tenant daemon (several bot accounts, each with its own `default_agent` serving a different customer) an agent induced via model hallucination or prompt injection could dispatch content into a different tenant's chat; the kernel now stamps the turn's originating account into the manifest and the in-process tool rejects an explicit `account_id` that differs from it on the same channel, mirroring the #6117 recipient guard's explicit-value-only scoping (the `/mcp` bridge path is a noted parallel surface, unchanged here) (#6443) (@houko)
- Scrub the daemon environment before `process_start` spawns a persistent process, mirroring `shell_exec` / `LocalBackend`: `ProcessManager::start` built the child command without `env_clear()` + the safe-var allowlist, so under the default `Allowlist` posture (where `env` is a safe_bin) an agent could `process_start {"command":"env"}` then `process_poll` and read back the daemon's full environment — including `LIBREFANG_VAULT_KEY` (offline decryption of every stored credential) and env-provided provider API keys; the child now inherits only the safe baseline plus the agent's resolved `allowed_env_vars` (#6446) (@houko)
- Enforce caller ownership on `process_poll` / `process_write` / `process_kill`: they looked a process up by the globally-sequential, guessable `proc_N` id with no owner check, so a second agent could read another agent's stdout (secret disclosure), inject into its stdin (code execution in the victim's interpreter), or kill it (DoS); the caller's agent id is now threaded down and a mismatched owner is reported as "not found" (no cross-agent existence oracle), matching the ownership model `start` / `list` already enforced (#6446) (@houko)
- Stop a persistent plugin hook subprocess from inheriting the daemon's entire environment: `PersistentProcess::spawn` did `env_clear()` then re-added every var from `std::env::vars()`, so a plugin that merely set `[hooks] persistent_subprocess = true` received `LIBREFANG_VAULT_KEY` and all provider keys while the default non-persistent path allowlists only a safe baseline; both spawn paths now share a `hook_baseline_env` helper that re-adds only PATH/HOME/runtime-passthrough/`allowed_env_vars` (filtered through `is_blocked_env_var`) + `plugin_env` (#6446) (@houko)
- Owner-gate `GET /api/config/export`: it returned the raw on-disk `config.toml` verbatim — including the inline plaintext master `api_key`, `network.shared_secret`, and provider/channel credentials that the sibling `GET /api/config` redacts — and, as a plain GET, was reachable by any authenticated role (Viewer / User / Admin), so a leaked master `api_key` (which re-presents as `Owner`) was a full privilege escalation; it now requires `Owner` via `min_role_for_privileged_get`, matching the Owner-only gating of `/api/config[/set|/reload]` (#6446) (@houko)
- Authorize channel slash-commands through the RBAC gate before dispatch: the typed-command and text/button command paths `return`ed above the chat-path `authorize_channel_user` gate, so with RBAC / an allowlist configured an unauthorized user — rejected on any normal message — could still `/approve` pending tool calls, spawn/switch agents, reset another user's session, or run control-plane commands; both command paths now run the same gate first (a no-op when RBAC is disabled) (#6446) (@houko)
- Enforce `webhook_triggers.max_payload_bytes` at the wire level on `/hooks/wake` and `/hooks/agent` (both the `/api/hooks/*` and unversioned aliases): the endpoints previously inherited only the global 8 MiB body cap, so the documented per-webhook cap was dead config and the routes were ~128x more permissive than advertised; a `RequestBodyLimitLayer` sized from the config now bounds them (#6446) (@houko)
- Gate the `process_start` tool through the same exec allowlist + dangerous-command checks as `shell_exec` before spawning: it was dispatched straight to the process manager with no gate, so a default agent — under both the `Allowlist` and the operator-hardened `Deny` exec postures — could `process_start /bin/sh -c '…'` for arbitrary command execution that `shell_exec`'s allowlist / metacharacter / taint / approval chain would have blocked (#6441) (@houko)
- Require Owner (not Admin) for MCP-server config mutations (`POST /api/mcp/servers`, `PUT` / `DELETE /api/mcp/servers/{name}`): a persisted stdio-transport server is a raw command spawned under the daemon UID — the same process-spawn privilege `install-deps` is Owner-gated to protect — so an Admin "config write" role could reach RCE; reads and the non-spawn `reconnect` / `taint` / `auth` sub-resources keep their Admin gate (#6441) (@houko)
- Role-gate the terminal and agent WebSocket GET upgrades before the blanket "GET is read-only" RBAC rule: `GET /api/terminal/ws` (and tmux window management) require Admin+ because they spawn a PTY under the daemon UID, and `GET /api/agents/{id}/ws` requires User+ because it drives full agent turns (tool execution, budget spend) — a Viewer per-user key previously obtained an interactive shell and could trigger LLM turns the REST path denied it (#6441) (@houko)
- Scope `GET /api/memory/agents/{id}/relations` to its path `agent_id` behind the proactive-namespace read guard: the handler discarded the id and ran an unscoped knowledge-graph query, returning every agent's relation triples to any authenticated caller and bypassing the namespace ACL (#6441) (@houko)
- Add a post-canonicalize `starts_with(workspaces_root)` containment check to named-workspace relative `path` declarations (mirroring the `mount` branch) and guard `create_dir_all` against escaping symlinks, so a symlink inside the workspaces tree can no longer resolve a `[workspaces]` path to an arbitrary host directory that is then trusted as a sandbox root (#6441) (@houko)
- Scan the skill `description` and tool name / description through the prompt-injection scanner at the load boundary and at create time, closing the gap where only `prompt_context` was scanned even though `description` is inlined into the `<available_skills>` prompt block (#6441) (@houko)
- Make per-approval TOTP replay prevention atomic (hold the claim lock across check → verify → record) so a single single-use code cannot authorize more than one concurrent approval, and widen the replay-record window so it covers the code acceptance / skew window (#6441) (@houko)
- Write migrated OpenClaw channel secrets `0600` from creation instead of via a post-hoc chmod, closing the world-readable umask window (#6441) (@houko)
- Bound the marketplace skill download and zip decompression (compressed size, uncompressed size, ratio, entry count) before the audit / extract, and bound the desktop-app installer download while no longer stripping the macOS Gatekeeper quarantine (#6441) (@houko)
- Apply the reserved-system-channel defense (`resolve_scope_channel`) on the non-streaming and `execute_llm_agent` session resolvers so an external caller cannot poison the internal cron / autonomous / webui sessions, warn on sidecar instance names that collapse to the same per-instance secret prefix, and collapse unmatched-route requests to a single Prometheus `path` label to remove an unauthenticated unbounded-cardinality memory-exhaustion DoS (#6441) (@houko)
- Close the WebSocket and terminal auth bypass that a hash-only master credential would otherwise have opened.
  Both upgrade paths derived "is auth configured?" as `!valid_api_tokens(..).is_empty()`, which was a sound proxy only while `api_key` was the single way to configure a master key: a daemon whose key exists solely as `api_key_hash` lists no plaintext token, so that test reported it as unauthenticated.
  The terminal path then drove `decide_auth` past its reject branch into `LocalBypass` — unauthenticated shell access on a daemon the operator believes is bearer-gated — and the WebSocket path skipped its whole auth block, re-opening the openfang #1034 B2 branch for the new config shape, while a caller presenting the *correct* key was rejected because there was nothing to compare against.
  Both sites now share one `master_auth_required` derivation and fall back to verifying the presented token against `api_key_hash`, so the next auth surface cannot re-derive it wrongly.
  That shared derivation reads the live auth handles the HTTP middleware already reads, rather than re-resolving the credential from a config snapshot per connection, which also stops a `vault:NAME` master key from costing an OS keyring read plus a vault-file decrypt on every WebSocket and terminal upgrade — twice each, on paths reachable before any credential has been presented (#6613) (@houko)
- Bind `POST /api/auth/refresh` to the session the caller can prove it owns.
  The endpoint used to fall back to scanning the process-global token store for any entry matching a `provider` hint — or, with neither field supplied, for literally any entry that had a refresh token — and it is reachable by any Admin, so a caller could refresh a different local user's upstream session and be handed their access and rotated refresh tokens with every scope that user had granted.
  Both fallbacks are removed: a request now presents either its own `refresh_token` — which `/api/auth/callback` returns to the client, deliberately, so this is the ordinary path — or, for a client that kept only the other half, the `access_token` that callback issued it, which the server matches in constant time against the stored entry it belongs to.
  Neither is a caller-supplied assertion like `sub` or `provider`, and no ownership can be inferred from the store itself, which is keyed by upstream OIDC subject with no record of which local user owns an entry.
  A blank string in either field counts as absent rather than selecting a branch, and the store lookup rejects an empty access token outright so that an identity provider returning one cannot leave an entry that matches a caller who presented nothing — a request that proves nothing gets a 400 instead of someone else's credentials (#6629) (#6639) (#6644) (@houko)
- Stop `GET /api/mcp/servers` and its `{name}` detail sibling from serializing MCP environment values.
  The `env` list is documented as variable names to pass through, but the supported representation also accepts an inline `KEY=VALUE`, so an operator could put a live credential there and any reader got it back verbatim.
  The report describes a Viewer-role caller; it is worse than that — the list route sits in `PUBLIC_ROUTES_DASHBOARD_READS`, so with `require_auth_for_reads` left at its default an unauthenticated caller could read those values too.
  Both routes now return variable names only, and the write path merges a submitted bare name against what is stored: redacting the read side alone would have been a data-loss bug worse than the disclosure, because the dashboard hydrates its edit form from the list response and submits every field back on save, which would have wiped the very credentials the caller was never shown (#6630) (#6639) (@houko)
- Require Owner authorization for every plugin route that can put plugin-controlled code on an execution path.
  Admin is "config write" by design, but it could previously install a Git-backed plugin and then invoke dependency installation, so npm / pip / Bundler / Composer ran attacker-supplied package lifecycle scripts under the daemon UID — crossing the Admin/Owner boundary into arbitrary code execution with access to daemon secrets.
  Owner is now required for `install`, `install-with-deps`, `install-deps`, `test-hook`, `upgrade`, `enable`, `reload`, `prewarm`, and `sign`; `sign` is included because load-time integrity verification rejects a hook whose hash no longer matches, so re-signing is what makes a tampered script loadable again.
  `install-with-deps` and the batch `prewarm` route are gated alongside their per-name/singular siblings: both call the identical underlying function (`install_plugin_with_deps`, `reload_plugin`) that the singular `install` and `reload` gates already cover, just through a top-level path with no `{name}` segment to match on.
  `uninstall`, `disable`, and `scaffold` deliberately stay at Admin: the first two *remove* code from the execution path, and gating them would leave an Admin unable to shut a malicious plugin off during an incident (#6631) (#6639) (@houko)

### Documentation

- Add the missing YAML front matter to the v2026.7.27 release article.
  The publish workflows both key off it: dev.to skips an article with no `title`, and the release Discussion body is extracted with `awk` that prints only after the second `---`, so it came out empty.
  The generator `scripts/changelog-to-article.sh` emits the block correctly — this article was written without it (#6602) (@houko)
- Fix the v2026.7.27 highlight that described `service install --system` as starting LibreFang "at login".
  The feature is a boot-time LaunchDaemon, which is the whole reason it exists, and the authoritative entry a few hundred lines above already said so (#6602) (@houko)
- Correct the NixOS off-host recipe, which annotated its `environmentFile` as "must export an API key".
  An environment file cannot set `api_key`; the only authentication the daemon reads from the unit environment is a dashboard credential pair (#6602) (@houko)
- Document the EveryAPI MCP bridge on the MCP/A2A integrations page (EN + zh mirror), and fix the `[[mcp_servers]]` examples that page already carried.
  The [EveryAPI](https://github.com/everyapi-ai/everyapi) CLI ships an MCP stdio server as `everyapi mcp` and LibreFang is an MCP stdio client, so the integration is two config stanzas and no code: a `[[mcp_servers]]` entry plus the server's name in the target agent's `mcp_servers` allowlist, with `env = []` because the stdio transport already forwards `HOME` and the server reads `~/.config/everyapi/credentials.json` itself.
  The page also carries an optional `~/.librefang/mcp/catalog/everyapi.toml` entry for `librefang mcp add everyapi`, the 15-tool read/write inventory, and the fact that LibreFang's `mcp_{server}_{tool}` namespacing does not de-duplicate — EveryAPI's already-`everyapi_`-prefixed tools become `mcp_everyapi_everyapi_*`, so an approval glob written against the single-prefix form silently matches nothing.
  Four `require_approval` globs gate all 8 write tools and leave the 7 read tools free, which is what keeps the unattended balance-check use case working; a blanket `mcp_everyapi_*` would stall it on an approval prompt.
  The security section states the bypasses rather than implying the globs are a hard block: `require_approval` pauses for a human instead of denying, `trusted_senders` does NOT waive it for MCP tools (see the classifier fix below), a channel `allowed_tools` rule short-circuits ahead of the list, a `hand:`-tagged agent auto-approves everything, `cache_approvals_per_session` defaults to `true` so one approval covers the rest of the session, and per-user RBAC `NeedsApproval` is the only gate all of those respect.
  A new `crates/librefang-extensions/tests/everyapi_catalog_entry.rs` extracts the fenced TOML straight out of the MDX and runs it through the real loader and the production `format_mcp_tool_name`, so the documented catalog entry, config stanza, and approval globs cannot drift from the code (#6589) (@houko)
- Fix every unparseable `[[mcp_servers]]` example in the docs and add a test that walks the whole tree so the class cannot come back.
  `McpServerConfigEntry` is `deny_unknown_fields`, so a stanza in the pre-`transport` schema is not a stylistic slip that degrades gracefully — it is a hard parse error that takes the daemon's entire `config.toml` down with it, and a user copying one got a daemon that would not boot.
  Twelve stanzas across six page pairs were broken in three distinct shapes, all copy-paste descendants of an older schema: top-level `command` / `args` with `env = { … }` as a table on the MCP/A2A integrations page, top-level `command` / `args` on the operations FAQ, a bare top-level `url` on the core configuration page, and a `[mcp_servers.transport.Http]` sub-table naming the enum variant in the table path instead of carrying the `type` tag that `#[serde(tag = "type")]` requires on the features configuration page.
  `crates/librefang-extensions/tests/docs_mcp_servers_examples.rs` now walks every `.mdx` under `docs/src/app`, extracts each stanza out of the fenced TOML (absorbing `[mcp_servers.…]` sub-tables so a multi-table entry is not falsely reported), and deserializes it through the real type; all 36 pass, new pages are covered with no list to maintain, and the test was mutation-checked against all three broken shapes (#6592) (@houko)
- Document how `[approval].trusted_senders` composes with `[[users]]` RBAC on the approvals security page (EN + zh mirror): the two are separate trust surfaces and the per-user RBAC gate is evaluated first, so an ID listed in `trusted_senders` that is not also a registered `[[users]]` on the `api` channel still has its low-risk tools (e.g. `memory_*`) gated by the `guest_gate`, because the forced-approval verdict short-circuits before the `trusted_senders` bypass is consulted; the new subsection gives the concrete fix (register the operator as a `[[users]]` bound to the `api` channel with a `tool_policy` covering the tools it drives) and notes that with no `[[users]]` configured `trusted_senders` works standalone (#6492) (@houko)
- Document per-user provider-key precedence on the provider-management page (EN + zh mirror), ratifying #6460 OQ#5 that a user's own stored key wins over the operator's rotation: the new "Per-user keys and rotation precedence" subsection states the full resolution order (org allowlist > agent-pinned `api_key_env` > the user's stored key via `PUT /api/users/{name}/provider-keys/{provider}` > `credential_pools` > `provider_api_keys` / `auth_profiles` rotation > catalog / convention env), explains that a user key bypasses the pool and rotation for that provider so upstream spend and chargeback attach to the human who brought the key while an agent-pinned `api_key_env` still overrides it, and names the two operator-facing failure modes — a user key is a hard single point of failure on the primary provider with no same-provider fall-through to the operator's pool, and a configured fallback chain can fail a user's key over onto the operator's credential for a different fallback provider they have not supplied a key for; docs-only, the `resolve_driver_for_owner` resolver already implements this behavior (#6460) (@houko)
- Document installation from the signed project-maintained Arch Linux pacman repository while AUR account registration is unavailable (#6386) (@pavver)
- Add `docs/architecture/multi-replica-rfc.md`, which enumerates every singleton subsystem that blocks running more than one daemon replica and proposes a four-phase path through them.
  Replacing SQLite is necessary and nowhere near sufficient: 24 named background workers, the in-process session locks, the audit hash chain, and the in-memory cost-reservation ledger each break in a different way under a second replica, and the document assigns a coordination mechanism to each rather than leaving "HA" as an open aspiration.
  The storage and coordination decisions are explicitly marked as needing maintainer approval before any implementation starts (#6634) (#6638) (@houko)

### Added

- Give the master api_key env/vault indirection and a hashed form, and close the hash-only WS/terminal auth bypass (#6667) (@houko)

### Fixed

- Inherit global exec_policy on hand activation and stop inferring autonomy from max_iterations (#6603) (@houko)
- Expose writable config fields in GET /api/config and guard against read/write drift (#6604) (@houko)
- Require an explicit [autonomous] or schedule declaration to start a hand loop (#6610) (@houko)
- Merge partial identity PATCHes and flag tool_allowlist entries that cannot grant (#6615) (@houko)
- Expose non-writable security config on read and guard writable paths against missing fields (#6618) (@houko)
- Base the status indicator on supervisor liveness instead of shared per-type traffic (#6620) (@houko)
- Render every approval decision distinctly instead of labelling unknown states "Edited" (#6621) (@houko)
- Let the global require_approval list survive exec_policy Full, and guard daemon lifecycle commands (#6622) (@houko)
- Repair the kubernetes workflow YAML, which never parsed, and guard the whole class (#6643) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Fix typo in prompts.rs comment (#6655) (@houko)

### Maintenance

- Update model snapshot (#6591) (@houko)
- Bump the cargo-minor-patch group with 10 updates (#6597) (@app/dependabot)
- Bump serial_test from 3.5.0 to 4.0.1 (#6598) (@app/dependabot)
- Bump jsonwebtoken from 10.4.0 to 11.0.0 (#6599) (@app/dependabot)
- Bump base64 from 0.22.1 to 0.23.0 (#6601) (@app/dependabot)
- Update model snapshot (#6616) (@houko)
- Bump docker/login-action from 4.4.0 to 4.5.2 in the actions-minor-patch group (#6626) (@app/dependabot)
- Bump actions/stale from 10.4.0 to 11.0.0 (#6627) (@app/dependabot)
- Bump the web-minor-patch group in /web with 4 updates (#6656) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 6 updates (#6657) (@app/dependabot)
- Bump jsdom from 29.1.1 to 30.0.1 in /crates/librefang-api/dashboard (#6658) (@app/dependabot)
- Bump the docs-minor-patch group in /docs with 5 updates (#6685) (@app/dependabot)

</details>


## [2026.7.27] - 2026-07-27

_33 PRs from 2 contributors since v2026.7.21._

### Highlights

- **EveryAPI integration** — connect EveryAPI as a model provider via `librefang models connect everyapi` from the CLI or the new Providers page connect action, with a built-in wiring doctor check
- **macOS boot-time service** — `service install --system` installs a LaunchDaemon so LibreFang starts automatically at login without a running user session
- **NixOS & additional Linux distros** — first-class NixOS deployment support with deepin/Debian distro awareness out of the box
- **Audit log integrity** — audit entries are now preserved (WORM-protected) when an agent is purged, preventing accidental evidence loss
- **Tool result and context fidelity fixes** — stops lossily truncating tool results in the spill dead band, and correctly honors `context_window` from `agent.toml` on all turn-execution paths

### Added

- Emit a failure metric for media-understanding (vision/STT) (#6538) (#6551) (@houko)
- First-class NixOS deployment and deepin/Debian distro awareness (#6582) (@houko)
- Add `librefang models connect everyapi` and an EveryAPI wiring doctor check (#6583) (@houko)
- Add `service install --system` for a boot-time LaunchDaemon on macOS (#6584) (@houko)
- Add an EveryAPI connect action to the Providers page (#6586) (@houko)

### Fixed

- Override sharp to >=0.35.0 to clear the libvips high advisory (#6546) (@houko)
- Include openrouter-models.snapshot.json in the flake source (#6547) (@houko)
- Anchor credential-prefix secret patterns to stop false positives (#6541) (#6548) (@houko)
- Make skills/reload honest in frozen Stable mode (#6540) (#6549) (@houko)
- Stop lossily truncating tool results in the spill dead band (#6545) (#6550) (@houko)
- Bump next to 16.2.11 to clear the App Router security advisories (#6557) (@houko)
- Don't delete audit_entries when purging an agent (WORM integrity) (#6553) (#6558) (@houko)
- Apply busy_timeout to every pooled connection in modify() test helper (#6561) (@houko)
- Treat blank parent_id / agent_id as absent instead of 404 (#6577) (@houko)
- Collect new_name on clone, reflect mcp_servers grants in Tools tab (#6578) (@houko)
- Resolve callback message_id from either metadata shape or the native id (#6579) (@houko)
- Honour agent.toml context_window on the paths that run a turn (#6580) (@houko)
- Install and search from the synced registry checkout (#6581) (@houko)
- Give the TUI a process-lifetime runtime (startup panic + silent session-summary loss) (#6585) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Move the misplaced `### Fixed` block out of the `### Added` list (#6587) (@houko)

### Maintenance

- Update model snapshot (#6539) (@houko)
- Bump the actions-minor-patch group across 1 directory with 4 updates (#6542) (@app/dependabot)
- Bump actions/labeler from 6.2.0 to 7.0.0 (#6543) (@app/dependabot)
- Bump actions/setup-python from 6 to 7 (#6544) (@app/dependabot)
- Update model snapshot (#6552) (@houko)
- Bump the web-minor-patch group in /web with 10 updates (#6554) (@app/dependabot)
- Bump the dashboard-minor-patch group across 1 directory with 12 updates (#6555) (@app/dependabot)
- Bump @testing-library/jest-dom from 6.9.1 to 7.0.0 in /crates/librefang-api/dashboard (#6556) (@app/dependabot)
- Update model snapshot (#6563) (@houko)
- Bump the docs-minor-patch group in /docs with 6 updates (#6567) (@app/dependabot)
- Update model snapshot (#6571) (@houko)
- Update model snapshot (#6574) (@houko)
- Update model snapshot (#6576) (@houko)

</details>


## [2026.7.21] - 2026-07-21

_61 PRs from 4 contributors since v2026.7.11._

### Highlights

- **Per-user LLM provider credentials** — users can bring their own API keys with per-owner spend attribution, org-wide provider allowlists, and budget enforcement on authenticated turns
- **MCP resources primitive** — agents can now access MCP-exposed resources, rounding out the MCP integration beyond tools alone
- **Multi-user knowledge graph scoping** — the knowledge graph is now partitioned per user on shared agents, closing a cross-user data leak
- **Slack multi-step progress** — long-running agent tasks surface live phase updates in Slack via Block Kit message edits instead of a single final reply
- **HAND.toml online editing & expanded delivery targets** — edit hand manifests directly from the Hands panel in the dashboard, and delivery-target channel presets now reach all sidecar adapters

### Added

- Env opt-out (TELEGRAM_STREAMING) for the streaming path (#6482) (@houko)
- Per-user LLM provider credentials with per-owner usage attribution (initial) (#6483) (@houko)
- Org-wide LLM provider allowlist (fail-closed at driver resolution) (#6484) (@houko)
- Slack multi-step progress display via AgentPhase-driven Block Kit updates (#6487) (@houko)
- Per-user attribution survey + API-level user filtering of audit queries (#6488) (@houko)
- Process_start completion notification via the async task tracker (#6489) (@houko)
- Edit HAND.toml online from the Hands panel (#6490) (@houko)
- Scope the knowledge graph per user (peer_id) on multi-user agents (#6494) (#6502) (@houko)
- Expand delivery-target channel presets to all sidecar adapters (#6506) (@houko)
- Owner-gated CRUD for per-user provider credentials (#6460) (#6509) (@houko)
- Implement the MCP resources primitive (#6501) (#6532) (@houko)

### Fixed

- Prefer the live model catalog with a build fallback (#6384) (@pavver)
- Security and correctness hardening from repo-wide audit (#6438) (@houko)
- Second-pass security and correctness hardening from repo-wide audit (#6439) (@houko)
- Third-pass security and correctness hardening from repo-wide audit (#6441) (@houko)
- Fourth-pass security and correctness hardening from repo-wide audit (#6446) (@houko)
- Resolve four reported bugs (#6423, #6442, #6443, #6444) (#6449) (@houko)
- Enforce cross-account channel_send guard through the /mcp bridge (#6443) (#6455) (@houko)
- Trust operator env allowlist in sandbox_command (#6465) (@houko)
- Treat retired pnpm audit endpoint as skip, not a dependency issue (#6466) (@houko)
- Field-scope dm_policy/group_policy so a partial override stops silently gating groups, and expose them on [[sidecar_channels]] (#6445) (#6468) (@houko)
- Distinguish context and budget limits (#6479) (@houko)
- Allow dashboard login script under CSP (#6480) (@houko)
- Treat auto_dream fork tool calls as system-internal so RBAC does not gate them (#6485) (@houko)
- Login page unreadable in light theme (CSS cascade source-order bug) (#6486) (@houko)
- Pin login_page.html to LF so the CSP-hash test passes on Windows (#6481) (#6496) (@houko)
- Gate compacted summary by owning session to stop cross-user prompt leak (#6493) (#6497) (@houko)
- Honour glob patterns in per-agent tool_allowlist/tool_blocklist (#6495) (#6498) (@houko)
- Approvals approve 415 false-success + status column in approvals list (#6492) (#6500) (@houko)
- Stop over-blocking MCP arguments that carry a long numeric id (#6499) (#6503) (@houko)
- Route post-approval reply through account-qualified outbound (#6492) (#6511) (@houko)
- Surface mid-stream provider errors instead of empty/garbled turns (#6512) (@houko)
- Release the token reservation on drop to stop a quota self-DoS (#6513) (@houko)
- Attribute owner-key spend and enforce per-user budget on authenticated API turns (#6514) (@houko)
- Honor response_format in the Gemini and Vertex AI drivers (#6515) (@houko)
- Treat [browser] config-reload as restart-required, not a false hot-reload (#6516) (@houko)
- Canonicalize provider before the per-user key lookup to stop an alias chargeback leak (#6517) (@houko)
- Serialize a canonical-session override on the per-agent lock to stop a lost-update race (#6518) (@houko)
- Scope MCP knowledge_add_* writes to the calling agent, not agent_id="" (#6519) (@houko)
- Recognize CHANGELOG attribution on a bullet's continuation lines (#6520) (@houko)
- Don't orphan another agent's relations when deleting a shared entity's first-writer (#6522) (@houko)
- Honor auto_approve and return 409 on double-resolve (#6492) (#6528) (@houko)
- Surface on-disk upload path to agents for every file type (#6531) (@neo-wanderer)

### Changed

- Normalize on-disk upload naming to <uuid>.<ext> (#6530) (#6536) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Explain trusted_senders vs [[users]] RBAC composition (#6492) (#6507) (@houko)
- Document per-user key precedence over operator rotation (#6460) (#6510) (@houko)

### Maintenance

- Bump the cargo-minor-patch group with 10 updates (#6452) (@app/dependabot)
- Bump tokio-tungstenite from 0.29.0 to 0.30.0 (#6453) (@app/dependabot)
- Update yanked spin 0.9.8 to 0.9.9 (#6454) (@houko)
- Bump the actions-minor-patch group with 3 updates (#6456) (@app/dependabot)
- Bump actions/setup-node from 6.4.0 to 7.0.0 (#6457) (@app/dependabot)
- Lock in env trust split across defer→approve→resume (follow-up to #6465) (#6467) (@houko)
- Bump the web-minor-patch group in /web with 5 updates (#6472) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 6 updates (#6473) (@app/dependabot)
- Bump @eslint/js from 9.39.4 to 9.39.5 in /crates/librefang-api/dashboard (#6474) (@app/dependabot)
- Bump serde_with from 3.18.0 to 3.21.0 (#6475) (@app/dependabot)
- Bump the docs-minor-patch group in /docs with 4 updates (#6491) (@app/dependabot)
- Update model snapshot (#6523) (@houko)
- Bump wasmtime from 46.0.1 to 47.0.1 (#6527) (@app/dependabot)
- Bump the cargo-minor-patch group across 1 directory with 17 updates (#6533) (@app/dependabot)
- Migrate librefang-acp to agent-client-protocol 1.3.0 (supersedes #6526) (#6534) (@houko)

</details>


## [2026.7.10] - 2026-07-10

_40 PRs from 4 contributors since v2026.6.29._

### Added

- Surface the model the Codex CLI is configured to run (#6365) (@houko)
- Complete and proofread dashboard and website translations (#6376) (@houko)

### Fixed

- Clear cargo-deny advisory failures on main (#6366) (@houko)
- Keep [Unreleased] at the top; prune stale buried entries (#6367) (@houko)
- Clear quick-xml RUSTSEC-2026-0194/0195 advisories (#6387) (@houko)
- Convert Markdown to Slack mrkdwn in the Slack sidecar (#6397) (@neo-wanderer)
- Clear crossbeam-epoch RUSTSEC-2026-0204 advisory (#6400) (@houko)
- Never re-spill read_artifact results at the post-tool chokepoint (#6406) (@houko)
- Request extended thinking via reasoning_effort in the OpenAI-compat driver (#6407) (@houko)
- Add LIBREFANG_REGISTRY_OFFLINE to skip registry network refresh in tests (#6408) (@houko)
- Correlate chat turns with their WS terminal frames (#6419) (@houko)
- Single-source the invisible-char set; skip reply-precheck on captionless media (#6426) (@houko)
- Drop redundant refs in TUI format! args (Rust 1.97 clippy) (#6428) (@houko)
- Reliable CLI→PyPI publishing — stable-only gate + stop desktop deleting CLI assets (#6433) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Add Arch Linux pacman installation instructions (#6386) (@pavver)
- Only the WhatsApp sidecar reads DM/group policy env vars (#6405) (@houko)
- Install CLI from official homebrew-core (#6414) (@houko)
- Wrap homebrew-core note at sentence boundaries (#6415) (@houko)

### Maintenance

- Bump rmcp from 1.7.0 to 2.0.0 (#6364) (@app/dependabot)
- Adopt tera 2.0 (#6368) (@houko)
- Adopt aes-gcm 0.11 and the cargo-minor-patch bumps (#6369) (@houko)
- Gitignore the AUR CI deploy key pair (#6370) (@houko)
- Bump the actions-minor-patch group with 2 updates (#6373) (@app/dependabot)
- Bump tauri-apps/tauri-action from 0.6.2 to 1.0.0 (#6374) (@app/dependabot)
- Bump the web-minor-patch group in /web with 10 updates (#6377) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 15 updates (#6378) (@app/dependabot)
- Bump the docs-minor-patch group in /docs with 10 updates (#6380) (@app/dependabot)
- Bump @types/node from 25.9.3 to 26.1.0 in /docs (#6381) (@app/dependabot)
- Raise job timeout to 120 minutes for cold builds (#6389) (@houko)
- Bump the cargo-minor-patch group with 5 updates (#6394) (@app/dependabot)
- Bump the actions-minor-patch group with 5 updates (#6401) (@app/dependabot)
- Make webhook-agent happy-path test hermetic (#6402) (@houko)
- Export LIBREFANG_REGISTRY_OFFLINE in the workspace test lanes (#6410) (@houko)
- Bump the web-minor-patch group in /web with 3 updates (#6411) (@app/dependabot)
- Bump typescript from 6.0.3 to 7.0.2 in /web (#6412) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 6 updates (#6413) (@app/dependabot)
- Move stable CLI to homebrew-core and fix tap sync bugs (#6416) (@houko)
- Match Homebrew class_s casing for versioned tap formula (#6418) (@houko)
- Seed registry content from a pinned in-repo fixture instead of the network (#6421) (@houko)
- Bump the docs-minor-patch group in /docs with 4 updates (#6424) (@app/dependabot)

</details>


## [2026.6.29] - 2026-06-29

_14 PRs from 4 contributors since v2026.6.26-beta.24._

### Highlights

- **Korean language support** — full UI, CLI/TUI, and error message translations added (233 keys covered)
- **ARM64 Linux packages** — aarch64 binaries now published alongside x86_64 via AUR and the project's pacman repo
- **Telegram setup resilience** — the setup form stays available after a describe timeout instead of disappearing
- **Codex CLI flexibility** — Codex CLI can now be used outside of Git repositories
- **Mixed-media message enrichment** — coalesced batches with mixed content types are now correctly enriched on the debounced path

### Added

- UI Korean translation (#6349) (@seungjin)
- Complete Korean error translations (43 → 233 keys) (#6353) (@houko)
- Add Korean (ko) translation for the CLI/TUI (#6356) (@houko)
- Publish aarch64 packages alongside x86_64 (#6334) (#6358) (@houko)

### Fixed

- Bump pdf-extract 0.10→0.12 to patch lopdf RUSTSEC-2026-0187 (#6339) (@houko)
- Keep Telegram setup form available after describe timeout (#6345) (@pavver)
- Allow Codex CLI outside Git repositories (#6347) (@pavver)
- Enrich coalesced mixed-media batches on the debounced path (#6348) (#6351) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Symlink legacy NDK binutils so vendored OpenSSL cross-compiles for Android (#6335) (@houko)
- Put NDK bin on PATH so openssl-src finds the legacy ranlib symlink (#6338) (@houko)
- Enable auto-merge instead of forcing --admin (#6340) (@houko)
- Publish AUR packages on release (#6334) (#6341) (@houko)
- Publish project-maintained pacman repo to R2 (#6334) (#6352) (@houko)

### Other

- Fix[flake.nix]: Add perl to nativeBuildInputs (#6346) (@FrantaNautilus)

</details>


## [2026.6.26] - 2026-06-26

_10 PRs from 2 contributors since v2026.6.24-beta.23._

### Added

- Add Ukrainian localization and extract hardcoded copy (#6312) (@pavver)
- Add AUR packaging for Arch Linux (#6314) (@pavver)
- Surface run params, errors, and one-click re-run (#6292) (#6324) (@houko)
- Allow a custom model id when editing an agent (#6318) (#6327) (@houko)

### Fixed

- Disable redirect following on OAuth HTTP clients (SSRF + credential leak) (#6315) (@houko)
- Block separator-less secret env names from WASM guests (#6316) (@houko)
- Guard gc_sweep running_tasks removal with task_id (TOCTOU) (#6317) (@houko)
- Describe inbound images on the debounced channel path (#6321) (#6323) (@houko)
- Accept empty-recipient HMAC so bootstrap_peers can connect (#6330) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Pin claude_code resolved-model parsing (#6318) (#6331) (@houko)

</details>


## [2026.6.24] - 2026-06-24

_33 PRs from 5 contributors since v2026.6.22-beta.22._

### Added

- Localize TUI Onboarding Wizard and Agents screen (#6253) (@pavver)
- Pluggable context-rewrite modules — per-agent engine + host-run request_llm_summary (closes #6264) (#6287) (@houko)
- Add scriptable tool result transform hook (#6291) (@pavver)
- Per-step errors and re-run with same parameters (#6292) (#6302) (@houko)
- Code_search workspace regex search tool (#6295) (#6307) (@houko)

### Fixed

- Gate provider hourly token budget pre-call so exhaustion flags the fallback chain (#5980) (#5988) (@DaBlitzStein)
- Persist agent skill & MCP-server assignments to agent.toml (#6046) (@DaBlitzStein)
- Embed developer-loop placeholder in first result delivery (closes #6251) (#6254) (@maoxin1234)
- Kill sidecar child on shutdown + forward async delegation result to channel (#6267) (@DaBlitzStein)
- Complete dashboard i18n coverage (#6271) (@pavver)
- Vendor OpenSSL unconditionally so cross-compiled release targets link (#6279) (@houko)
- Merge updater plugin config into base tauri.conf.json (closes #6270) (#6283) (@houko)
- Handle /new in the TUI chat surfaces (closes #6265) (#6284) (@houko)
- Merge the dual [Unreleased] sections into one at the top (#6285) (@houko)
- Forward web-UI-initiated delegation results to the home channel (refs #6266) (#6286) (@houko)
- AUTH PLAIN fallback for SMTP + expose input/error in workflow runs list (#6293) (@DaBlitzStein)
- Scope GET /api/workflows/{id}/runs to the path workflow (#6298) (@houko)
- Block sandbox escape via intermediate-ancestor symlink on writes to non-existent dirs (#6299) (@houko)
- Reject newline/CR/NUL in secret key to prevent secrets.env line injection (#6300) (@houko)
- Graceful 400 for non-table [memory]/[proactive_memory] in PATCH /api/memory/config (#6301) (@houko)
- Apply api_key_env/base_url in PATCH /api/agents/{id}/config instead of dropping them (#6303) (@houko)
- Wire provider cooldown breaker into the LLM retry path and fix its probe gate (#6305) (@houko)
- Gate scriptable transform-hook tests behind cfg(unix) to unbreak Windows CI (#6306) (@houko)
- Gate use std::sync::Arc behind cfg(unix) to complete the Windows-red fix (#6308) (@houko)
- Install libdbus-1-dev in the release Bump Version job (#6309) (@houko)

### Performance

- JSON-aware token estimation for tool paths (#6281) (@maoxin1234)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Token-estimation accuracy benchmark with multi-tokenizer baselines (#6269) (@maoxin1234)
- Bump the cargo-minor-patch group across 1 directory with 8 updates (#6276) (@app/dependabot)
- Bump wasmtime from 45.0.2 to 46.0.0 (#6277) (@app/dependabot)
- Bump cron from 0.16.0 to 0.17.0 (#6278) (@app/dependabot)
- Bump vulnerable/yanked lockfile deps to clear global CI failures (#6280) (@houko)
- Bump actions/checkout from 6 to 7 (#6296) (@app/dependabot)
- Bump actions/cache from 5.0.5 to 6.0.0 (#6297) (@app/dependabot)

</details>


## [2026.6.22] - 2026-06-22

_1 PR from 1 contributor since v2026.6.22-beta.21._

### Highlights

- **Safer upgrades** — the installer now falls back to the last known-good release and automatically rolls back a failed upgrade instead of leaving the app in a broken state.

### Fixed

- Fall back to installable release, roll back bad upgrades (#6272) (@houko)


## [2026.6.17] - 2026-06-17

_22 PRs from 3 contributors since v2026.6.16-beta.19._

### Added

- Per-conversation agent routing for multi-agent groups (#5323) (#6127) (@houko)
- Passkey (WebAuthn/FIDO2) dashboard login (#5981) (#6129) (@houko)
- Deterministic inbound dispatch — channel-instance binding lookup (#5671 Model A) (#6131) (@houko)
- GitHub/Codeberg registry source selector (#6142) (@houko)
- Gate auto-routing on AutoRouteStrategy, not the "assistant" name (#6139) (#6148) (@houko)
- Propagate W3C traceparent on outbound MCP tool calls (#6128) (#6153) (@houko)
- Report the model codex actually used (#6134) (#6157) (@houko)
- Dock the agent panel as a resizable sidebar with a larger prompt editor (#6154 #6155) (#6164) (@houko)
- The cron-management tool disables jobs instead of deleting them (#6159) (#6165) (@houko)
- Enlarge TOML view, edit agent system prompt and tools with reset-to-default (#6150 #6151 #6152) (#6166) (@houko)
- Central prompt repository page with versions and agent binding (#6160) (#6167) (@houko)

### Fixed

- Enforce cross-chat dispatch guard through the /mcp bridge (#6117) (#6125) (@houko)
- Take over a stale conversation-ownership claim from a channel-ineligible holder (#5323) (#6132) (@houko)
- Respect `LIBREFANG_HOME` when resolving plugin directory (#6136) (@HuaGu-Dragon)
- Close channel media RBAC bypass and audit findings (#6141) (@houko)
- Keep Save actionable after a passing Test (#6144) (#6146) (@houko)
- Refetch hand settings after save so inputs persist (#6145) (#6147) (@houko)
- Show the correct Hand agent name in the sessions view (#6156) (#6162) (@houko)
- Build vendored OpenSSL on Windows so webauthn-rs links (#6161) (#6163) (@houko)
- Pin vendored OpenSSL to Strawberry Perl on the Windows test lane (#6171) (@houko)

### Changed

- Lift tool dispatch table to typed ToolError (#3576 slice 5) (#6124) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Bump the actions-minor-patch group with 2 updates (#6140) (@app/dependabot)

</details>


## [2026.6.16] - 2026-06-16

_18 PRs from 3 contributors since v2026.6.11-beta.18._

### Highlights

- **External Skill Registry** — agents can now discover and consume skills hosted on a Codeberg registry, with diff and propose-to-registry support for pending evolution drafts
- **Persistent MCP Server Config** — MCP server configurations are stored in SQLite and survive restarts; runtime writes to `/api/mcp/servers` are also persisted
- **Ukrainian Language Support** — backend and web UI are now fully localized in Ukrainian
- **DeepSeek V4 Pro Reasoning** — DeepSeek v4-pro is now treated as a thinking-with-tools model so `reasoning_content` is correctly echoed through
- **WhatsApp Voice Notes & Matrix Memory** — ElevenLabs voice notes send as Ogg/Opus with proper MIME sniffing; Matrix peers with colons in their IDs can now use the Memory tool

### Added

- Consume a Codeberg-hosted skill registry via registry.registry_host (#6095) (#6103) (@houko)
- Diff + propose-to-registry for pending evolution drafts (#5819) (#6104) (@houko)
- SidecarChannelConfig.agent + available_agents (#5671 PR-A) (#6105) (@houko)
- SQLite-backed MCP server config storage + boot merge (#6021) (#6106) (@houko)
- Add Ukrainian language support for backend and web UI (#6109) (@pavver)
- Persist /api/mcp/servers writes to a DB store via mcp_runtime_store (#6113) (#6115) (@houko)

### Fixed

- Accept `version` field in ClawHubInstallRequest (#6038) (#6039) (@DaBlitzStein)
- Stage Skills-tab edits behind a Save button (#6042) (@DaBlitzStein)
- Refresh detect-secrets baseline for migrated Cloudflare account_id (#6093) (@houko)
- Treat deepseek-v4-pro as thinking-with-tools so reasoning_content is echoed (#6098) (@DaBlitzStein)
- Preserve caller-supplied channel name case in channel_send (#6078) (#6101) (@houko)
- Percent-encode colons in peer_id so Matrix peers can use Memory (#6100) (#6102) (@houko)
- Pin brace-expansion override to 2.0.2 to unbreak the Cloudflare docs build (#6110) (@houko)
- Send ElevenLabs voice notes as Ogg/Opus and sniff audio mime (#6116) (#6118) (@houko)

### Changed

- Migrate web_search.rs to ToolError (#3576 slice) (#6107) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Migrate worker config to librefang Cloudflare account (#6092) (@houko)
- Scope frontend pnpm audit to production deps (#6108) (@houko)
- Free runner disk space before the integration shard build (fixes ENOSPC on main) (#6112) (@houko)

</details>


## [2026.6.11] - 2026-06-11

_8 PRs from 2 contributors since v2026.6.10-beta.17._

### Added

- **mcp/api: `mcp_runtime_store = "db"` persists `/api/mcp/servers` writes to SQLite instead of `config.toml`, so MCP servers can be managed at runtime when the config file is read-only** (#6113) (@houko).
  #6106 added the DB-backed `mcp_server_configs` store and a boot-time merge, but the API write-path (`POST` / `PUT` / `DELETE /api/mcp/servers`, the taint patch) and the read-path still only saw `config.toml`, so a DB-managed server was invisible to the API and could not be added at all when `config.toml` was a read-only Kubernetes ConfigMap (the #6021 motivation).
  The new `config.toml: mcp_runtime_store` knob (default `file`, byte-for-byte the prior behaviour) routes writes to the store when set to `db`.
  The boot overlay and `reload_mcp_servers` now share one `McpConfigStore::merge_over` helper — previously the hot-reload path dropped DB-backed servers the boot merge had applied — and the handlers read the effective (file + DB) set, so DB-backed servers are listed, fetched, updated, and deleted like file-backed ones and take effect without a restart.
  Tests: `mcp_config_store::tests::merge_over_*` and the `mcp_runtime_store_db_test` API integration suite.

### Fixed

- **llm-drivers(deepseek): recognise `deepseek-v4-pro` as a thinking-with-tools model so its `reasoning_content` is echoed back** (@DaBlitzStein).
  `deepseek-v4-pro` was excluded from `is_deepseek_v4_thinking_with_tools` on the #4842 assumption that it "works out-of-the-box", but production multi-turn tool-call conversations on it return `400 "The reasoning_content in the thinking mode must be passed back to the API."` — the same echo requirement as V4 Flash.
  A delegated agent running `deepseek-v4-pro` failed every turn once its history contained a tool-call thinking turn, so `agent_send` / shared-queue tasks to it never executed; a sibling agent on the same model only avoided it by never trimming its history.
  The model is now matched (Flash + Pro) so the `Echo` reasoning-echo policy applies and the thinking text is round-tripped intact. Regression in `test_is_deepseek_v4_thinking_with_tools_matches_v4_flash`.
- Persist run state outside the state lock so GET /run never spuriously reports running:false (#6083) (@houko)
- Inject embedded SDK into the sidecar --describe probe so the configure form isn't empty without pip install (#6085) (@houko)
- Encode qrcode_img_content so the login QR is scannable (#6086) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Bump @whiskeysockets/baileys from 6.7.21 to 6.7.22 in /packages/whatsapp-gateway (#6077) (@app/dependabot)
- Bump @types/react from 19.2.16 to 19.2.17 in /web in the web-minor-patch group (#6079) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 3 updates (#6080) (@app/dependabot)
- Free runner disk space before nix build (#6082) (@houko)
- Free runner disk space before the unit-test build (fixes ENOSPC on main) (#6089) (@houko)

</details>


## [2026.6.10] - 2026-06-10

_78 PRs from 6 contributors since v2026.5.31-beta.16._

### Highlights

- **Parallel tool-call dispatch** — agents can now execute multiple tools concurrently (opt-in via config flag), reducing round-trip latency for multi-tool turns.
- **Remote Hand marketplace installs** — Hands can be installed directly from the remote marketplace without manual packaging.
- **Skill evolution approval gate** — `auto_evolve` updates now flow through an approval step, and a new `evolution_mode` gives you control over how skills self-improve.
- **Shell execution trusted-binary shortcut** — opt into `safe_bins_skip_approval` to skip approval prompts for a strict allowlisted set of shell commands.
- **Security hardening across the board** — fixes for SSRF allowlist gaps (IMDS/CGNAT addresses), TOML/query-string injection in agent manifests, OOM vectors in streamed tool calls and sidecar stderr, DNS-rebinding in WASM `net_fetch`, supply-chain audit bypass in zip installs, and a pre-handshake memory-exhaustion DoS; plus credential-redaction and vault KDF correctness fixes.

### Added

- Externalize template routing rules to an overridable TOML (#5946) (@houko)
- Persist goal runs and recover stale runs at boot (#5947) (@houko)
- Activate parallel tool-call dispatch behind config flag (#5948) (@houko)
- Wire RL rollout export producer into AgentLoopEnd hook (#5950) (@houko)
- Execute WASM hooks in the sandbox as pure-compute (#5951) (@houko)
- Remote marketplace install for Hands (#5954) (@houko)
- Opt-in safe_bins_skip_approval for shell_exec (#6000) (@houko)
- Creator_match filter for TaskClaimed / TaskCompleted triggers (#5960) (#6001) (@houko)
- Skill evolution_mode + gate auto_evolve updates through approval (#5844, #5819) (#6003) (@houko)
- Emit cron-fire and auto-disable observability metrics (#6029) (@neo-wanderer)

### Fixed

- Gate skill_evolve_* tools on auto_evolve + skill_workshop flags (#5678) (@DaBlitzStein)
- Correct stale openapi.sha256 baseline to repair main red (#5945) (#5953) (@houko)
- Stop Cargo.lock changes from busting the rust-cache (cold compile) (#5958) (@houko)
- Pre-flight hand role spawns before reactivation teardown (#5959) (@houko)
- Cron day-of-week follows POSIX convention (0 and 7 = Sunday) (#5967) (@DaBlitzStein)
- Atomic compare-and-swap in task_claim to prevent double-claim (#5961) (#5968) (@houko)
- Ship MCP caller context via _meta instead of arguments (#5965) (#5969) (@houko)
- Retry past lost CAS race in task_claim + post-review nits (#5961, #5965) (#5973) (@houko)
- Memory/wiki ACL denials degrade gracefully instead of killing the turn (#5984) (@houko)
- Trigger evaluator self-deadlocks when per-event budget is exhausted (#5977) (#5987) (@DaBlitzStein)
- History fold preserves tool-result content on omit AND parse failure (#5978) (#5991) (@DaBlitzStein)
- Loop-guard block is soft, and a persistent block stall degrades to a real reply (#5979) (#5992) (@DaBlitzStein)
- Propagate per-sidecar account_id for multi-bot isolation (#5955) (#5996) (@houko)
- Make safe_bins_skip_approval a strict subset of the allowlist gate (#6004) (@houko)
- Tolerate <think> preamble in history_fold summary parsing (#6009) (#6011) (@houko)
- Redact images for text-only models via catalog supports_vision (#6010) (#6013) (@houko)
- Assign approved workshop skill to the creating agent (#5989) (#6014) (@houko)
- Cron enable/disable now PUTs with an {enabled} body instead of POSTing a PUT-only route (#6018) (@neo-wanderer)
- Resolve channel_send mirror owner via bindings, not just default_agent (#6023) (@neo-wanderer)
- Daemon_json surfaces error-less 4xx instead of silent success (#6019) (#6024) (@houko)
- Stabilize non-headless Chrome startup under env isolation (#6028) (@app/copilot-swe-agent)
- Explain empty sidecar form + warn on legacy [channels.*] config (#6030) (@houko)
- Chrono_lite_date() returns wrong dates for most of the year (#6048) (@houko)
- Quota/budget time windows compare RFC3339 text lexicographically, ignoring time-of-day (#6049) (@houko)
- Unbounded Vec growth from attacker-controlled streamed tool-call index (OOM) (#6050) (@houko)
- Self-referential $ref in a tool schema overflows the stack (DoS from untrusted MCP/skill schemas) (#6051) (@houko)
- Redact_secrets leaks a real token that follows a short match (#6052) (@houko)
- SSRF allowlist omits 0.0.0.0, CGNAT/Alibaba IMDS, 192.0.0.192, and AWS IMDS hostnames (#6053) (@houko)
- Single-quote dotenv value panics credential resolution (#6054) (@houko)
- WASM net_fetch follows redirects without per-hop SSRF re-validation (DNS-rebinding); misses Azure IMDS (#6055) (@houko)
- TOML injection via unescaped system_prompt / name / tags in generated agent manifests (#6056) (@houko)
- Unauthenticated pre-handshake read can pin a 16 MiB buffer (memory-exhaustion DoS) (#6057) (@houko)
- Non-ASCII snippet offset misalignment; body cap not enforced on rendered bytes (#6058) (@houko)
- Query-string injection via unescaped MiniMax task_id/file_id (#6059) (@houko)
- Apply_patch files_moved counter incremented before the move write succeeds (#6060) (@houko)
- Vault staging-file race across processes; OAuth deny hangs 5 minutes (#6061) (@houko)
- Trim/prune drop in-memory entries even when the SQLite DELETE fails (#6062) (@houko)
- Exec timeout leaks docker process; bind-mount validation never runs (#6063) (@houko)
- Taint_scanning=false silently disables documented always-on credential key-name blocking (#6064) (@houko)
- Auto-update script TOCTOU/symlink exec; skill-install path traversal (#6065) (@houko)
- ClawHub/Skillhub zip install bypasses the supply-chain audit (.pth RCE) (#6066) (@houko)
- Permission bridge serializes all sessions, dropping approval events on broadcast lag (#6067) (@houko)
- Channel error truncation panics on multi-byte UTF-8 boundary (#6068) (@houko)
- Sidecar stderr read is unbounded — same OOM vector already capped for stdout (#6069) (@houko)
- Describe_event panics on multi-byte Custom payload; correct false test-env safety claim (#6070) (@houko)
- Vault KDF uses volatile Argon2::default() while on-disk format stores no params (#6071) (@houko)
- Allow unused_mut on chromium launch args off-Linux (#6072) (@houko)

### Changed

- Split role-trait god-file into per-domain modules (#5970) (@houko)
- Split the 14.6k-line main.rs into per-command modules (#5971) (@houko)
- Derive task_claim retry budget from pool size (#5974) (@houko)
- Split routes/agents.rs into per-concern modules (#5975) (@houko)
- Split routes/workflows.rs into per-concern modules (#5985) (@houko)
- Split routes/skills.rs into per-concern modules (#5986) (@houko)
- Split routes/config.rs into per-concern modules (#5993) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Guard against editing a re-created worktree on a stale base (#6002) (@houko)

### Maintenance

- Populate sessions.peer_id on save (#5286) (@f-liva)
- Make required-status-checks enforceable — CI Gate, aarch64 lane, openapi-drift fix (#5943) (@houko)
- Merge_group support (prereq for merge queue) [stacked on #5943] (#5944) (@houko)
- Extract heartbeat de-dup transition into a testable helper (#5949) (@houko)
- Faster + reliable docker dev iteration — mold linker + per-worktree target (#5952) (@houko)
- Auto-commit regenerated codegen on same-repo PRs (#5994) (@houko)
- Ignore skill scaffolder template TODOs (#5982, #5983) (#5995) (@houko)
- Bump the cargo-minor-patch group with 11 updates (#6006) (@app/dependabot)
- Bump the web-minor-patch group in /web with 9 updates (#6007) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 12 updates (#6008) (@app/dependabot)
- Ignore .github self-scan that spawns false-positive issues (#6012) (@houko)
- Bump the docs-minor-patch group in /docs with 6 updates (#6015) (@app/dependabot)
- Bump next from 15.5.18 to 16.2.7 in /docs (#6016) (@app/dependabot)

</details>


## [2026.5.31] - 2026-05-31

_16 PRs from 2 contributors since v2026.5.30-beta.15._

### Added

- Inline skill assignment on the agent Skills tab (#4917) (#5930) (@houko)
- Port command-policy and message coalescing to sidecar channels (#5931) (@houko)
- Propose evolved skill as PR to registry (#5932) (@houko)
- Ship librefang-sidecar-telegram binary in release tarballs (#5937) (@houko)

### Fixed

- Tool_runner shell — timeout clamp, streaming output, process group kill, Windows compat (#5763) (@leszek3737)
- Tool_runner knowledge — confidence clamp, input validation, result limits, property bounds (#5767) (@leszek3737)
- Tool_runner image — extension whitelist, 50MB limit, BMP i32, JPEG markers, PNG sig (#5768) (@leszek3737)
- Enable agent model Save on any field change (#5917) (#5925) (@houko)
- Empty mcp_servers = [] grants no MCP tools, not all (#5855) (#5928) (@houko)
- Move getpgrp to the x86_64-only seccomp block to unbreak aarch64 (#5929) (@houko)
- Patch rand (0.8.6/0.9.3) and link-preview-js (4.0.1) security advisories (#5934) (@houko)
- Migrate ssh-backend to russh 0.61.1 (clears 5 RustSec advisories) (#5935) (@houko)

### Changed

- Migrate read_artifact to ToolError (error-contracts slice 2) (#5926) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Regression test for #5857 Windows provider-key path validation (#5927) (@houko)
- Skip deleted (410 Gone) issues in auto-close reconciler (#5933) (@houko)
- Rustfmt knowledge.rs to unbreak main Quality (post #5767) (#5938) (@houko)

</details>


## [2026.5.30] - 2026-05-30

_68 PRs from 5 contributors since v2026.5.28-beta.14._

### Added

- Add source attribution to GET /api/tools response (#5679) (@DaBlitzStein)
- Tools tab in agent detail with grouped view (closes #5677) (#5680) (@DaBlitzStein)
- Expose auto_evolve toggle in Skills tab (#5741) (@DaBlitzStein)
- Kanban task board page (#5745) (#5805) (@houko)
- Support custom-URL self-hosted STT/TTS providers (fixes #5740) (#5814) (@houko)
- Rust Telegram sidecar adapter (parity with Python) (#5831) (@houko)
- Just dev --docker + TELEGRAM_LOG tracing (#5833) (@houko)
- Run WASM skill runtime via the runtime WasmSandbox (#5835) (@houko)
- Autonomous long-horizon goal runner (#5840) (@houko)
- Out-of-process `engine = "sidecar"` (#5849) (@houko)
- Scan tool-result content for indirect prompt injection (#5859) (@houko)

### Fixed

- Strip ANTHROPIC_API_KEY when OAuth credentials present (#5292) (@f-liva)
- Reconcile cascade-leak THEMATIC_HEADERS with post-#5053 prompt builder (#5351) (@f-liva)
- Tool_runner sandbox — RAII cleanup, TOCTOU removed, container_id redacted (#5757) (@leszek3737)
- Tool_runner workflow — artifact type check, deterministic sort, recursion limit (#5758) (@leszek3737)
- Tool_runner schedule — AM/PM parsing, minute precision, owner verification, cron validation (#5759) (@leszek3737)
- Tool_runner system — URL const, client reuse, error diagnostics (#5760) (@leszek3737)
- Tool_runner media — size limits, async fs, UUID filenames, ffmpeg deadlock, extension allowlist (#5761) (@leszek3737)
- Tool_runner web_legacy — SSRF protection, streaming body limit, unified UA, status check (#5764) (@leszek3737)
- Tool_runner canvas — XSS escape, whitelist parser, data: URI block, size limit (#5766) (@leszek3737)
- Tool_runner memory — truncation, pagination, key validation (#5770) (@leszek3737)
- Tool_runner agent — taint all inputs, narrow capabilities, deny None, network strict (#5775) (@leszek3737)
- Tool_runner process — output cap, strict caller_id, arg logging, serde_json (#5778) (@leszek3737)
- Tool_runner fs — backslash rejection, canonicalize, TOCTOU fix, read limit, dir pagination, atomic write (#5783) (@leszek3737)
- Route auto_evolve creates through skill_workshop pending queue (#5800) (@DaBlitzStein)
- Reset taint editor state when server prop changes (#5803) (@houko)
- Use catalog api_key_env for custom provider key resolution (#5807) (@houko)
- Regenerate stale openapi schema baseline to repair main red (#5834) (@houko)
- Make DAG-path step timeout error actionable (#5836) (@houko)
- Finish Option::zip migration in kernel tests (clippy 1.96.0) (#5837) (@houko)
- Keep custom providers across restarts, tolerate unknown tier (#5838) (@houko)
- Audit sweep — 5 CRITICAL + 7 HIGH (split-brain, RBAC, decay, dedup, prompt budget, async consolidate) (#5839) (@houko)
- Apply search filter to FangHub skills grid (#5843) (@DaBlitzStein)
- Use Option::zip for hand timestamp pairing (clippy) (#5845) (@houko)
- Close goal-run self-cleanup race + termination test coverage (follow-up #5840) (#5848) (@houko)
- MEDIUM follow-ups — counter map sweep, hot-reload on PATCH, multi-keyword search, configurable UPDATE thresholds (#5850) (@houko)
- Make extra_params / extra_body BTreeMap for deterministic wire-body key order (#5860) (@houko)
- Close trusted_senders all-or-nothing approval bypass for high-risk tools (#5861) (@houko)
- Make subprocess plugin sandbox secure-by-default (#2) (#5862) (@houko)
- Scrub internal errors from 5xx responses to prevent detail leakage (#5863) (@houko)
- Validate hand id as a safe path component to block traversal (#5865) (@houko)
- Apply config hot-reload for read-live fields, not only hot actions (#5867) (@houko)
- Reserve the global USD budget on the streaming dispatch path (#5869) (@houko)
- Bound consolidation candidate load with a per-agent LIMIT (#5871) (@houko)
- Stop logging API key, account cache tokens, keep stream tool ids (#5875) (@houko)
- Cover all per-agent override keys with a drift-guarded detector (#6) (#5876) (@houko)
- Guard agent_msg_locks GC with Arc::strong_count (symmetry with session_msg_locks) (#5877) (@houko)
- Account prompt-cache tokens in usage normalization (#5879) (@houko)
- Handle no-arg tool calls and UTF-8-safe thinking summary (#5882) (@houko)
- Route attachment download through the redirect-revalidating client (#5884) (@houko)
- Pin every redirect hop in web_fetch to close DNS-rebinding window (#5886) (@houko)
- Clean up per-flow OAuth vault entries on all callback exits (#5895) (@houko)
- Scan prompt context for injection at the load/reload boundary (#5897) (@houko)
- Retry transport-layer errors and make retry count configurable (#10) (#5898) (@houko)
- Detect re-entrant keyed agent_send to prevent session-lock deadlock (#5900) (@houko)
- Delimit all fields in the Merkle entry hash to close ambiguity (#5903) (@houko)
- Enforce RBAC on session auth path; offload workflow template write (#5906) (@houko)
- Low-severity correctness — workshop cap race, token saturating, ephemeral comment (#5910) (@houko)
- Keep anthropic stream block alignment; report effective claude_code timeout (#5913) (@houko)
- Gate media link URLs through safeUrl; share urlTransform with streaming view (#5916) (@houko)
- Allowlist glibc-startup syscalls for exec'd plugin binaries (fixes native_runtime_timeout CI failure) (#5920) (@houko)

### Changed

- Unify the three sidecar bridges onto a shared transport crate (#5852) (@houko)

### Performance

- Offload blocking filesystem/zip IO off the tokio runtime (#5892) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Fix three agent-facing architecture drift points (#5901) (@houko)

### Maintenance

- Bump the docs-minor-patch group in /docs with 2 updates (#5847) (@app/dependabot)
- Cargo fmt recently-merged code (repair main Quality fmt) (#5853) (@houko)
- Fix Windows-only red in shell capability test (path-not-found wording) (#5854) (@houko)
- Raise Test / Windows shard timeout 45 → 60 min to match macOS (#5856) (@houko)

</details>

## [2026.5.28] - 2026-05-28

_46 PRs from 5 contributors since v2026.5.25-beta.13._

### Breaking Changes

- Rust sidecar adapter SDK + AI-codegen-era rationale rewrite (#5821) (@houko)

### Added

- Per-agent channel allowlist (#5738) (@DaBlitzStein)
- Implement describe_image() and wire ImageFile description through channel adapters (#5815) (@houko)
- Rust sidecar adapter SDK + AI-codegen-era rationale rewrite (#5821) (@houko)

### Fixed

- Isolate attachment pre-inject per chat session — close cross-chat image leak (#5334) (@f-liva)
- Make migrate path containment existence-independent (fixes #5716) (#5719) (@houko)
- Repair discussion-to-issue backfill — gh api --jq doesn't take --arg (#5754) (@houko)
- Tool_runner taint — unified SECRET_KEYS, substring match, header trim, single-pass normalization (#5762) (@leszek3737)
- Tool_runner shell_safety — command injection hardening, quote-aware tokenizer (#5765) (@leszek3737)
- Tool_runner definitions — ALWAYS_NATIVE complete, OnceLock caches, schema fixes, tool_name constants (#5771) (@leszek3737)
- Tool_runner error — Upstream message preserved, MissingParameter String, ResourceNotFound 404 (#5772) (@leszek3737)
- Tool_runner cron — sender_id override, TOCTOU reduction, HashSet lookup, empty job_id rejected (#5773) (@leszek3737)
- Tool_runner dispatch — mutex split, fallback ACL, ACP args, spill wiring, snapshot ordering (#5774) (@leszek3737)
- Tool_runner spill — config-based threshold, validation, fast-path (#5776) (@leszek3737)
- Tool_runner wiki — limit cap, input validation, safe usize, caller_agent_id required (#5777) (@leszek3737)
- Tool_runner meta — case-insensitive lookup, Cow optimization, deterministic sort (#5779) (@leszek3737)
- Tool_runner task — typed deserialization, contextual errors, empty validation, status default (#5780) (@leszek3737)
- Tool_runner notify — length limit, control char sanitization, PII removal (#5782) (@leszek3737)
- Tool_runner hand — deterministic sort, empty id reject, config whitelist, output sanitization (#5784) (@leszek3737)
- Tool_runner goal — progress type fix, range validation (#5785) (@leszek3737)
- Tool_runner event — event_type validation, caller identity, reserved prefix guard (#5786) (@leszek3737)
- Tool_runner a2a — session_id taint, SSRF diagnostics, zero-alloc agent check (#5787) (@leszek3737)
- Tool_runner artifact — spawn_blocking, explicit errors, usize safe, zero-length reject (#5788) (@leszek3737)
- Tool_runner channel — poll u8 safe, file size limit, email regex, mirror dedup, thread_id routing (#5789) (@leszek3737)
- Skip bridge-side formatting for sidecar adapters (fixes #5795) (#5796) (@DaBlitzStein)
- Return forward-slash relative path from registry/content on Windows (#5801) (@houko)
- Make step timeout errors actionable with remediation guidance (#5806) (@houko)
- Eliminate Instant subtraction that panics on Windows CI (fixes #5726) (#5808) (@houko)
- Seed Feishu/Lark configure form when Python SDK is absent (#5809) (@houko)
- Unbreak coverage build — thread session_id into two SessionWriter test stubs (#5816) (@houko)
- Wiki.rs lifetime + shell.rs test arity after #5774/#5777 (#5818) (@houko)
- Unbreak main — agent channels in ApiDoc + fmt + secrets baseline (#5820) (@houko)
- Install gh CLI for release flow (#5826) (@houko)
- Run `gh auth setup-git` to unblock git push from container (#5827) (@houko)
- Override host-absolute credential helper path inside container (#5829) (@houko)

### Changed

- Migrate tool_runner tools to ToolError (#3576) (#5737) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Bump the cargo-minor-patch group with 4 updates (#5748) (@app/dependabot)
- Bump wasmtime from 44.0.1 to 45.0.0 (#5749) (@app/dependabot)
- Bump sysinfo from 0.38.4 to 0.39.2 (#5750) (@app/dependabot)
- Bump which from 7.0.3 to 8.0.2 (#5751) (@app/dependabot)
- Bump tikv-jemallocator from 0.6.1 to 0.7.0 (#5752) (@app/dependabot)
- Bump the actions-minor-patch group with 4 updates (#5790) (@app/dependabot)
- Bump actions/setup-python from 5 to 6 (#5791) (@app/dependabot)
- Bump the web-minor-patch group in /web with 3 updates (#5810) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 7 updates (#5811) (@app/dependabot)
- Bump globals from 15.15.0 to 17.6.0 in /crates/librefang-api/dashboard (#5812) (@app/dependabot)
- Docker fallback for `just release` when cargo is missing (#5825) (@houko)

</details>


## [2026.5.25] - 2026-05-25

_308 PRs from 7 contributors since v2026.5.17-beta.12._

### Breaking Changes

- Migrate ntfy from in-process adapter to sidecar (P7) (#5224) (@houko)
- Remove in-process telegram adapter (now sidecar-only) (#5241) (@houko)
- Migrate gotify from in-process adapter to sidecar (#5263) (@houko)
- Migrate mastodon from in-process adapter to sidecar (#5264) (@houko)
- Remove 6 low-value channel adapters (#5265) (@houko)
- Drop 12 unmaintained adapters (#5267) (@houko)
- Migrate bluesky from in-process adapter to sidecar (#5277) (@houko)
- Migrate reddit from in-process adapter to sidecar (#5281) (@houko)
- Migrate twitch from in-process adapter to sidecar (#5297) (@houko)
- Migrate rocketchat from in-process adapter to sidecar (#5298) (@houko)
- Migrate discord from in-process adapter to sidecar (#5299) (@houko)
- Migrate nextcloud from in-process adapter to sidecar (#5301) (@houko)
- Migrate slack from in-process adapter to sidecar (#5302) (@houko)
- Migrate webex from in-process adapter to sidecar (#5309) (@houko)
- Migrate zulip from in-process adapter to sidecar (#5310) (@houko)
- Migrate line from in-process adapter to sidecar (#5312) (@houko)
- Migrate mattermost from in-process adapter to sidecar (#5315) (@houko)
- Migrate signal from in-process adapter to sidecar (#5317) (@houko)
- Migrate qq from in-process adapter to sidecar (#5325) (@houko)
- Migrate matrix from in-process adapter to sidecar (#5368) (@houko)
- Migrate feishu from in-process adapter to sidecar (#5380) (@houko)
- Migrate wecom from in-process adapter to sidecar (WebSocket-only) (#5392) (@houko)
- Migrate email from in-process adapter to sidecar (#5408) (@houko)
- Migrate dingtalk from in-process adapter to sidecar (Stream mode only) (#5417) (@houko)
- Migrate wechat from in-process adapter to sidecar (#5421) (@houko)
- Migrate teams from in-process adapter to sidecar (#5433) (@houko)
- Migrate whatsapp from in-process adapter to sidecar (dual-mode) (#5445) (@houko)
- Migrate webhook from in-process adapter to sidecar (#5455) (@houko)
- Migrate google_chat from in-process adapter to sidecar (#5459) (@houko)
- Delete dead per-channel REST endpoints + their helpers (#5463) (@houko)

### Highlights

- **Channel adapter sidecar migration** — all 27 messaging integrations (Slack, Discord, Telegram, WhatsApp, Signal, Teams, and more) are now isolated sidecar processes instead of in-process adapters; 18 unmaintained adapters were removed. Sidecar adapters can be configured directly from the dashboard.
- **Human-in-the-loop (HITL) approval step** — agents can now pause and request operator approval mid-run; approvals route back to the originating chat with inline keyboard buttons on supported adapters, and the same tool only prompts once per session.
- **Credential pools** — configure multiple API keys per LLM provider for automatic round-robin rotation and instant failover on rate limits.
- **Schedule tab & budget visibility** — the dashboard now has an editable Schedule tab for managing triggers, cron jobs, and continuous mode; a new per-provider budget caps surface shows spend and limits per provider.
- **Security hardening** — session tokens are now hashed at rest, SSRF validation added to URL inputs, path-traversal guards tightened across asset and file routes, SQL bindings replace string concatenation in session cleanup, and request bodies are size-capped against pre-allocation DoS.

### Added

- Credential pools — multi-key rotation per provider with… (#5063) (@Chukwuebuka-2003)
- Add per-agent memory isolation via agent_id parameter (#5071) (@leszek3737)
- Propagate W3C traceparent to outbound LLM HTTP requests (#5190) (@neo-wanderer)
- Implement HITL operator-step — notify dispatch, timeout watchdog, HTTP actions→resume (#5133, #5134, #5135) (#5191) (@houko)
- Caller-controlled conversation_key for agent_send (#5212) (@houko)
- Forced /compact with async spawn, ack+event, summary banner (#5213) (@houko)
- Sidecar channel parity — protocol, supervision, config (P0–P3) (#5219) (@houko)
- Python sidecar channel adapter framework (P4) (#5220) (@houko)
- Hard-block new in-process channel adapters (P5) (#5221) (@houko)
- Migrate ntfy from in-process adapter to sidecar (P7) (#5224) (@houko)
- Compute wasMentioned from group_trigger_patterns when mentionedJids is empty (#5230) (@f-liva)
- Telegram full sidecar parity (formatter + full inbound/outbound), stdlib-only (#5232) (@houko)
- Remove in-process telegram adapter (now sidecar-only) (#5241) (@houko)
- Configure sidecar adapters (telegram/ntfy) from dashboard (#5252) (@houko)
- Editable Schedule tab — triggers, cron, continuous mode (#4924) (#5256) (@houko)
- HITL operator-step dashboard surfaces (#4977) (#5257) (@houko)
- Credential pools for multi-key per-provider rotation (#4965) (#5260) (@houko)
- Migrate gotify from in-process adapter to sidecar (#5263) (@houko)
- Migrate mastodon from in-process adapter to sidecar (#5264) (@houko)
- Migrate bluesky from in-process adapter to sidecar (#5277) (@houko)
- Migrate reddit from in-process adapter to sidecar (#5281) (@houko)
- Migrate twitch from in-process adapter to sidecar (#5297) (@houko)
- Migrate rocketchat from in-process adapter to sidecar (#5298) (@houko)
- Migrate discord from in-process adapter to sidecar (#5299) (@houko)
- Migrate nextcloud from in-process adapter to sidecar (#5301) (@houko)
- Migrate slack from in-process adapter to sidecar (#5302) (@houko)
- Migrate webex from in-process adapter to sidecar (#5309) (@houko)
- Migrate zulip from in-process adapter to sidecar (#5310) (@houko)
- Migrate line from in-process adapter to sidecar (#5312) (@houko)
- Migrate mattermost from in-process adapter to sidecar (#5315) (@houko)
- Migrate signal from in-process adapter to sidecar (#5317) (@houko)
- Migrate qq from in-process adapter to sidecar (#5325) (@houko)
- Migrate matrix from in-process adapter to sidecar (#5368) (@houko)
- Migrate feishu from in-process adapter to sidecar (#5380) (@houko)
- Migrate wecom from in-process adapter to sidecar (WebSocket-only) (#5392) (@houko)
- Migrate email from in-process adapter to sidecar (#5408) (@houko)
- Migrate dingtalk from in-process adapter to sidecar (Stream mode only) (#5417) (@houko)
- Migrate wechat from in-process adapter to sidecar (#5421) (@houko)
- Migrate teams from in-process adapter to sidecar (#5433) (@houko)
- Migrate whatsapp from in-process adapter to sidecar (dual-mode) (#5445) (@houko)
- Migrate webhook from in-process adapter to sidecar (#5455) (@houko)
- Migrate google_chat from in-process adapter to sidecar (#5459) (@houko)
- Restore ChannelsPage as a sidecar-only page (#5470) (@houko)
- Embed librefang-sdk + reconnect WeChat QR flow (#5472) (@houko)
- Approval notifications use inline keyboard on interactive-capable adapters (#5483) (@houko)
- Route approval popup to originating chat (follow-up to #5483) (#5484) (@houko)
- Thread chat_id through approval flow for group-chat support (#5489) (@houko)
- Cache per-session approvals so the same tool only prompts once (#5663) (@houko)
- Per-agent [proactive_memory] extraction_model override (#5475) (#5690) (@houko)
- Bootstrap ESLint with jsx-no-target-blank guard (fixes #5561) (#5701) (@houko)
- Propagate kernel-attested caller context to MCP servers (fixes #5699) (#5704) (@houko)
- Expose per-provider budget caps surface (#5705) (@houko)

### Fixed

- Force HOME so spawned CLI can find its credentials (#4997) (@f-liva)
- Distinguish JoinError cancellation from panic in streaming bridge (#5058) (#5064) (@leszek3737)
- Spill oversized MCP/tool results to artifact store before truncation (#5149) (@neo-wanderer)
- Deny unknown fields in request DTOs to catch body typos (#5131) (#5151) (@houko)
- Validate expression at insert and auto-disable on repeated fallback (#5160) (@houko)
- Unwedge cooldown on wall-clock backstep (#5162) (@houko)
- Respect per-agent fallback_models override — None inherits global, Some([]) opts out (#5167) (@DaBlitzStein)
- Serde/config polish (#5145) (#5172) (@houko)
- AuxClient inherits agent fallback chain when [llm.auxiliary] unset (#5169) (#5173) (@houko)
- Cap rate-limited autonomous loop re-fires (#5168) (#5174) (@houko)
- Time/clock/scheduling robustness (#5136) (#5175) (@houko)
- Surface swallowed errors on persistence/IO paths (#5137) (#5176) (@houko)
- Enforce prompt-cache key determinism (#5143) (#5177) (@houko)
- Security defense-in-depth — symlink/archive/header/IP edge cases (#5141) (#5178) (@houko)
- Enforce per-user memory/wiki ACL at tool dispatch (#5139) (#5179) (@houko)
- Concurrency hazard follow-ups — kill_agent run/abort lifecycle (#5142) (#5180) (@houko)
- Memory substrate data integrity (#5138) (#5181) (@houko)
- Data-layer invalidation + a11y + dead code (#5140) (#5182) (@houko)
- Task lifecycle / resource-leak follow-ups (#5144) (#5184) (@houko)
- Reject same-task re-entrant agent_msg_lock acquisition (#5125, #5126) (#5187) (@houko)
- Show full agent name on hover in chat sidebar (#5188) (@neo-wanderer)
- Regenerate OpenAPI/SDK/schema baselines for #5151 DTO changes (#5165) (#5189) (@houko)
- Prevent history_fold mid-string truncation on verbose-JSON models (#5206) (@houko)
- Re-enable send button on typing:stop (#5207) (@houko)
- /context reports real model context window (#5208) (@houko)
- Surface config deserialize errors and fail closed on hard parse failure (#5209) (@houko)
- Honor token-trigger in inner compaction gate (#5210) (@houko)
- Canonical session pointer recovery on restart (#5198, #5199) (#5211) (@houko)
- Cover ChainExhausted in PooledDriver match (unblock main) (#5215) (@houko)
- Restore rustfmt-clean main after #5209 (#5214) (#5216) (@houko)
- Expose background section + drop stale /api/cron/list allowlist row (#5217) (@houko)
- Sidecar protocol/SDK follow-ups from #5219/#5220 review (#5223) (@houko)
- Move first-party channel adapters out of examples into librefang-sdk (#5228) (@houko)
- Unwrap ephemeral/viewOnce/edited wrappers before reading contextInfo (closes #48) (#5229) (@f-liva)
- Surface producer crash via ProducerCrashed, not SystemExit (#5231) (@houko)
- Handle inbound poll_answer in telegram adapter (sidecar parity) (#5242) (@houko)
- Close kill_agent/dispatch race + break HITL self-cycle (#5244 follow-ups) (#5244) (@houko)
- Unblock main — pass force=false in compact gate test (#5210/#5213 collision) (#5245) (@houko)
- Sidecar channels visible AND read-only on the dashboard (no 404 actions) (#5249) (@houko)
- Surface telegram/ntfy discovery rows on the channels page (#5250) (@houko)
- Auto-pin agentId-only sessions + bind dropdown active to live connection (#5199) (#5253) (@houko)
- Cron picker click no longer closes schedule form (#5247) (#5254) (@houko)
- Agent wizard tools/skills selectable + MCP servers dropdown (#5246) (#5255) (@houko)
- Follow-ups from third sidecar-configure review (#5261) (@houko)
- Block cross-chat memory bleed via chat-scoped recall (#5227) (#5262) (@houko)
- Patch Baileys executeInitQueries to non-blocking allSettled (#5268) (@f-liva)
- Align opentelemetry stack on 0.32 to fix main build break (#5279) (@houko)
- Include kernel Bearer token on all REST forwards (#5285) (@f-liva)
- Thread sender context through streaming message handler (#5288) (@f-liva)
- Skip file-upload OCR for image/* mime types (closes #5290) (#5291) (@DaBlitzStein)
- Add default_agent to SidecarChannelConfig — restore inbound routing pin (closes #5294) (#5295) (@DaBlitzStein)
- Restore main — fmt drift, MCP caller_agent_id semantics, openapi baseline (#5300) (@houko)
- Honour Retry-After across sidecar polling adapters (#5303) (@houko)
- Emit poll bursts in chronological order across sidecar adapters (#5305) (@houko)
- Restore main — fmt drift + stale config schema baseline (#5307) (@houko)
- Detect chat-template `[User]` line-leader as cascade leak (#5308) (@f-liva)
- Update openclaw test fixtures after mattermost sidecar (closes #5316) (#5318) (@houko)
- Wrap config sub-tabs + hide number-input spinner buttons (closes #5293) (#5319) (@houko)
- Pin response_format = Json on history_fold + web_augment aux calls (closes #5287) (#5320) (@houko)
- Observability + regression coverage on sidecar reconnect loop (closes #5111) (#5321) (@houko)
- Define api-error-generic across all 6 locales (audit: api-error-generic-missing-fluent-key) (#5322) (@houko)
- Use canonical getStoredApiKey for export download (audit: audit-export-401) (#5324) (@houko)
- Purge pending_approvals on agent cascade-delete + schema-walking guard (audit: agent-cascade-delete-missing-tables) (#5328) (@houko)
- Refuse to boot without LIBREFANG_STATE_SECRET when external_auth.enabled (closes #5336) (#5337) (@houko)
- Validate skill name + hand against path traversal (closes #5338) (#5339) (@houko)
- Wrap upload_routes in route-local RequestBodyLimitLayer (closes #5342) (#5343) (@houko)
- Cap triggers per agent at MAX_TRIGGERS_PER_AGENT = 50 (closes #5345) (#5346) (@houko)
- Verify caller owns from_agent_id before comms_send (closes #5349) (#5350) (@houko)
- SSRF-validate URLs at create + update (closes #5352) (#5353) (@houko)
- Gate require_auth_for_reads=false bypass behind external_auth_proxy (closes #5356) (#5357) (@houko)
- Add /api/auth/callback to rate-limit allowlist (closes #5358) (#5359) (@houko)
- Expect() on serde_json::to_writer in stream_json (closes #5360) (#5361) (@houko)
- Write Argon2id upgrade-hint to 0600 file instead of log (closes #5364) (#5365) (@houko)
- Kernel_err_to_status helper for 404/409 mapping (closes #5366) (#5367) (@houko)
- Require auth on GitHub Copilot OAuth endpoints (closes #5369) (#5370) (@houko)
- Atomic-rename write for secrets.env eliminates 0644 TOCTOU (closes #5371) (#5372) (@houko)
- Scrub raw rusqlite errors before responding (#5378) (@houko)
- Split /api/auth/login allowlist into exact + slash-prefix (#5382) (@houko)
- Always emit Secure on logout cookie clear (#5384) (@houko)
- Anchor [SILENT] cron marker to message prefix (#5386) (@houko)
- Clamp listing endpoints — no more limit=None → full collection (#5388) (@houko)
- Rel=noopener noreferrer + safeUrl on MCP catalog get_url (#5390) (@houko)
- Hand-write Debug to redact OAuthTokens secrets (#5395) (@houko)
- Warn on serde(other) Unknown variants with raw tag (#5397) (@houko)
- Bind cleanup_orphan_sessions IN-clause instead of string-concat (#5401) (@houko)
- Hotfix dangling refs from #5368 + #5380 sidecar migrations (FeishuConfig + pulldown-cmark) (#5402) (@houko)
- Drop dangling channels.feishu access in openclaw roundtrip test (#5404) (@houko)
- Bound regex cache at 4096 entries with FIFO eviction (#5406) (@houko)
- Per-process random anonymous fingerprint (#5410) (@houko)
- Wire check_json_depth into global request middleware (#5412) (@houko)
- Use SHA-256 (128-bit truncated) for DriverCache::cache_key (#5414) (@houko)
- Persist trimmed active_sessions after periodic GC (#5419) (@houko)
- Tighten SQLite database files to 0o600 + data dir to 0o700 (#5422) (@houko)
- Recover sessionWebhook via ChannelUser.librefang_user (#5423) (@houko)
- Sanitize Custom channel names that collide with kernel-internal cron/autonomous/webui (#5425) (@houko)
- Byte + char dual cap on chat-message size (#5427) (@houko)
- Saturating_add inner cache-token sum (#5430) (@houko)
- Recover passive-reply msg_id via ChannelUser.librefang_user (#5431) (@houko)
- Apply foreign_keys=ON + full PRAGMA set to PromptStore second pool (#5434) (@houko)
- Scan raw string for command substitution — close double-quote bypass (#5436) (@houko)
- Recover per-message reply correlation via ChannelUser.librefang_user across 6 sidecars (#5439) (@houko)
- Size-bounded PII regex compilation (#5444) (@houko)
- Repair persisted session after trim+pinned-rescue (#5447) (@houko)
- Recover context_token via librefang_user across sidecar restart (#5448) (@houko)
- Recover req_id via librefang_user across sidecar restart (#5449) (@houko)
- Include traceback + cmd_type when on_command bare-except logs (#5450) (@houko)
- WARN on env-vs-keyring master-key divergence (#5453) (@houko)
- Recover main build from sidecar fallout (missing default + orphans + test drift) (#5456) (@houko)
- Repair test build after #5455 (write_service_account_env removed) (#5460) (@houko)
- Cross-audit follow-ups (Retry-After x4, dedupe x2, LINE reply API) (#5462) (@houko)
- Rewrite ModuleNotFoundError into actionable install hint (#5465) (@houko)
- Preserve specific cause in last_error after circuit-breaker trip (#5468) (@houko)
- Redact WhatsApp JIDs atomically (no partial-redact via phone regex) (#5469) (@f-liva)
- Recover reply context via XRPC on cache miss (closes #5452) (#5471) (@houko)
- Demote /api/metrics 401 from WARN to DEBUG (#5482) (@houko)
- Repair 3 pre-existing main CI breakers inherited by all open PRs (#5486) (@houko)
- Ack duplicate `/approve <id>` instead of error-shaped not-found (#5487) (@houko)
- Wake idle agent after approval resolve so the chat gets the result (#5488) (@houko)
- Suppress redundant /approve|/reject ack on inline-keyboard tap (#5490) (@houko)
- Route agent reply through channel after wake — fixes "tap Approve → silence" (#5491) (@houko)
- Cargo fmt + regenerate sdk/ to repair main CI (Quality, OpenAPI Drift) (#5494) (@houko)
- Log only email domain at INFO in OIDC auth_callback (#5504) (@houko)
- Sanitize reserved channel names at every SenderContext ingress (#5506) (@houko)
- Return path relative to home_dir, not absolute (#5509) (@houko)
- Keyboard nav for NotificationCenter (WAI-ARIA Menu Button) (#5510) (@houko)
- Record comms_send in hash-chained audit log (#5512) (@houko)
- Reject empty code in OAuth callback before token exchange (#5515) (@houko)
- SSRF-validate attachment URLs + DNS-rebind pin (#5517) (@houko)
- Cap bulk-handler Vec::with_capacity to prevent DoS pre-allocation (#5520) (@houko)
- Bound buckets map with hard cap + periodic sweep (#5522) (@houko)
- Never log raw IdP token-endpoint response bodies (#5526) (@houko)
- Detect partial-upgrade drift between migrations table and user_version pragma (#5528) (@houko)
- Never silently default or fabricate from corrupt JSON-in-TEXT columns (#5532) (@houko)
- Reclaim per-session bucket on session delete (#5534) (@houko)
- Bound RoundRobin cursor with cycle-aware iteration (#5536) (@houko)
- Restore main — rustfmt drift + 2 PR-only test failures (#5538) (@houko)
- Release prune lock across try_summarize_trim().await; CAS on messages_generation (#5541) (@houko)
- Validate provider name shape before deriving env var (#5542) (@houko)
- Bijective SHA-256 agent_id suffix to stop container-name collisions (#5545) (@houko)
- Hold ledger mutex across check + add (#5548) (@houko)
- Validate tool args at boundary before forwarding to MCP server (#5550) (@houko)
- Acquire per-agent semaphore in workflow send_message closure (#5554) (@houko)
- Cap system_prompt size and lock down create-handler invariants (#5558) (@houko)
- Allow zero spaces in attribution regex (#5560) (@houko)
- Swap RefCell for parking_lot::Mutex to remove async borrow-panic footgun (#5563) (@houko)
- Reject `..` per-segment in react_asset, not by substring (#5565) (@houko)
- Switch useSessionStream to authenticated WebSocket (#5567) (@houko)
- Make agent_concurrency_for entry construction atomic (#5569) (@houko)
- Hash session tokens at rest in sessions.json — backup-snapshot replay resistance (#5571) (@houko)
- #[serde(skip_serializing)] api_key + proxy_url (#5573) (@houko)
- Escape translator HTML, route via <Trans> (#5576) (@houko)
- Canonicalize + containment-check source/target_dir (#5577) (@houko)
- Warn at boot when declared provider API-key env vars are unset or empty (#5579) (@houko)
- Gate X-Forwarded-Proto on trusted_proxies for session cookie Secure flag (#5581) (@houko)
- Allowlist --network and --cap-add to prevent sandbox collapse (#5583) (@houko)
- Install-deps program allowlist + flag denylist + Owner-only role (#5588) (@houko)
- Warn on manifest swap when session_mode or max_concurrent_invocations changes (#5590) (@houko)
- Remove partial identity files on write failure (#5592) (@houko)
- Evict JWKS + discovery caches on external_auth hot-reload (#5594) (@houko)
- Fail-closed when guard-bash-safety lib is missing (#5596) (@houko)
- Rephrase strip_images placeholder so LLM does not deny image reception (#5597) (@DaBlitzStein)
- Wrap connect_mcp_servers spawns in spawn_supervised (#5599) (@houko)
- Hold Lane::Trigger permit across run_workflow spawn (#5602) (@houko)
- Derive deterministic SessionId for New-mode fires (#5604) (@houko)
- Align missed-fire log with single-catchup behaviour (#5606) (@houko)
- Classify refresh failures, single-flight refresh, drop unwrap (#5609) (@houko)
- Allow known framework source dirs, not just the librefang home (#5614) (@houko)
- Backfill missing #[utoipa::path] handlers + regenerate openapi.json (#5620) (@houko)
- API-surface hygiene — SPA route allowlist, registry id validation, auth/providers gating (#5638) (@houko)
- Non-IdP external_auth edits are a no-op, not a restart (#5646) (@houko)
- Propagate sender peer_id through remember_interaction_b… (#5647) (@Chukwuebuka-2003)
- Persist /sync since_token across restarts (#5651) (@neo-wanderer)
- External_auth IdP change is hot-reload, not restart (restore main) (#5652) (@houko)
- Clear clippy Quality lane (needless borrow, doc indentation, manual char comparison, await-holding-lock) (#5654) (@houko)
- Downgrade boot integrity-check failure to WARN (#5659) (@houko)
- Migrate legacy shared-namespace row on fallback hit (#5660) (@houko)
- Bound graceful shutdown so daemon.lock release isn't blocked by a hung phase (#5662) (@houko)
- Plug data leaks, restore lost state, harden parsing (#5674) (@leszek3737)
- Harden pre-commit + add detect-secrets CI workflow (#5681) (@houko)
- CommsKeys hierarchy + TerminalTabs storage helper + Modal autoFocus (#5682) (@houko)
- Soft-cap in-memory entries between trims at 1.5x max_in_memory_entries (#5683) (@houko)
- Harden build.rs git/date invocation; document pnpm audit ignores (#5684) (@houko)
- WARN when [agents.<name>.proactive_memory] appears in config.toml (real path is agent.toml) (#5687) (@houko)
- Filter /commands dispatch by account_id (multi-bot isolation) (#5688) (@houko)
- Widen exclusions, regenerate baseline, ignore generated_at drift (#5691) (@houko)
- Update audit_retention_test for #5683 soft-cap drain (#5693) (@houko)
- Strip line_number drift from detect-secrets baseline diff (#5695) (@houko)
- Log bot_token fingerprint instead of full token (fixes #5543) (#5700) (@houko)
- Replace removed `all-channels` feature with `telemetry` (#5702) (@houko)
- Add provider_budget_routes_test to detect-secrets baseline (#5707) (@houko)
- Regenerate SDKs for /api/budget/providers to repair main CI drift (#5709) (@houko)
- Include sdk/python/librefang in flake source filter (#5714) (@houko)

### Changed

- Unify error contracts — RFC + ToolError + first migration (#3576) (#5258) (@houko)
- Extract shared helpers + WS client + test fakes (#5335) (@houko)
- Return librefang-types IntegrationError from install_integration (stop leaking ExtensionResult) (#5622) (@houko)
- Return types-owned outcome from install_integration (stop leaking InstallResult) (#5644) (@houko)
- Widen ApiErrorResponse::internal_scrub sweep across routes (#5661) (@houko)

### Performance

- Use count_sessions() on status + snapshot (audit: list-sessions-decode-on-poll) (#5326) (@houko)
- Use list_arcs() in agent_budget_ranking (closes #5347) (#5348) (@houko)
- Evict stale tool-call timestamps on push (closes #5362) (#5363) (@houko)
- Rotate to next key on first RateLimit (closes #5373) (#5374) (@houko)
- Tx-wrap recall access bump + batched IN hydrate (closes #5375) (#5376) (@houko)
- Composite sessions(agent_id, updated_at) + audit_entries(agent_id, timestamp) indexes (#5399) (@houko)
- Stream extract_text_content into a single String to avoid per-save Vec<String> allocation (#5501) (@houko)
- Offload SQLite insert+prune via spawn_blocking, counter-gate prune (#5524) (@houko)
- Block_in_place for ImageFile reads (4 sites) (#5530) (@houko)
- Memoize dashboard_snapshot_inner with 900ms TTL cache (#5552) (@houko)
- Unblock axum executor on create_backup + persist_budget (spawn_blocking) (#5556) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Sidecar-first channel documentation (P6) (#5225) (@houko)
- Import audit backlog (120 tracking items) (#5240) (@houko)
- Fix stale telegram.rs reference in custom-channel example (#5248) (@houko)
- Fill in [[sidecar_channels]] samples for all 27 adapters (#5464) (@houko)
- Canonical config-reload field table derived from build_reload_plan (#5642) (@houko)

### Maintenance

- Restore rustfmt-clean main (Quality CI gate) (#5222) (@houko)
- Add Dockerfile.rust-dev with Tauri Linux GTK deps (#5233) (@houko)
- Cross-impl protocol conformance corpus + versioned spec (v1) (#5237) (@houko)
- Remove 6 low-value channel adapters (#5265) (@houko)
- Drop per-merge auto-update trigger from auto-update-branches (#5266) (@houko)
- Drop 12 unmaintained adapters (#5267) (@houko)
- Bump the cargo-minor-patch group with 8 updates (#5269) (@app/dependabot)
- Bump opentelemetry-otlp from 0.31.1 to 0.32.0 (#5270) (@app/dependabot)
- Bump russh-keys from 0.45.0 to 0.49.2 (#5271) (@app/dependabot)
- Bump shlex from 1.3.0 to 2.0.1 (#5272) (@app/dependabot)
- Bump tracing-opentelemetry from 0.32.1 to 0.33.0 (#5273) (@app/dependabot)
- Cargo fmt — fix rustfmt drift on main after channel-removal merges (#5274) (@houko)
- Bump Apple-Actions/upload-testflight-build from 5.1.0 to 5.2.1 in the actions-minor-patch group (#5304) (@app/dependabot)
- Pin silent_response markers against prompt-builder output (#5344) (@f-liva)
- Drop pulldown-cmark workspace dep, orphaned by matrix sidecar #5368 (#5407) (@houko)
- Pin SessionMode strict-variant deserialization (audit-disputed) (#5416) (@houko)
- Bump the web-minor-patch group in /web with 9 updates (#5438) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 12 updates (#5440) (@app/dependabot)
- 3 nits from post-merge audit (#5454) (@houko)
- Remove dead in-process channel scaffolding (#5461) (@houko)
- Delete dead per-channel REST endpoints + their helpers (#5463) (@houko)
- Rephrase docstring "stub" mentions to stop bot false positives (#5467) (@houko)
- Prune unused dependencies across the workspace (#5473) (@houko)
- Clean up sidecar migration tails (#5479) (@houko)
- Bump the docs-minor-patch group in /docs with 10 updates (#5493) (@app/dependabot)
- Skip Cloudflare Pages deploy on Dependabot PRs (#5495) (@houko)
- Run Coverage workflow on push:main only, not per-PR (#5496) (@houko)
- Make the per-PR test lane Linux-only (#5498) (@houko)
- Cover LIBREFANG_VAULT_KEY 32-ASCII-vs-32-bytes pitfall (#5611) (@houko)
- Replace fixed 150ms sleeps with condition-based polling (#5613) (@houko)
- Parallel semaphore-contention coverage for trigger concurrency caps (#5616) (@houko)
- Assert every KernelConfig field is reload-classified + backfill (#5619) (@houko)
- Replace unmaintained serde_yaml with serde_yaml_ng (RUSTSEC-2024-0320) (#5626) (@houko)
- Full-router semantic tests for lifecycle routes (suspend/resume/mode) (#5628) (@houko)
- Convert tools integration tests from mock to full router (#5630) (@houko)
- Convert load_test from mock to full router (exercise real middleware) (#5632) (@houko)
- Full-router semantic tests for files (path-traversal) + capabilities routes (#5634) (@houko)
- Convert agent_identity_registry tests from mock to full router (#5636) (@houko)
- Full-router semantic tests for clone/reload/push + bulk routes (#5640) (@houko)
- Delete 65 audit docs whose GitHub issue is closed (#5670) (@houko)
- Rename librefang-migrate → librefang-import + reconcile stale CLAUDE.md + justfile policy (#5668) (#5685) (@houko)

### Reverted

- Roll back v2026.5.25-beta.13 / beta.14 version bumps to 2026.5.17-beta.12 (#5717) (@houko)

### Other

- [Medium] Per-trigger `session_mode_override = New` is throttled by the manifest's `Persistent` clamp (#5624) (@houko)

</details>


