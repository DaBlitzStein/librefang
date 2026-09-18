import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  emptyManifestForm,
  ORPHAN_POLICIES,
  TOOL_EXEC_BACKENDS,
  TOOL_PROFILES,
} from "../agentManifest";

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
  channels: "readonly: channel membership is set from the channel's own page",
  metadata: "preserved: arbitrary key/value table, no widget",
  exec_policy:
    "partial: only the shorthand string form has a widget; the full table is preserved",
  tools:
    "missing: per-tool `[tools.<name>.params]` override table, no widget (distinct from capabilities.tools, which lists names)",
  is_hand: "readonly: derived from the manifest's origin",
  auto_dream_enabled: "elsewhere: toggled from Memory > Auto Dream",
  auto_evolve: "elsewhere: toggled from the drawer's Skills tab",
  channel_overrides: "missing: 29-key per-channel table, no widget",
  skill_workshop: "missing: no widget anywhere",
  context_engine: "missing: no widget anywhere",
  triggers:
    "elsewhere: the Schedule tab edits the runtime registry, not this manifest array",
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

// Enum-valued manifest fields have to be written in the form serde expects,
// and that form is not the Rust variant name.
//
// `ToolProfile` and `OrphanPolicy` are `#[serde(rename_all = "snake_case")]`
// and `BackendKind` is `rename_all = "lowercase"`, so the file holds `full`,
// `keep`, `local` — not `Full`, `Keep`, `Local`. Writing a variant name
// produces a manifest the daemon cannot deserialise, and because every one of
// these fields is `#[serde(default)]` the failure is a value that quietly
// reverts rather than an error anyone sees.
//
// Reading the Rust here rather than restating it: the lists in
// `agentManifest.ts` are a copy of something upstream owns, and a copy is
// exactly what goes stale when a variant is added or a rename is changed.
// Built from the imports rather than by naming them through the module, so a
// renamed export is a compile error here instead of an `undefined` the test
// would compare against and pass.
const MANIFEST_ENUM_CONSTANTS: Record<string, readonly string[]> = {
  TOOL_PROFILES,
  ORPHAN_POLICIES,
  TOOL_EXEC_BACKENDS,
};

const RUST_ENUMS: Array<{ file: string; enum: string; exported: string }> = [
  { file: "agent.rs", enum: "ToolProfile", exported: "TOOL_PROFILES" },
  { file: "agent.rs", enum: "OrphanPolicy", exported: "ORPHAN_POLICIES" },
  { file: "tool_exec.rs", enum: "BackendKind", exported: "TOOL_EXEC_BACKENDS" },
];

/** `rename_all` → the function that turns `CamelCase` into the serialised form. */
function applyRenameAll(variant: string, rule: string): string {
  if (rule === "lowercase") return variant.toLowerCase();
  if (rule === "snake_case") {
    return variant
      .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
      .replace(/([A-Z])([A-Z][a-z])/g, "$1_$2")
      .toLowerCase();
  }
  return variant;
}

function rustEnum(file: string, name: string): { variants: string[]; renameAll: string } {
  const source = readFileSync(
    join(__dirname, "..", "..", "..", "..", "..", "librefang-types", "src", file),
    "utf8",
  );
  const m = new RegExp(
    `((?:\\s*#\\[[^\\n]*\\]\\n)*)\\s*pub enum ${name}\\b[^{]*\\{([^}]*)\\}`,
  ).exec(source);
  if (!m) throw new Error(`enum ${name} not found in ${file}`);
  const rename = /rename_all\s*=\s*"([^"]+)"/.exec(m[1]);
  const variants = [...m[2].matchAll(/^\s*(?:#\[[^\]]*\]\s*)?([A-Z]\w*)/gm)].map((v) => v[1]);
  return { variants, renameAll: rename ? rename[1] : "none" };
}

describe("manifest enum spellings match the Rust rename rules", () => {
  it.each(RUST_ENUMS)("$exported lists exactly the serialised $enum", ({ file, enum: name, exported }) => {
    const { variants, renameAll } = rustEnum(file, name);
    expect(variants.length).toBeGreaterThan(0);

    const expected = variants.map((v) => applyRenameAll(v, renameAll));
    // `MODULE` exports are `as const` arrays; compare as plain arrays.
    const actual = [...(MANIFEST_ENUM_CONSTANTS[exported] as readonly string[])];

    expect(
      actual,
      `${exported} in agentManifest.ts and the serialised form of ${name} have ` +
        `diverged. A value the form writes must be one the daemon can read: the ` +
        `serialised spellings are ${JSON.stringify(expected)} (rename_all = ` +
        `"${renameAll}").\n\nExpected: ${JSON.stringify(expected)}\nActual:   ` +
        JSON.stringify(actual),
    ).toEqual(expected);
  });
});
