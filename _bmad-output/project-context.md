---
project_name: librefang
user_name: Blitz
date: 2026-05-24
sections_completed:
  - technology_stack
  - architecture_rules
  - rust_rules
  - api_route_rules
  - dashboard_rules
  - testing_rules
  - security_rules
  - performance_rules
  - federation_rules
  - governance_rules
  - git_workflow
existing_patterns_found: 42
---

# Project Context for AI Agents — LibreFang

_Critical rules and patterns that AI agents must follow when implementing
code. Focus on non-obvious details with motivations. Verified against
source code 2026-05-24._

---

## What This Is

LibreFang is a corporate Agent Operating System in Rust. Not a chatbot
framework. Not a clone of Hermes or OpenClaw. It provides:

- **Governance** for organizations that need real control over agents
- **Federation** with other agent systems (A2A, P2P via OFP protocol)
- **Security** at every layer (auth, RBAC, audit hash chains, vault)
- **Token efficiency** — maximum output per LLM dollar
- **Low memory footprint** — fast, lean agents
- **Controlled environment** — stricter than open-source alternatives

24 workspace crates + React/TanStack dashboard SPA.

## Technology Stack

### Backend
- Rust edition 2021, MSRV 1.94.1
- Tokio 1.x (full) — async runtime
- Axum 0.8 — HTTP/WebSocket server
- SQLite (bundled rusqlite) — persistence
- serde 1 + serde_json + toml — serialization
- thiserror 2.0 (libs) / anyhow 1 (app) — errors
- dashmap 6 + crossbeam 0.8 + arc-swap 1 — concurrency
- OpenTelemetry 0.31 + Prometheus — telemetry

### Frontend (dashboard SPA)
- React 19 + TypeScript 5.9 (strict mode)
- Vite 8 (Rolldown) + pnpm 10.33
- Tailwind CSS 4.2 (Vite plugin, no config file)
- TanStack Query 5 + Router 1.169 + Zustand 5
- Vitest 4.1 + Playwright 1.59
- i18next 26 + react-i18next 17

### External
- librefang-registry (GitHub: librefang/librefang-registry) — community
  registry of agents, hands, skills, plugins, providers, workflows, MCP
  servers. Daemon queries registry-index.json for marketplace/discovery.

---

## Architecture Rules

### KernelHandle = 14 role traits, not one god-trait
**Why (#3746):** Original was a 50+ method god-trait mixing 14 unrelated
domains. Split so callers express narrower bounds and test stubs catch
missing capabilities at compile time.

Import the prelude: `use librefang_runtime::kernel_handle::prelude::*`.
Without it, method calls fail with "no method found." Many traits have
default impls returning `Err("X not available")` — test stubs silently
fall back to runtime errors.

### Registry publication ordering
**Why (#3338):** Load-test race: `find_by_name` resolved a name to an ID
not yet published in the `agents` map.

`register()` acquires the `name_index.entry()` lock first (duplicate
check), then inserts into `agents`, then `tag_index`, then publishes
the name binding via `vacant.insert(id)` last. The binding is what
concurrent readers see. `remove()` mirrors: drop name binding first,
then retract entry. Both orderings are comment-documented but not
type-enforced.

### Deterministic prompt ordering
**Why (#3298):** HashMap iteration order varies across processes.
Non-deterministic prompt strings silently invalidate LLM prompt caches
even when content is unchanged.

Anything reaching an LLM prompt — tool definitions, MCP summaries,
skill registries, agent lists — MUST be sorted before stringification.
Two valid approaches: (1) BTreeMap/BTreeSet for inherent order, or
(2) HashMap + sort-at-boundary before emitting. The codebase uses both.
Regression tests verify byte-identical output across input orders.

### Snapshot isolation via Arc::make_mut
Registry stores `Arc<AgentEntry>`. Reads get cheap pointer clones.
Mutations use `Arc::make_mut` — readers holding old Arcs keep their
snapshot intact. No locks needed for read consistency.

### Session ID = UUID v5 (deterministic)
`AgentId::from_name(name)` uses a fixed namespace UUID. Same name =
same ID across restarts. Renaming produces a new ID and loses history.
Resolution: explicit override > per-trigger > channel > cron > manifest
default. Channel always wins over manifest `session_mode`.

---

## Rust Rules

### ErrorTranslator is !Send
**Why:** Wraps FluentBundle which uses `Rc` (intl-memoizer crate) → !Send.
Applies to async handlers only.

Must `drop(t)` before any `.await`. The compiler error says
"Handler<_, _> trait bound" — doesn't mention ErrorTranslator. Create
it only inside the error match arm, extract strings into owned
variables, then drop before awaiting.

### Config field checklist
Adding a KernelConfig field requires:
1. Struct field with `#[serde(default)]`
2. Entry in `Default` impl (missing = compile error)
3. `Serialize` + `Deserialize` derives
4. If using `#[serde(alias = "...")]`: add to `MANUAL_TOP_LEVEL_ALIASES`
   in `validation.rs` (schema doesn't surface aliases)

Unknown config fields are warnings, not errors — typos silently use
defaults. Allowlists are auto-derived from schemars JSON Schema since
PR #4298.

### Option<Arc<dyn Trait>> fields
Mark `#[serde(skip)]` AND implement Clone/Debug manually. Derive tries
to implement traits for skipped fields. Error says "dyn Trait: Serialize
not satisfied" — misleading.

### Error handling split
thiserror in library crates (structured enums), anyhow in app code.
API handlers pattern-match on error enum when possible; fall back to
substring matching on `.to_string()` when the enum is opaque.

### cargo test must be scoped
`cargo test -p <crate>` only. Enforced by `check-bash-rules.py` hook
(rule `cargo-test-unscoped`). Workspace-wide test runs cause lock
contention on shared `target/` directory: hangs, "could not acquire
lock", corrupted artifacts.

---

## API Route Rules

### Auth middleware allowlist
**Why:** Three lists allow fine-grained deployment control —
`require_auth_for_reads` toggles dashboard reads between public and
authenticated.

New unauthenticated endpoints must be added to `middleware.rs` — three
lists: `PUBLIC_ROUTES_ALWAYS` (any method), `PUBLIC_ROUTES_GET_ONLY`,
`PUBLIC_ROUTES_DASHBOARD_READS` (conditional). NOT by reordering routes
in server.rs. Missing = 401 in production, passes in tests.

### Route handler pattern
1. Extract `State(state)` + `lang: Option<Extension<RequestLanguage>>`
2. Validate input (errors NOT translated — internal parsing)
3. Call kernel (errors translated via ErrorTranslator)
4. Map kernel error variants to HTTP status + semantic error code
5. Error codes are semantic strings ("agent_already_exists"), not HTTP
   status names

### Agent workspace layout
**Why (#3230):** Multiple agents sharing a workspace collided on
identity files.

Identity files live in `{workspace}/.identity/`, not root.
`migrate_identity_files()` runs on every spawn and auto-moves root-level
files. Writing to root = file gets orphaned after migration.

---

## Dashboard Rules

### Data layer boundary
**Why:** Single point of change. Removing a symbol from `api.ts` breaks
at `client.ts` re-export, not across 30+ hook files.

All API access MUST go through hooks in `lib/queries/` and
`lib/mutations/`. No inline `fetch()` in pages/components. Hooks import
only from `lib/http/client.ts` (explicit whitelist, no `export *`).
Exceptions exist for blob downloads and one-shot probes — annotated
with `// lint-disable-next-line dashboard/no-inline-fetch`.

### Query key factories (keys.ts)
Every key uses `as const` (NOT cargo-cult — React Query 5 needs readonly
tuple inference for structural prefix matching). Every sub-key anchored
with `...fooKeys.all`. Hierarchy: `all / lists() / list(filters) /
details() / detail(id)`. Non-hierarchical keys break
`invalidateQueries` silently.

### Mutation invalidation
Each mutation with side-effects calls `invalidateQueries` with factory
keys in `onSuccess`. Wrap multi-view invalidations in `Promise.all()`.
Use `setQueryData` + `invalidateQueries` (belt-and-suspenders) when
the server returns the updated entity.

### Hand-role agent split
Hand-role agents use `usePatchHandAgentRuntimeConfig()`, NOT
`usePatchAgentConfig()`. Caller checks `agent.is_hand` from cached
detail and branches. Wrong hook = writes to wrong config slot + stale
hand detail view.

### Polling queries
`refetchIntervalInBackground: false` on every query with
`refetchInterval`. Omitting wastes API quota on hidden tabs.

### Enable guards
Every query with optional params: `enabled: !!param`. Without it,
fires request to `/api/foo/undefined` → cached 404.

### No backdrop-filter on shell containers
**Why (CSS W3C spec):** `backdrop-filter` makes the element a containing
block for fixed-positioned descendants, trapping dropdown menus.

### i18n
Include `defaultValue` on all non-core translation keys. Missing
translations render blank text without it.

### Lazy chunks
All page components use `lazyWithReload()` to detect stale chunks and
auto-reload once per session (10s guard prevents loops).

---

## Testing Rules

### Integration tests mandatory for route changes
**Why:** Catches missing server.rs registrations, un-deserialized config
fields, kernel-API type drift, and empty/null payloads.

Add `#[tokio::test]` against `TestServer` in `tests/*.rs`. Pattern:
`start_test_server()` → reqwest → assert status + response shape.
For write endpoints: follow up with a read to assert side effect.
Run: `cargo test -p librefang-api`.

### Dashboard tests
Spy before `mutate()`, use `waitFor()` for async assertions.

---

## Security Rules

### Symlink rejection
`symlink_metadata()` before reading user-injected file paths.
Context.md loaded into LLM prompt — symlink attack can exfiltrate
arbitrary files.

### Prompt injection sanitization
`sanitize_tool_label()`: non-alphanumeric → `_`.
`sanitize_sender_label()`: restricted to `[A-Za-z0-9._-]`.
Sanitize injection markers BEFORE truncating (two-stage pipeline).

### VAULT_KEY encoding
`LIBREFANG_VAULT_KEY` must base64-decode to exactly 32 bytes.
`openssl rand -base64 32` produces 44 chars. 32 ASCII chars ≠ 32 bytes.
`librefang doctor` validates this.

### A2A SSRF prevention
**Why (#3782):** Redirect to `169.254.169.254` (cloud metadata) bypassed
SSRF check on the original host.

Disable redirect following entirely. Re-check SSRF on every redirect
target. Pending A2A agents cannot receive tasks until explicitly
approved.

### Capability inheritance
Child agents cannot escalate beyond parent's grants. `*` stops at `/`
and `\\` separators. Bare `"*"` is universal match (backward compat).

### Force-human approval flag
`DeferredToolExecution.force_human = true` means approval CANNOT be
auto-skipped, even for trusted agents. Blocks RBAC M3 privilege
escalation via trusted agent proxy.

### TOTP lockout atomicity
`failure_rw_mutex` prevents TOCTOU races on lockout checks. Grace
period tracked per `sender_id`. Lockout after 5 failures with atomic
threshold verification.

### 4-layer tool gate
Layer 1: User denied/allowed tools (deny wins). Layer 2:
Channel-specific policy. Layer 3: Budget (RBAC M5). Layer 4: Global
approval policy. Glob matching: `shell_*` matches `shell_exec`.

---

## Performance Rules

### In-flight budget reservation
**Why:** Concurrent trigger fires observed pre-call total, all passed
the gate, overshooting caps N-fold.

Reserve budget BEFORE dispatching LLM calls via `MeteringReservation`
(`#[must_use = "must be settled or released"]`). Settle AFTER recording
actual usage. `pending_reserved_usd()` must be added to every budget
check. Drop impl releases unsettled reservations as safety net.

### Context budget windowing
Per-result cap = 30% of context window. Single result max = 50%.
Total headroom = 75%. Truncation: head (60%) + tail (40%) with
`[...truncated middle...]` marker. CJK ≈ 3 bytes/char vs ASCII ≈ 1.

### Context compression
**Why:** Thresholds >90% caused truncation mid-thought.

Triggers at 80% of context window (`threshold_ratio: 0.80`). Preserves
head (3 messages) + tail (10 messages). Iterates up to 3x. Compressor
is stateless — state tracked via injected `[CONTEXT COMPRESSION
SUMMARY]` message.

---

## Federation Rules

### OFP wire authentication
**Why:** Leaked shared_secret alone must not enable impersonation.

Two layers: HMAC-SHA256 (cluster gate) + Ed25519 (per-node identity).
TOFU pins pubkey to node_id. Subsequent handshakes with different
pubkey = rejected. First-contact peers without Ed25519 accepted via
TOFU grace period (legacy compat); once pinned, downgrade is rejected.

### Ephemeral X25519 KEX
Per-handshake keypair provides forward secrecy. Ephemeral secret
zeroizes on drop. `ephemeral_pubkey` is `Option<String>` on wire —
missing = fallback to legacy derivation.

---

## Governance Rules

### Trigger dispatch concurrency
**Why:** Unbounded `task_post` loops spawned unlimited tokio tasks.

3 layered caps: global Lane::Trigger semaphore (default 8), per-agent
semaphore (default 1), per-session mutex. Per-agent cap NOT invalidated
on manifest hot-reload — must kill+respawn agent. `persistent + cap > 1`
auto-clamped to 1 with WARN log.

### Approval escalation
Max 3 rounds. Each adds `extra_timeout_secs` (default 120). Three
variants: Deny (default), Skip, Escalate.

### Audit hash chain
**Why:** Compliance (SOC 2) tamper-evidence requirement.

Each entry contains SHA-256(contents + prev_hash). AuditAction enum
is append-only (renaming breaks all subsequent hashes). Hard cap:
10,000 in-memory entries with retention trim.

### Per-user budget isolation
User budgets are checked post-call (LLM already executed). "Exceeded"
= next call denied, not rollback. Zero limit = unlimited. Bob's spend
does NOT count against Alice.

---

## Git & Workflow

- Conventional commits: `feat:`, `fix:`, `docs:`, `refactor:`,
  `chore:`, `ci:`, `perf:`, `test:`
- Never include "generated by Claude" in commit messages
- Worktree-only development: `git worktree add /tmp/librefang-<feature>`
- pre-commit: `cargo fmt --check` + detect-secrets
- pre-push: `cargo clippy -- -D warnings`
- OpenAPI drift detection runs in CI only, not local hooks
