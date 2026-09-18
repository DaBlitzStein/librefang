// Tests SystemPromptSection / DescriptionSection / ChannelsSection directly
// — AgentsPage has no render harness (~20 hooks).

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  canEditAgentIdentity,
  cloneResultNotice,
  createDrawerSeed,
  hasTokenFootprintData,
  SystemPromptSection,
  DescriptionSection,
  ChannelsSection,
  TAB_SECTIONS,
  tabForFirstInvalidField,
} from "./AgentsPage";
import { MANIFEST_SECTION_IDS } from "../components/AgentManifestForm";
import { emptyManifestForm, validateManifestForm } from "../lib/agentManifest";
import { usePatchAgent, useSetAgentChannels } from "../lib/mutations/agents";
import { useBindPromptVersionToAgent } from "../lib/mutations/prompts";
import { usePromptVersions, useAgentChannels } from "../lib/queries/agents";

vi.mock("../lib/mutations/agents", () => ({
  usePatchAgent: vi.fn(),
  useSetAgentChannels: vi.fn(),
}));

vi.mock("../lib/mutations/prompts", () => ({
  useBindPromptVersionToAgent: vi.fn(),
}));

vi.mock("../lib/queries/agents", () => ({
  usePromptVersions: vi.fn(),
  useAgentChannels: vi.fn(),
}));

const addToastMock = vi.fn();
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: { addToast: typeof addToastMock }) => unknown) =>
    selector({ addToast: addToastMock }),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: unknown) =>
      opts && typeof opts === "object" && "defaultValue" in (opts as Record<string, unknown>)
        ? (opts as { defaultValue: string }).defaultValue
        : key,
    i18n: { language: "en" },
  }),
}));

const usePatchAgentMock = usePatchAgent as unknown as ReturnType<typeof vi.fn>;
const useBindMock = useBindPromptVersionToAgent as unknown as ReturnType<typeof vi.fn>;
const usePromptVersionsMock = usePromptVersions as unknown as ReturnType<typeof vi.fn>;
const useAgentChannelsMock = useAgentChannels as unknown as ReturnType<typeof vi.fn>;
const useSetAgentChannelsMock = useSetAgentChannels as unknown as ReturnType<typeof vi.fn>;

// The daemon gates both appearance writes at `role >= UserRole::Admin`, so the
// floor is the whole content of this predicate: a `user` shown these controls
// collects a 403, and an `admin` denied them is blocked from work the daemon
// would accept.
describe("canEditAgentIdentity", () => {
  it("lets an admin and an owner through", () => {
    expect(canEditAgentIdentity("admin")).toBe(true);
    expect(canEditAgentIdentity("owner")).toBe(true);
  });

  it("turns away the roles that could only collect a 403", () => {
    expect(canEditAgentIdentity("user")).toBe(false);
    expect(canEditAgentIdentity("viewer")).toBe(false);
  });

  // Silence is not permission. `whoami` has not answered on the first render,
  // and reading that as "yes" would flash controls that then disappear.
  it("treats an unanswered whoami as no", () => {
    expect(canEditAgentIdentity(undefined)).toBe(false);
    expect(canEditAgentIdentity("")).toBe(false);
  });
});

// The receiving half of the agent-types Run round trip. AgentsPage has no
// render harness, so this is the only thing pinning the mapping from the
// `template` search param to what the drawer opens on; the param name itself is
// the contract with the sender on /agent-types.
describe("createDrawerSeed", () => {
  it("opens the drawer on the template tab with the named type", () => {
    expect(createDrawerSeed("researcher")).toEqual({
      createMode: "template",
      templateName: "researcher",
    });
  });

  it("opens nothing when the param is absent", () => {
    expect(createDrawerSeed(undefined)).toBeNull();
  });

  // No agent type can be named "", and admitting it would open the drawer on an
  // empty picker with Create disabled and no way back.
  it("opens nothing for an empty value", () => {
    expect(createDrawerSeed("")).toBeNull();
  });
});

describe("cloneResultNotice", () => {
  const base = { agent_id: "agent-copy", name: "copy" };

  it("keeps complete clones on the success path", () => {
    expect(cloneResultNotice({ ...base, partial: false, warnings: [] })).toEqual({
      partial: false,
      warnings: "unknown",
    });
  });

  it("preserves stable warning codes for partial clones", () => {
    expect(cloneResultNotice({
      ...base,
      partial: true,
      warnings: ["identity_files_copy_failed", "registry_identity_copy_failed"],
    })).toEqual({
      partial: true,
      warnings: "identity_files_copy_failed, registry_identity_copy_failed",
    });
  });

  it("fails safe when warnings and the partial flag disagree", () => {
    expect(cloneResultNotice({
      ...base,
      partial: false,
      warnings: ["destination_workspace_missing"],
    }).partial).toBe(true);
  });
});

describe("hasTokenFootprintData", () => {
  it("treats a genuine zero footprint as data (tools-disabled agent, no system_prompt)", () => {
    expect(hasTokenFootprintData(0)).toBe(true);
  });

  it("treats a non-zero footprint as data", () => {
    expect(hasTokenFootprintData(1200)).toBe(true);
  });

  it("treats a missing field as no data", () => {
    expect(hasTokenFootprintData(null)).toBe(false);
    expect(hasTokenFootprintData(undefined)).toBe(false);
  });
});

function renderSection(prompt: string) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={qc}>
      <SystemPromptSection agentId="agent-1" prompt={prompt} />
    </QueryClientProvider>,
  );
}

describe("SystemPromptSection (#6187)", () => {
  let patchMutate: ReturnType<typeof vi.fn>;
  let bindMutate: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    patchMutate = vi.fn();
    bindMutate = vi.fn();
    usePatchAgentMock.mockReturnValue({ mutate: patchMutate, isPending: false });
    useBindMock.mockReturnValue({ mutate: bindMutate, isPending: false });
    usePromptVersionsMock.mockReturnValue({ data: [], isLoading: false, isError: false });
  });

  it("Save is disabled until the prompt is edited", () => {
    renderSection("original prompt");
    const save = screen.getByRole("button", { name: /common\.save/i });
    expect(save).toBeDisabled();
  });

  it("editing the prompt and saving PATCHes system_prompt", () => {
    renderSection("original prompt");
    const textarea = screen.getByRole("textbox");
    expect(textarea).toHaveValue("original prompt");
    fireEvent.change(textarea, { target: { value: "updated prompt" } });
    const save = screen.getByRole("button", { name: /common\.save/i });
    expect(save).not.toBeDisabled();
    fireEvent.click(save);
    expect(patchMutate).toHaveBeenCalledTimes(1);
    expect(patchMutate.mock.calls[0][0]).toEqual({
      agentId: "agent-1",
      body: { system_prompt: "updated prompt" },
    });
  });

  it("binding a library version calls useBindPromptVersionToAgent with the version", () => {
    const version = {
      id: "ver-1",
      agent_id: "agent-1",
      version: 3,
      content_hash: "abc",
      system_prompt: "library prompt",
      tools: [],
      variables: [],
      created_at: "2025-01-01T00:00:00Z",
      created_by: "tester",
      is_active: false,
    };
    usePromptVersionsMock.mockReturnValue({
      data: [version],
      isLoading: false,
      isError: false,
    });
    renderSection("original prompt");
    fireEvent.click(screen.getByRole("button", { name: /Bind from library/i }));
    fireEvent.click(screen.getByRole("button", { name: /^Bind$/i }));
    expect(bindMutate).toHaveBeenCalledTimes(1);
    expect(bindMutate.mock.calls[0][0]).toEqual({
      agentId: "agent-1",
      version,
      previousSystemPrompt: "original prompt",
    });
  });
});

function renderDescription(description: string) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={qc}>
      <DescriptionSection agentId="agent-1" description={description} />
    </QueryClientProvider>,
  );
}

describe("DescriptionSection (#7742)", () => {
  let patchMutate: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    patchMutate = vi.fn();
    usePatchAgentMock.mockReturnValue({ mutate: patchMutate, isPending: false });
  });

  it("Save is disabled until the description is edited", () => {
    renderDescription("original description");
    const save = screen.getByRole("button", { name: /common\.save/i });
    expect(save).toBeDisabled();
  });

  it("editing the description and saving PATCHes description", () => {
    renderDescription("original description");
    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "updated description" } });
    const save = screen.getByRole("button", { name: /common\.save/i });
    expect(save).not.toBeDisabled();
    fireEvent.click(save);
    expect(patchMutate).toHaveBeenCalledTimes(1);
    expect(patchMutate.mock.calls[0][0]).toEqual({
      agentId: "agent-1",
      body: { description: "updated description" },
    });
  });

  it("allows setting a description from empty (the field is no longer hidden when blank)", () => {
    renderDescription("");
    expect(screen.getByRole("textbox")).toHaveValue("");
  });
});

function renderChannels() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  render(
    <QueryClientProvider client={qc}>
      <ChannelsSection agentId="agent-1" />
    </QueryClientProvider>,
  );
}

describe("ChannelsSection (#7742)", () => {
  let setChannelsMutate: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    setChannelsMutate = vi.fn();
    useSetAgentChannelsMock.mockReturnValue({ mutate: setChannelsMutate, isPending: false });
    useAgentChannelsMock.mockReturnValue({
      data: { assigned: ["telegram"], available: ["telegram", "discord", "slack"], mode: "allowlist" },
      isLoading: false,
    });
  });

  it("renders the picker seeded with the assigned channels and no Save button while pristine", () => {
    renderChannels();
    // MultiSelectCmdk swaps its placeholder to "Add more…" once at least
    // one chip is selected (see MultiSelectCmdk.tsx), so with "telegram"
    // already assigned the combobox itself — not the custom placeholder —
    // is the stable query target.
    expect(screen.getByRole("combobox")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove telegram" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /common\.save/i })).not.toBeInTheDocument();
  });

  it("shows the 'no channels configured' message when the instance has none", () => {
    useAgentChannelsMock.mockReturnValue({
      data: { assigned: [], available: [], mode: "all" },
      isLoading: false,
    });
    renderChannels();
    expect(
      screen.getByText("No channels configured on this instance."),
    ).toBeInTheDocument();
  });

  it("still renders an allowlist whose channels are no longer configured on the instance (#7749 review)", async () => {
    // `get_agent_channels` builds `available` from `config.sidecar_channels`
    // alone, so an `agent.toml` carrying `channels = ["telegram"]` after that
    // sidecar channel was removed from `config.toml` reports a non-empty
    // `assigned` against an empty `available`. Gating the picker on
    // `available` hid a live restriction behind "No channels configured" and
    // left no way to clear it.
    const user = userEvent.setup();
    useAgentChannelsMock.mockReturnValue({
      data: { assigned: ["telegram"], available: [], mode: "allowlist" },
      isLoading: false,
    });
    renderChannels();

    expect(
      screen.queryByText("No channels configured on this instance."),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove telegram" })).toBeInTheDocument();

    // …and it is clearable from here, which is the half that mattered.
    await user.click(screen.getByRole("button", { name: "Remove telegram" }));
    fireEvent.click(screen.getByRole("button", { name: /common\.save/i }));
    expect(setChannelsMutate).toHaveBeenCalledTimes(1);
    expect(setChannelsMutate.mock.calls[0][0]).toEqual({ agentId: "agent-1", channels: [] });
  });

  it("picking a channel from the dropdown and saving PUTs the new allowlist (#7742)", async () => {
    const user = userEvent.setup();
    renderChannels();

    const input = screen.getByRole("combobox");
    await user.click(input);
    const list = await screen.findByRole("listbox");
    await user.click(within(list).getByText("discord"));

    const save = screen.getByRole("button", { name: /common\.save/i });
    fireEvent.click(save);

    expect(setChannelsMutate).toHaveBeenCalledTimes(1);
    expect(setChannelsMutate.mock.calls[0][0]).toEqual({
      agentId: "agent-1",
      channels: ["telegram", "discord"],
    });
  });
});

describe("agent drawer — manifest section layout", () => {
  const hosted = Object.entries(TAB_SECTIONS).flatMap(([tab, ids]) =>
    (ids ?? []).map((id) => ({ tab, id })),
  );

  // A section the form can render but no tab hosts is a field the operator
  // cannot reach — the same defect as a manifest key with no widget, one level
  // up, and invisible in every other test because the form would still render
  // it happily wherever it was asked to.
  it("hosts every manifest section somewhere", () => {
    const orphans = MANIFEST_SECTION_IDS.filter(
      (id) => !hosted.some((entry) => entry.id === id),
    );
    expect(
      orphans,
      `Sections exist in the editor that no tab renders, so they are ` +
        `unreachable from the drawer. Add each to TAB_SECTIONS.\n\n` +
        `Orphaned: ${orphans.join(", ")}`,
    ).toEqual([]);
  });

  // The other direction, and the one that was missing.
  //
  // Everything above asks whether each *real* section is hosted somewhere. None
  // of it asks whether each hosted id is real, so a tab naming a section the
  // editor does not implement passed every assertion while rendering a hole:
  // `sections={["skils"]}` finds no match in the form and draws nothing, and
  // the drawer looks like it loaded an empty tab rather than like it is wrong.
  //
  // The ids are a join between two files — the form implements them, the page
  // arranges them — and a join is exactly where a name has to be checked from
  // both sides.
  it("hosts no section the editor does not implement", () => {
    const known = new Set<string>(MANIFEST_SECTION_IDS);
    const imaginary = hosted.filter((entry) => !known.has(entry.id));

    expect(
      imaginary,
      `TAB_SECTIONS names sections the editor does not implement, so those ` +
        `tabs render nothing where a section should be. Check the id against ` +
        `MANIFEST_SECTION_IDS in AgentManifestForm.tsx.\n\n` +
        `Unknown (${imaginary.length}):\n` +
        imaginary.map((e) => `${e.id} (${e.tab})`).join("\n"),
    ).toEqual([]);
  });

  // A section hosted twice is the redundancy the single-surface design exists
  // to remove: two controls writing one field from two places, which is how the
  // complexity router ended up split between a tab and a drawer two levels
  // down.
  it("hosts no manifest section in two tabs", () => {
    const seen = new Map<string, string>();
    const clashes: string[] = [];
    for (const { tab, id } of hosted) {
      const previous = seen.get(id);
      if (previous) clashes.push(`${id} (${previous} and ${tab})`);
      else seen.set(id, tab);
    }
    expect(
      clashes,
      `A section rendered by two tabs means two controls for one field.\n\n` +
        `Duplicated: ${clashes.join(", ")}`,
    ).toEqual([]);
  });
});

// The tab jump on a failed save. With the sections split across tabs, the
// field that failed validation is usually on a tab the operator is not looking
// at — an error nobody can see is indistinguishable from no error, and this is
// the half of the layout change that keeps it visible.
//
// Driven by the real validator rather than by hand-written field paths: what
// matters is the pair (validator says X) -> (drawer goes there), and asserting
// on paths I chose myself would only confirm my idea of what the validator
// emits. Each case trips exactly one rule in an otherwise valid form.
describe("tabForFirstInvalidField", () => {
  const valid = () => {
    const form = emptyManifestForm();
    form.name = "an-agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    return form;
  };

  /** Trips one rule, asserts the validator agrees, and returns the tab. */
  function tabFor(mutate: (form: ReturnType<typeof valid>) => void) {
    const form = valid();
    mutate(form);
    const errors = validateManifestForm(form);
    expect(errors.length, `the fixture should trip exactly one rule`).toBeGreaterThan(0);
    return { errors, tab: tabForFirstInvalidField(errors) };
  }

  it.each([
    ["a missing name", (f: ReturnType<typeof valid>) => { f.name = ""; }, "conversation"],
    ["a blank cron", (f: ReturnType<typeof valid>) => { f.schedule = { mode: "periodic", cron: "" }; }, "schedule"],
    ["a zero check interval", (f: ReturnType<typeof valid>) => { f.schedule = { mode: "continuous", check_interval_secs: "0" }; }, "schedule"],
    ["an out-of-range temperature", (f: ReturnType<typeof valid>) => { f.model.temperature = "9"; }, "routing"],
    ["an out-of-range top_p", (f: ReturnType<typeof valid>) => { f.model.top_p = "7"; }, "routing"],
    ["an unparseable JSON schema", (f: ReturnType<typeof valid>) => {
      f.response_format = { mode: "json_schema", name: "s", schema: "{not json", strict: false };
    }, "conversation"],
    ["a shared folder with no path", (f: ReturnType<typeof valid>) => {
      f.workspaces = [{ _uid: "u1", name: "docs", path: "", mode: "rw" }];
    }, "conversation"],
  ])("sends %s to the tab that owns the field", (_label, mutate, expected) => {
    const { tab } = tabFor(mutate as (form: ReturnType<typeof valid>) => void);
    expect(tab).toBe(expected);
  });

  // The first message wins, because the operator is sent to exactly one tab and
  // the validator's order is the order the rules are written in.
  it("follows the first error when several are present", () => {
    const form = valid();
    form.name = "";
    form.model.temperature = "9";
    const errors = validateManifestForm(form);
    expect(errors.length).toBeGreaterThan(1);

    // `name` is checked before the model ranges, and identity lives on
    // Conversation while the model lives on Routing.
    expect(tabForFirstInvalidField(errors)).toBe("conversation");
  });

  it("returns nothing when there is nothing to report", () => {
    expect(tabForFirstInvalidField([])).toBeUndefined();
    expect(tabForFirstInvalidField(validateManifestForm(valid()))).toBeUndefined();
  });

  // A path no section claims is already a failure of the coverage guard in
  // AgentManifestForm.test.tsx; this pins the behaviour here so the caller's
  // `if (owningTab)` fallback is a no-op rather than a crash.
  it("returns nothing for a field path no section claims", () => {
    expect(tabForFirstInvalidField(["campo.inventado"])).toBeUndefined();
  });
});
