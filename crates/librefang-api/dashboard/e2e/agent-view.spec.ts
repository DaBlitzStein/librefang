// The agent view's two-tab reform, driven end to end.
//
// No daemon: Playwright serves the dashboard from vite and `page.route`
// fulfills every backend call, the same shape as `everyapi-connect.spec.ts`.
// What that buys here is the part the unit tests structurally cannot see —
// `AgentsPage` mounts some twenty hooks and has no render harness, so the tab
// bars, the brief and the group panels are only ever asserted against their
// maps in vitest. This renders them.
//
// Screenshots land in `e2e-screenshots/` (gitignored) so a reviewer can look at
// the same surface the assertions describe.

import { expect, test, type Page, type Route } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

const SHOTS = process.env.LIBREFANG_SHOT_DIR ?? "e2e-screenshots";

const AGENT_ID = "agent-1";

const AGENT = {
  id: AGENT_ID,
  name: "test-agent",
  state: "running",
  description: "an agent for the screenshot",
  model: { provider: "anthropic", model: "claude-sonnet-5" },
  system_prompt: "You are a test agent.",
  last_active: "2026-09-18T14:00:00Z",
  injected_footprint_tokens: 1234,
  capabilities: { tools: true, skills: ["alpha", "beta"] },
  skills: ["alpha", "beta"],
  mcp_servers: [],
  is_hand: false,
  identity: { emoji: "🤖", color: "#888888" },
};

const MANIFEST_TOML = `name = "test-agent"
description = "an agent for the screenshot"
version = "0.1.0"
tags = ["test"]

[model]
provider = "anthropic"
model = "claude-sonnet-5"

[limits]
max_tokens = 4096
`;

function json(route: Route, body: unknown, status = 200) {
  return route.fulfill({
    status,
    contentType: "application/json",
    body: JSON.stringify(body),
  });
}

/** Every backend call the agent page makes, fulfilled from fixtures. */
async function mockBackend(page: Page) {
  await page.route("**/api/**", async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname;

    if (path === "/api/auth/dashboard-check") return json(route, { mode: "none" });
    if (path === "/api/authz/whoami") return json(route, { role: "owner", name: "tester" });
    if (path === "/api/version") return json(route, { version: "test", hostname: "devbox" });

    if (path === "/api/dashboard/snapshot") {
      return json(route, {
        health: { status: "ok" },
        status: { agents: 1, sessions: 1 },
        agents: [AGENT],
        providers: [],
        channels: [],
        skillCount: 0,
        workflowCount: 0,
        webSearchAvailable: false,
      });
    }

    if (path === `/api/agents/${AGENT_ID}`) return json(route, AGENT);
    if (path === `/api/agents/${AGENT_ID}/stats`) {
      return json(route, {
        sessions_24h: 3,
        cost_24h: 0.42,
        p95_latency_ms: 1200,
        active_now: 1,
        samples: 12,
        prev: { sessions_24h: 2, cost_24h: 0.3, p95_latency_ms: 1500 },
      });
    }
    if (path === `/api/agents/${AGENT_ID}/events`) {
      return json(route, {
        events: [
          {
            timestamp: "2026-09-18T14:00:00Z",
            model: "claude-sonnet-5",
            provider: "anthropic",
            input_tokens: 120,
            output_tokens: 30,
            cost_usd: 0.0021,
            tool_calls: 1,
            latency_ms: 900,
          },
        ],
      });
    }
    if (path === `/api/agents/${AGENT_ID}/sessions`) return json(route, { sessions: [] });
    if (path === `/api/agents/${AGENT_ID}/manifest`) {
      return route.fulfill({
        status: 200,
        contentType: "text/plain; charset=utf-8",
        body: MANIFEST_TOML,
      });
    }
    if (path === `/api/agents/${AGENT_ID}/channels`) {
      return json(route, { assigned: ["telegram"], available: ["telegram", "discord"], mode: "allowlist" });
    }
    if (path === `/api/agents/${AGENT_ID}/avatar`) return json(route, {}, 404);

    // The list endpoints whose response is an object wrapper rather than a
    // bare array. Getting one of these wrong throws inside a `useMemo` and
    // takes the whole page to the error boundary — which is exactly how this
    // spec first failed, so they are spelled out rather than inferred.
    if (path === "/api/mcp/servers") {
      return json(route, { configured: [], connected: [], total_configured: 0, total_connected: 0 });
    }
    if (path === "/api/providers") return json(route, { providers: [], total: 0 });
    if (path === "/api/models") return json(route, { models: [], total: 0, available: 0 });
    if (path === "/api/skills") return json(route, { items: [] });
    if (path === "/api/model-router/profiles") return json(route, { enabled: false, profiles: [] });

    // Everything else: an empty list or an empty object, whichever shape the
    // caller expects. The page degrades to its empty states, which is fine —
    // none of them is what this spec is asserting about.
    if (path.startsWith("/api/agents/") || path.endsWith("s")) return json(route, []);
    return json(route, {});
  });
}

async function openAgent(page: Page) {
  // `base: "/dashboard/"` in vite.config.ts, so the route is not at the root.
  await page.goto("/dashboard/agents");
  await page.getByRole("button", { name: /test-agent/ }).first().click();
  await expect(page.getByRole("tab", { name: "logs & info" })).toBeVisible();
}

test.beforeEach(async ({ page }) => {
  mkdirSync(SHOTS, { recursive: true });
  await mockBackend(page);
});

test("the agent view opens on logs & info with the brief and its four sub-tabs", async ({ page }) => {
  await openAgent(page);

  // The brief: state, model, last activity and the footprint, all above the
  // sub-tab bar and all present without opening anything.
  await expect(page.getByText("claude-sonnet-5").first()).toBeVisible();
  await expect(page.getByText("Token footprint")).toBeVisible();
  await expect(page.getByText(/Last activity/)).toBeVisible();
  await expect(page.getByText("Live conversation")).toBeVisible();

  // Two main tabs plus the four sub-tabs.
  await expect(page.getByRole("tab", { name: "config" })).toBeVisible();
  // `exact` throughout: "Logs" is a prefix of "logs & info", and Playwright's
  // default substring match resolves both.
  for (const label of ["Logs", "Memory", "Prompts & experiments", "History"]) {
    await expect(page.getByRole("tab", { name: label, exact: true })).toBeVisible();
  }
  await expect(page.getByRole("tab")).toHaveCount(6);

  await page.screenshot({ path: join(SHOTS, "01-info-brief.png"), fullPage: true });
});

test("the token footprint folds open in place", async ({ page }) => {
  await openAgent(page);

  const summary = page.getByText("Token footprint");
  const details = summary.locator("xpath=ancestor::details");
  await expect(details).not.toHaveAttribute("open", "");

  await summary.click();
  await expect(details).toHaveAttribute("open", "");
  // The recent calls are inside the fold, not in a drawer.
  await expect(page.getByText("Recent calls")).toBeVisible();
  await page.screenshot({ path: join(SHOTS, "02-info-footprint-open.png"), fullPage: true });
});

test("config carries all eight groups and one save", async ({ page }) => {
  await openAgent(page);
  await page.getByRole("tab", { name: "config" }).click();

  for (const label of [
    "General",
    "Model & routing",
    "Tools & skills",
    "Memory",
    "Limits & cost",
    "Channels",
    "Planning",
    "Conversation",
  ]) {
    await expect(page.getByRole("tab", { name: label, exact: true })).toBeVisible();
  }
  // Two main tabs + eight groups.
  await expect(page.getByRole("tab")).toHaveCount(10);
  await expect(page.getByRole("checkbox", { name: "Advanced mode" })).not.toBeChecked();
  // Settle the main-tab underline transition before any shot, for the reason
  // the group loop gives.
  await expect(page.getByRole("tab", { name: "config" })).toHaveCSS(
    "border-bottom-color",
    "rgb(56, 189, 248)",
  );
  // No screenshot here: the group loop captures General as `group-01`, and
  // two files of the same frame make a reviewer compare a picture with itself.
});

test("advanced mode opens every folded group; the switch is off by default", async ({ page }) => {
  await openAgent(page);
  await page.getByRole("tab", { name: "config" }).click();

  const folded = page.locator("details[data-advanced]").first();
  await expect(folded).toBeVisible();
  const before = await page.locator("details[data-advanced][open]").count();

  await page.getByRole("checkbox", { name: "Advanced mode" }).check();
  const after = await page.locator("details[data-advanced][open]").count();
  expect(after).toBeGreaterThan(before);

  await page.screenshot({ path: join(SHOTS, "03-config-advanced.png"), fullPage: true });

  await page.getByRole("checkbox", { name: "Advanced mode" }).uncheck();
  expect(await page.locator("details[data-advanced][open]").count()).toBeLessThan(after);
});

// One screenshot per group. The surface is the deliverable and it is reviewed
// by eye, so the spec produces the pictures as a side effect of asserting that
// each group has content of its own.
test("every group renders and is captured", async ({ page }) => {
  await openAgent(page);
  await page.getByRole("tab", { name: "config" }).click();

  const groups = [
    "General",
    "Model & routing",
    "Tools & skills",
    "Memory",
    "Limits & cost",
    "Channels",
    "Planning",
    "Conversation",
  ];
  for (const [index, label] of groups.entries()) {
    const tab = page.getByRole("tab", { name: label, exact: true });
    await tab.click();
    // The highlight has to follow the click: a bar that stays on the previous
    // group while the panel below swaps is worse than no highlight at all, and
    // it is the one thing a screenshot cannot be trusted to report about
    // itself.
    await expect(tab).toHaveAttribute("aria-selected", "true");
    // …and it has to be *painted* before the screenshot. The pills carry
    // `transition-colors`, so shooting straight after the click catches the
    // background mid-fade: the classes are already right while the pixels are
    // still the unselected colour, and four of these eight frames were
    // captured that way. Asserting the computed colour is both the wait and a
    // stronger assertion than `aria-selected` — it pins what the operator
    // actually sees.
    await expect(tab).toHaveCSS("background-color", "rgb(56, 189, 248)");
    // A group with nothing in it is the defect the guard exists to catch; the
    // screenshot is only trustworthy if something is on screen.
    await expect(page.locator("[data-section]").first()).toBeVisible();
    await page.screenshot({
      path: join(SHOTS, `group-${String(index + 1).padStart(2, "0")}-${label.replace(/\W+/g, "-")}.png`),
      fullPage: true,
    });
  }
});

test("the general group hosts the identity sections and the model group the model", async ({ page }) => {
  await openAgent(page);
  await page.getByRole("tab", { name: "config" }).click();

  // General: identity (Basics) and the system prompt.
  await expect(page.locator('[data-section="identity"]')).toBeVisible();
  await expect(page.locator('[data-section="prompt"]')).toBeVisible();
  await expect(page.locator('[data-section="model"]')).toHaveCount(0);

  await page.getByRole("tab", { name: "Model & routing", exact: true }).click();
  await expect(page.locator('[data-section="model"]')).toBeVisible();
  await expect(page.locator('[data-section="identity"]')).toHaveCount(0);

  // The way back to the deployment default, which the drawer's model editor
  // used to own: a pinned agent has to be unpinnable without hand-editing
  // `agent.toml`.
  await expect(page.getByRole("button", { name: "Use global default" })).toBeVisible();
});

// The grants panels are the other half of "Tools & skills": live writes over
// their own endpoints, above the manifest sections for the same subject. They
// are also the piece with the least cover — removing both from the group left
// every other test in this file green, because a group that renders its
// manifest sections still shows a `[data-section]`.
test("the tools & skills group keeps its live grants panels", async ({ page }) => {
  await openAgent(page);
  await page.getByRole("tab", { name: "config" }).click();
  await page.getByRole("tab", { name: "Tools & skills", exact: true }).click();

  // One string from each panel, and nothing else on the agent view renders
  // either of them.
  await expect(page.getByText("Using all available skills")).toBeVisible();
  await expect(page.getByText("Using all available tools")).toBeVisible();
  // The auto-evolve switch is the skills panel's own write, not a manifest
  // field, so it goes with them.
  await expect(page.getByText(/Auto-evolve/)).toBeVisible();
});

test("the channels group hosts the live allowlist and the 29 overrides together", async ({ page }) => {
  await openAgent(page);
  await page.getByRole("tab", { name: "config" }).click();
  await page.getByRole("tab", { name: "Channels", exact: true }).click();

  // The picker (its own endpoint) and the manifest's own channel_overrides
  // section, one above the other.
  await expect(page.getByRole("button", { name: "Remove telegram" })).toBeVisible();
  await expect(page.locator('[data-section="channel_overrides"]')).toBeVisible();
});

// The four manifest tables that had no editor at all, driven the way an
// operator reaches them: the group tab, the section, the control.
//
// Pure vitest cover cannot say whether the control is *reachable* — a section
// hosted by no group, or a field the group's `shows()` filter excludes, passes
// every unit test in the suite and renders nowhere. These four journeys are
// what closes that gap, and each one captures the editor as it was filled in.
//
// `Section` and `CollapsibleSection` fold by default, so each journey turns on
// Advanced mode first: that is what opens every fold in one move, and it is
// the switch an operator looking for these fields would reach for too.
async function openGroup(page: Page, group: string) {
  await openAgent(page);
  await page.getByRole("tab", { name: "config" }).click();
  await page.getByRole("checkbox", { name: "Advanced mode" }).check();
  await page.getByRole("tab", { name: group, exact: true }).click();
}

test("general edits the metadata table", async ({ page }) => {
  await openGroup(page, "General");

  const section = page.locator('[data-section="metadata"]');
  await expect(section).toBeVisible();
  // The card is the field's only label here, so the rows carry their names
  // through `aria-label` — a bare grid of inputs announces nothing.
  await section.getByRole("button", { name: "Add metadata entry" }).click();
  await section.getByLabel("Key 1", { exact: true }).fill("owner");
  await section.getByLabel("Value 1", { exact: true }).fill("ops");

  await section.getByRole("button", { name: "Add metadata entry" }).click();
  await section.getByLabel("Key 2", { exact: true }).fill("pinned");
  await section.getByLabel("Type 2", { exact: true }).selectOption("boolean");
  // A boolean row cannot hold free text, so the value control is a select.
  await expect(section.getByLabel("Value 2", { exact: true })).toHaveJSProperty("tagName", "SELECT");
  await section.getByLabel("Value 2", { exact: true }).selectOption("true");

  await expect(section.getByLabel("Key 1", { exact: true })).toHaveValue("owner");
  await expect(section.getByLabel("Value 2", { exact: true })).toHaveValue("true");

  await page.screenshot({
    path: join(SHOTS, "10-editor-metadata.png"),
    fullPage: true,
  });
});

test("tools & skills edits a per-tool override", async ({ page }) => {
  await openGroup(page, "Tools & skills");

  const section = page.locator('[data-section="tools"]');
  await expect(section).toBeVisible();
  await section.getByRole("button", { name: "Add tool override" }).click();
  // `exact` because "Tool" is a substring of the remove button's name.
  await section.getByLabel("Tool", { exact: true }).fill("web_search");

  await section.getByRole("button", { name: "Add parameter" }).click();
  await section.getByLabel("Parameter 1", { exact: true }).fill("max_results");
  await section.getByLabel("Type 1", { exact: true }).selectOption("number");
  await section.getByLabel("Value 1", { exact: true }).fill("5");

  await expect(section.getByLabel("Parameter 1", { exact: true })).toHaveValue("max_results");
  await expect(section.getByLabel("Value 1", { exact: true })).toHaveValue("5");

  await page.screenshot({
    path: join(SHOTS, "11-editor-tools.png"),
    fullPage: true,
  });
});

test("the exec policy table completes the shorthand in lifecycle", async ({ page }) => {
  await openGroup(page, "General");

  const lifecycle = page.locator('[data-section="lifecycle"]');
  await expect(lifecycle).toBeVisible();
  // The mode select is the control that was already there; the table beside it
  // is what this change adds. Reached by its own name rather than by position:
  // the lifecycle card's first select is `session_mode`.
  const mode = lifecycle.getByLabel("Exec policy");
  await mode.selectOption("allowlist");

  // `[]` and the absent key are different policies, and only one of them is a
  // statement about the safe list — the note is how the operator says which.
  await expect(lifecycle.getByText("Using the daemon's built-in safe list.")).toBeVisible();
  await lifecycle.getByRole("button", { name: "Declare an empty list" }).click();
  await expect(
    lifecycle.getByText("Declared empty: nothing bypasses the allowlist."),
  ).toBeVisible();

  const safeBins = lifecycle.getByPlaceholder("cat, head, wc");
  await safeBins.fill("head");
  await safeBins.press("Enter");

  // Asserted on the chip rather than on the input: the placeholder is only
  // rendered while the list is empty, so adding the first entry removes the
  // thing the locator was built on.
  await expect(lifecycle.getByRole("button", { name: "remove head" })).toBeVisible();
  await expect(mode).toHaveValue("allowlist");

  await page.screenshot({
    path: join(SHOTS, "12-editor-exec-policy.png"),
    fullPage: true,
  });
});

test("memory edits the context engine", async ({ page }) => {
  await openGroup(page, "Memory");

  const section = page.locator('[data-section="context_engine"]');
  await expect(section).toBeVisible();
  // `engine` is a `String` on the Rust side, so the select is an offer rather
  // than a closed set: a name it does not know is appended and survives.
  await section.locator("select").first().selectOption("summary");
  // `exact` because the plugin stack's placeholder starts with the same word.
  await section
    .getByPlaceholder("qdrant-recall", { exact: true })
    .fill("qdrant-recall");

  // The nine hook paths fold behind Advanced, which the journey already
  // opened: fill one the way an operator wiring a plugin would.
  await section
    .getByPlaceholder("~/.librefang/plugins/hook.py")
    .first()
    .fill("~/.librefang/plugins/recall.py");

  // A registry row, then the switch that says the list is deliberate.
  await section.getByRole("button", { name: "Add registry" }).click();
  await section.getByLabel("Registry name 1").fill("Mine");
  await section.getByLabel("GitHub repository 1").fill("acme/librefang-plugins");

  await expect(section.locator("select").first()).toHaveValue("summary");
  await expect(section.getByLabel("GitHub repository 1")).toHaveValue(
    "acme/librefang-plugins",
  );

  await page.screenshot({
    path: join(SHOTS, "13-editor-context-engine.png"),
    fullPage: true,
  });

  // The knobs sit below the fold, and `fullPage` cannot reach them: the app
  // scrolls an inner container, so the document is exactly one viewport tall
  // and the option is a no-op. Scrolling to the last block of the section is
  // what puts the grouped controls — runtime, limits, caching, the sandbox
  // switches, the circuit breaker, the env schema — on screen.
  const knobs = section.getByText("Environment variables");
  await knobs.scrollIntoViewIfNeeded();
  await expect(knobs).toBeVisible();
  await page.screenshot({
    path: join(SHOTS, "13b-editor-context-engine-knobs.png"),
  });
});
