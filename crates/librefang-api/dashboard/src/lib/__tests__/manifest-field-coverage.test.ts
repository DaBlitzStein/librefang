import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { emptyManifestForm } from "../agentManifest";

// Requirement: "Ningún parámetro ausente — todo lo que un `agent.toml` admite
// debe ser configurable."
//
// The source of truth for what an `agent.toml` admits is the Rust struct
// `AgentManifest` in crates/librefang-types/src/agent.rs. The source of truth
// for what this editor can *write* is `ManifestFormState` in
// src/lib/agentManifest.ts. When those two drift, a field exists in the file
// format that no operator can reach from the dashboard — which is the bug this
// test exists to catch.
//
// Reading the struct at test time (rather than hardcoding a field list here)
// matters: a hand-copied list goes stale the moment upstream adds a field, and
// a stale list fails open. Parsing the real struct fails closed.
//
// This scans source at test time in the same spirit as
// `no-inline-fetch.test.ts` — the rule is enforced by the suite rather than by
// an extra build dependency.

const AGENT_RS = join(
  __dirname,
  "..",
  "..",
  "..",
  "..",
  "..",
  "librefang-types",
  "src",
  "agent.rs",
);

/**
 * Top-level `AgentManifest` keys with no editor surface, each with the reason
 * it is allowed to stay that way.
 *
 * An entry here is a *debt*, not an exemption: it says "this field is
 * reachable from the file format and not from the UI". The list is expected to
 * shrink as surfaces land. `readonly` entries are the only ones meant to be
 * permanent — provenance and runtime-owned values that an operator should not
 * be able to type into a form.
 */
const NO_SURFACE: Record<string, string> = {
  owner:
    "readonly: principal the agent acts for on unattended turns; no widget yet",
  source_template: "readonly: provenance written by the create flow",
  profile: "readonly: ToolProfile is displayed, not written, by the drawer",
  channels: "readonly: channel membership is set from the channel's own page",
  mcp_disabled: "readonly: folded into the MCP section's display computation",
  metadata: "preserved: arbitrary key/value table, no widget",
  exec_policy:
    "partial: only the shorthand string form has a widget; the full table is preserved",
  tools:
    "missing: per-tool `[tools.<name>.params]` override table, no widget (distinct from capabilities.tools, which lists names)",
  is_hand: "readonly: derived from the manifest's origin",
  auto_dream_enabled: "elsewhere: toggled from Memory > Auto Dream",
  auto_dream_min_hours: "missing: no widget anywhere",
  auto_dream_min_sessions: "missing: no widget anywhere",
  show_progress: "missing: no widget anywhere",
  auto_evolve: "elsewhere: toggled from the drawer's Skills tab",
  channel_overrides: "missing: 29-key per-channel table, no widget",
  max_history_messages: "missing: no widget anywhere",
  max_concurrent_invocations: "missing: no widget anywhere",
  cache_context: "missing: no widget anywhere",
  tool_exec_backend: "missing: no widget anywhere",
  skill_workshop: "missing: no widget anywhere",
  proactive_memory: "missing: per-agent override has no widget (the global key does)",
  compaction: "missing: no widget anywhere",
  context_engine: "missing: no widget anywhere",
  rl_export: "missing: no widget anywhere",
  triggers:
    "elsewhere: the Schedule tab edits the runtime registry, not this manifest array",
  reconcile_orphans: "missing: no widget anywhere",
  async_tasks: "missing: no widget anywhere",
};

/**
 * Pull the top-level `pub` field names out of `pub struct <name> { … }`,
 * honouring `#[serde(rename = "…")]` because the name that matters for a TOML
 * file is the serialized one.
 */
function structFieldKeys(source: string, structName: string): string[] {
  const start = source.indexOf(`pub struct ${structName} {`);
  if (start < 0) throw new Error(`struct ${structName} not found in agent.rs`);

  // Walk braces so a nested closure type in a field type can't end the block.
  let depth = 0;
  let end = start;
  for (let i = source.indexOf("{", start); i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}") {
      depth--;
      if (depth === 0) {
        end = i;
        break;
      }
    }
  }
  const body = source.slice(start, end);

  const keys: string[] = [];
  const lines = body.split("\n");
  let pending: string[] = [];
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith("#[")) {
      pending.push(trimmed);
      continue;
    }
    const field = /^pub (\w+)\s*:/.exec(trimmed);
    if (!field) continue;
    const rename = pending
      .map((a) => /rename\s*=\s*"([^"]+)"/.exec(a))
      .find((m): m is RegExpExecArray => m !== null);
    keys.push(rename ? rename[1] : field[1]);
    pending = [];
  }
  return keys;
}

describe("manifest field coverage", () => {
  it("every top-level AgentManifest key has an editor surface or a documented reason", () => {
    const declared = structFieldKeys(readFileSync(AGENT_RS, "utf8"), "AgentManifest");
    expect(declared.length).toBeGreaterThan(0);

    const formKeys = new Set(Object.keys(emptyManifestForm()));

    const unaccounted = declared.filter(
      (key) => !formKeys.has(key) && !(key in NO_SURFACE),
    );

    expect(
      unaccounted,
      `These AgentManifest keys are neither editable in ManifestFormState nor listed ` +
        `in NO_SURFACE with a reason. Add the surface, or add the key with the reason ` +
        `it has none.\n\nUnaccounted (${unaccounted.length}):\n${unaccounted.join("\n")}`,
    ).toEqual([]);
  });

  it("every NO_SURFACE entry names a real manifest key", () => {
    const declared = new Set(
      structFieldKeys(readFileSync(AGENT_RS, "utf8"), "AgentManifest"),
    );
    const stale = Object.keys(NO_SURFACE).filter((key) => !declared.has(key));

    expect(
      stale,
      `NO_SURFACE lists keys that AgentManifest no longer declares. A stale entry ` +
        `is a field that was renamed or removed upstream while the list kept ` +
        `claiming it had no surface.\n\nStale (${stale.length}):\n${stale.join("\n")}`,
    ).toEqual([]);
  });

  it("every NO_SURFACE entry carries a reason", () => {
    const unreasoned = Object.entries(NO_SURFACE)
      .filter(([, reason]) => reason.trim().length === 0)
      .map(([key]) => key);

    expect(unreasoned, "An exemption without a reason is just a hole.").toEqual([]);
  });
});
