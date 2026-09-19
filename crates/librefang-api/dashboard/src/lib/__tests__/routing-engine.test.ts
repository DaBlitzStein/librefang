import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { emptyManifestForm, MODEL_MODES, type ManifestFormState } from "../agentManifest";
import {
  applyRoutingEngine,
  ROUTING_ENGINES,
  ROUTING_ENGINE_SETTINGS,
  routingEngineOf,
} from "../routingEngine";

// The engine selector is a mapping from one operator choice onto three manifest
// fields, and every cell of that mapping is a claim about what the kernel does
// with those fields. The claims live in Rust source this test reads at test
// time, in the same spirit as `manifest-field-coverage.test.ts` reading
// `AgentManifest`: a hand-copied restatement of the rule goes stale the moment
// upstream renames a field or reorders a branch, and — worse than stale — it
// stays green while the form writes a state the kernel now resolves
// differently. Parsing the real code fails closed when any anchor moves.
//
// Four anchors, one per row of the table in `lib/routingEngine.ts`:
//
//   - `ModelMode`'s serialised spellings (agent.rs), which is what `[model]
//     mode = …` has to contain;
//   - `AgentRouterOverride::fixed` (model_profile.rs) and the `Bypassed` arm it
//     produces (model_router.rs), which is why the profile engine writes it
//     false and both other engines write it true;
//   - `AgentManifest::routing: Option<ModelRoutingConfig>` (agent.rs), whose
//     presence is what arms the tier router;
//   - the precedence in `model_selection_path` and the `Flexible` gate in
//     `route_to_profile` (agent_execution.rs), which is *why* each engine sets
//     the mode it does.

const CRATES = join(__dirname, "..", "..", "..", "..", "..");
const TYPES_SRC = join(CRATES, "librefang-types", "src");

const read = (...parts: string[]): string =>
  readFileSync(join(...parts), "utf8");

const AGENT_RS = read(TYPES_SRC, "agent.rs");
const MODEL_PROFILE_RS = read(TYPES_SRC, "model_profile.rs");
const KERNEL_SRC = join(CRATES, "librefang-kernel", "src");
const AGENT_EXECUTION_RS = read(KERNEL_SRC, "kernel", "agent_execution.rs");
const MODEL_ROUTER_RS = read(KERNEL_SRC, "model_router.rs");

/**
 * The serialised spellings of a Rust enum, honouring its `rename_all` — the
 * subset `manifest-field-coverage.test.ts` already applies to the enums the
 * form writes. Duplicated rather than shared because the two tests must fail
 * for their own reasons: this one is about the routing engines, and a helper
 * that drifts between them would be a third thing to keep right.
 */
function rustEnumSpellings(source: string, name: string): string[] {
  const m = new RegExp(`pub enum ${name}\\b[^{]*\\{([^}]*)\\}`).exec(source);
  if (!m) throw new Error(`enum ${name} not found`);
  const attrs = source.slice(Math.max(0, m.index - 400), m.index);
  const rename = /rename_all\s*=\s*"([^"]+)"/.exec(attrs);
  const rule = rename ? rename[1] : "none";
  return [...m[1].matchAll(/^\s*(?:#\[[^\]]*\]\s*)?([A-Z]\w*)/gm)]
    .map((v) => v[1])
    .map((variant) => {
      if (rule === "lowercase") return variant.toLowerCase();
      if (rule === "snake_case") {
        return variant
          .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
          .replace(/([A-Z])([A-Z][a-z])/g, "$1_$2")
          .toLowerCase();
      }
      return variant;
    });
}

/**
 * The body of `fn <name>`, by walking its braces.
 *
 * Walking rather than matching `\n}` because `route_to_profile` is a method
 * inside an `impl` block: its closing brace is indented, so a regex that looks
 * for a brace in column zero runs past the end of the function and keeps
 * reading the rest of the impl — which would let an assertion about the gate
 * be satisfied by some other function's text.
 */
function rustFunctionBody(source: string, name: string): string {
  const start = new RegExp(`fn ${name}\\b`).exec(source);
  if (!start) throw new Error(`fn ${name} not found`);
  const open = source.indexOf("{", start.index);
  let depth = 0;
  for (let i = open; i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}") {
      depth--;
      if (depth === 0) return source.slice(start.index, i + 1);
    }
  }
  throw new Error(`fn ${name} has no closing brace`);
}

describe("the routing engine mapping is anchored to the kernel's own rules", () => {
  it("spells `model.mode` the way `ModelMode` serialises, and covers every variant", () => {
    const spellings = rustEnumSpellings(AGENT_RS, "ModelMode");
    // A third variant would mean a routing state this table cannot name, so the
    // engine list has to be re-derived rather than silently under-described.
    expect(spellings, "ModelMode grew or lost a variant").toHaveLength(2);
    const [fixed, flexible] = spellings;

    expect(ROUTING_ENGINE_SETTINGS.fixed.mode).toBe(fixed);
    expect(ROUTING_ENGINE_SETTINGS.effort.mode).toBe(fixed);
    expect(ROUTING_ENGINE_SETTINGS.profile.mode).toBe(flexible);
    // The form's own copy of the spellings, which the serializer writes
    // verbatim — the two must not drift apart.
    expect([...MODEL_MODES]).toEqual(spellings);
  });

  it("keeps `router_fixed` on a real `AgentRouterOverride::fixed`, still bypassing the router", () => {
    // The field the form writes as `router_override = { fixed = true }`.
    expect(MODEL_PROFILE_RS).toMatch(
      /pub struct AgentRouterOverride \{[\s\S]*?pub fixed: bool,/,
    );
    // And the arm that gives it meaning: an agent whose override is fixed is
    // Bypassed *before* any profile is matched, which is what makes `true` the
    // right value for the two engines that must not be profile-routed, and
    // `false` the right value for the one that must.
    expect(MODEL_ROUTER_RS).toMatch(
      /if !config\.enabled \|\| agent_override\.is_some_and\(\|o\| o\.fixed\)/,
    );

    expect(ROUTING_ENGINE_SETTINGS.fixed.router_fixed).toBe(true);
    expect(ROUTING_ENGINE_SETTINGS.effort.router_fixed).toBe(true);
    expect(ROUTING_ENGINE_SETTINGS.profile.router_fixed).toBe(false);
  });

  it("arms the tier router by the presence of `AgentManifest::routing`", () => {
    expect(AGENT_RS).toMatch(/pub routing: Option<ModelRoutingConfig>,/);

    // Exactly one engine writes `[routing]`: under the profile engine a turn
    // that matched no profile would otherwise fall through into the tiers.
    expect(ROUTING_ENGINE_SETTINGS.fixed.routing_enabled).toBe(false);
    expect(ROUTING_ENGINE_SETTINGS.effort.routing_enabled).toBe(true);
    expect(ROUTING_ENGINE_SETTINGS.profile.routing_enabled).toBe(false);
  });

  it("gates profile routing on the flexible mode the profile engine writes", () => {
    const body = rustFunctionBody(AGENT_EXECUTION_RS, "route_to_profile");
    // The reader stops at this function's own closing brace: the next item in
    // the impl must not be part of what these assertions read.
    expect(body).not.toContain("fn execute_llm_agent");
    expect(
      body,
      "route_to_profile no longer declines a non-flexible manifest, so the " +
        "profile engine's mode is no longer what decides between the two routers.",
    ).toMatch(/manifest\.model\.mode != ModelMode::Flexible/);
    expect(body).toMatch(/!cfg\.model_router\.enabled/);
  });

  it("resolves profile before tiers, in that order", () => {
    const body = rustFunctionBody(AGENT_EXECUTION_RS, "model_selection_path");
    const profile = body.indexOf("ModelSelectionPath::Profile");
    const tier = body.indexOf("ModelSelectionPath::Tier");
    expect(profile).toBeGreaterThan(-1);
    expect(tier).toBeGreaterThan(-1);
    expect(
      profile < tier,
      "The profile router is no longer resolved before the tier router. Every " +
        "cell of the engine table assumes it wins when it applies: the effort " +
        "engine relies on being ineligible for it, not on coming first.",
    ).toBe(true);

    // The call site passes the two candidates positionally, so their order is
    // part of the contract too.
    expect(AGENT_EXECUTION_RS).toMatch(
      /model_selection_path\(\s*is_stable,\s*routed_profile\.is_some\(\),\s*tier_routing_config\.is_some\(\),?\s*\)/,
    );
  });
});

// The guards above are only worth what their readers are worth: a reader that
// returned a happy-path constant would keep every anchor green no matter what
// upstream did. These two prove the readers act on the text handed to them.
describe("the Rust readers read the source they are given", () => {
  it("applies the enum's own rename rule to whatever variants are there", () => {
    const synthetic = `
#[derive(Debug)]
#[serde(rename_all = "lowercase")]
pub enum ModelMode {
    /// A doc comment naming ModelSelectionPath::Profile, which is not a variant.
    #[default]
    Fixed,
    Flexible,
}
`;
    expect(rustEnumSpellings(synthetic, "ModelMode")).toEqual(["fixed", "flexible"]);
    expect(rustEnumSpellings(synthetic.replace("Flexible", "Adaptive"), "ModelMode")).toEqual([
      "fixed",
      "adaptive",
    ]);
  });

  it("stops a function body at that function's closing brace", () => {
    const synthetic = `
pub(crate) fn model_selection_path() -> X {
    if a {
        ModelSelectionPath::Profile
    }
}

fn execute_llm_agent() {
    ModelSelectionPath::Tier
}
`;
    const body = rustFunctionBody(synthetic, "model_selection_path");
    expect(body).toContain("ModelSelectionPath::Profile");
    expect(body).not.toContain("ModelSelectionPath::Tier");
  });
});

describe("routingEngineOf classifies a manifest the way the kernel resolves it", () => {
  const state = (
    mode: ManifestFormState["model"]["mode"],
    router_fixed: boolean,
    routing_enabled: boolean,
  ): ManifestFormState => {
    const form = emptyManifestForm();
    form.model = { ...form.model, mode, router_fixed };
    form.routing = { ...form.routing, enabled: routing_enabled };
    return form;
  };

  // Every combination of the three fields, and the engine the kernel resolves
  // it to. `flexible` with the override on is not profile routing — the
  // `Bypassed` arm above stops it before a profile is even matched — so it
  // classifies by what is left.
  const CASES: Array<[ManifestFormState["model"]["mode"], boolean, boolean, string]> = [
    ["fixed", false, false, "fixed"],
    ["fixed", false, true, "effort"],
    ["fixed", true, false, "fixed"],
    ["fixed", true, true, "effort"],
    ["flexible", false, false, "profile"],
    ["flexible", false, true, "profile"],
    ["flexible", true, false, "fixed"],
    ["flexible", true, true, "effort"],
  ];

  it.each(CASES)(
    "mode=%s router_fixed=%s routing.enabled=%s is the %s engine",
    (mode, router_fixed, routing_enabled, expected) => {
      expect(routingEngineOf(state(mode, router_fixed, routing_enabled))).toBe(expected);
    },
  );

  it("covers every combination of the three fields", () => {
    const combinations = new Set(
      CASES.map(([mode, fixed, routing]) => `${mode}|${fixed}|${routing}`),
    );
    expect(combinations.size).toBe(CASES.length);
    expect(CASES).toHaveLength(2 * 2 * 2);
  });

  it("never names an engine outside ROUTING_ENGINES", () => {
    for (const [mode, router_fixed, routing_enabled] of CASES) {
      expect(ROUTING_ENGINES).toContain(
        routingEngineOf(state(mode, router_fixed, routing_enabled)),
      );
    }
  });
});

describe("applyRoutingEngine writes the documented triple", () => {
  it.each([
    ["fixed", "fixed", true, false],
    ["effort", "fixed", true, true],
    ["profile", "flexible", false, false],
  ] as const)(
    "%s sets mode=%s, router_fixed=%s, routing.enabled=%s",
    (engine, mode, router_fixed, routing_enabled) => {
      const next = applyRoutingEngine(emptyManifestForm(), engine);
      expect(next.model.mode).toBe(mode);
      expect(next.model.router_fixed).toBe(router_fixed);
      expect(next.routing.enabled).toBe(routing_enabled);
    },
  );

  it.each([...ROUTING_ENGINES])("round-trips %s through the classifier", (engine) => {
    expect(routingEngineOf(applyRoutingEngine(emptyManifestForm(), engine))).toBe(engine);
  });

  it("leaves the other engine's settings alone, so switching back loses nothing", () => {
    const form = emptyManifestForm();
    form.routing = {
      ...form.routing,
      simple_model: "haiku",
      complex_model: "opus",
      complex_threshold: "900",
    };
    form.model = {
      ...form.model,
      router_allowed_profiles: ["coding"],
      router_cost_budget: "cheap",
      router_default_profile: "research",
    };

    const asProfile = applyRoutingEngine(form, "profile");
    expect(asProfile.routing.simple_model).toBe("haiku");
    expect(asProfile.model.router_allowed_profiles).toEqual(["coding"]);

    const backToEffort = applyRoutingEngine(asProfile, "effort");
    expect(backToEffort.routing.complex_model).toBe("opus");
    expect(backToEffort.routing.complex_threshold).toBe("900");
    expect(backToEffort.model.router_cost_budget).toBe("cheap");
    expect(backToEffort.model.router_default_profile).toBe("research");
  });

  it("does not touch the fields it does not own", () => {
    const form = emptyManifestForm();
    form.name = "router-test";
    form.reconcile_orphans = "keep";
    form.model.provider = "openai";

    const next = applyRoutingEngine(form, "profile");
    expect(next.name).toBe("router-test");
    expect(next.reconcile_orphans).toBe("keep");
    expect(next.model.provider).toBe("openai");
  });
});
