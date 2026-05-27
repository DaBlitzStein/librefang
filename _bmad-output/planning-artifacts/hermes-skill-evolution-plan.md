# Skill Evolution — Hermes Parity Plan

**Date:** 2026-05-24  |  **Author:** Blitz  |  **Status:** Ready for implementation

## LibreFang Already Has (Hermes Doesn't)
- Heuristic Workshop Scanners (zero-LLM-cost regex detection)
- Approval Policy (Pending vs Auto with security scanning)
- Multi-tiered Gate System (Budget->Semaphore->Cooldown, non-destructive)
- Registry Freeze (Stable Mode)
- Prompt-Injection Scanning (SkillVerifier)
- Structured Evolution Metadata (.evolution.json with versions/hashes/changelog)

## 6 PRs in Priority Order

### PR-3 (P1): Enhanced Background Review Prompt
Files: `crates/librefang-kernel/src/kernel/tools_and_skills.rs`
Hermes hierarchy: UPDATE loaded -> UPDATE umbrella -> ADD support file -> CREATE new umbrella. Anti-patterns. User-preference embedding. Add `write_file` action.

### PR-1 (P2): Skill Usage Telemetry
Files: `crates/librefang-skills/src/evolution.rs`, `crates/librefang-runtime/src/tool_runner/skill.rs`
Add: last_used_at, last_viewed_at, last_patched_at, view_count.

### PR-2 (P3): Skill Lifecycle States
Depends: PR-1. Files: `crates/librefang-skills/src/{lib,evolution,registry}.rs`
SkillState: Active/Stale/Archived/Pinned. Auto-transitions (30d->stale, 90d->archived).

### PR-4 (P4): Curator System
Depends: PR-2. Files: NEW `crates/librefang-kernel/src/curator/`
Weekly consolidation. Only agent-created. Never hard-deletes.

### PR-5 (P5): Background Review Callback
Independent. SkillEvolutionCompleted event -> channel notification.

### PR-6 (P6): Dashboard Skill Evolution UI
Depends: PR-1/2/4. State badges, usage stats, pin toggle, curator panel.
