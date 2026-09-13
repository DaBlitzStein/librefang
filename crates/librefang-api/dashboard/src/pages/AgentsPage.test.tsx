// Tests SystemPromptSection / DescriptionSection / ChannelsSection directly,
// plus a full-page harness for the detail panel's tab strip.
// The page reads ~20 hooks, so the harness mocks each of them at the
// queries/mutations layer; only the History tab's own data path is left real.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AgentsPage, cloneResultNotice, hasTokenFootprintData, SystemPromptSection, DescriptionSection, ChannelsSection } from "./AgentsPage";
import { usePatchAgent, useSetAgentChannels } from "../lib/mutations/agents";
import { useBindPromptVersionToAgent } from "../lib/mutations/prompts";
import { usePromptVersions, useAgentChannels } from "../lib/queries/agents";
import { getAgentDetail, getAgentManifestHistory } from "../lib/http/client";
import { useDashboardSnapshot } from "../lib/queries/overview";

// Rendering the whole page (rather than a section) is what the History tab
// needs: the tab strip, the active-tab state and the `enabled` gate that
// makes the history request lazy only exist inside `AgentsPage` itself.
// Everything the page reads is mocked at the hook layer per the dashboard
// data-layer rule, except `useAgentManifestHistory` / `agentQueries`, which
// stay real so the laziness assertion runs through the actual query factory
// and only the HTTP boundary is stubbed.
const idleQuery = () => ({
  data: undefined,
  isLoading: false,
  isError: false,
  isFetching: false,
  isSuccess: false,
  error: null,
  refetch: vi.fn(),
});

vi.mock("../lib/mutations/agents", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/mutations/agents")>()),
  usePatchAgent: vi.fn(),
  useSetAgentChannels: vi.fn(),
}));

vi.mock("../lib/mutations/prompts", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/mutations/prompts")>()),
  useBindPromptVersionToAgent: vi.fn(),
}));

vi.mock("../lib/queries/agents", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/queries/agents")>()),
  usePromptVersions: vi.fn(),
  useAgentChannels: vi.fn(),
  useAgentEvents: vi.fn(() => idleQuery()),
  useAgentSessions: vi.fn(() => idleQuery()),
  useAgentStats: vi.fn(() => idleQuery()),
  useAgentTemplates: vi.fn(() => idleQuery()),
  useAgentTools: vi.fn(() => idleQuery()),
  useAgentSkills: vi.fn(() => idleQuery()),
  useAgentMcpServers: vi.fn(() => idleQuery()),
  useAgentManifest: vi.fn(() => idleQuery()),
  useTools: vi.fn(() => idleQuery()),
}));

vi.mock("../lib/http/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/http/client")>()),
  getAgentDetail: vi.fn(),
  getAgentManifestHistory: vi.fn(),
}));

vi.mock("../lib/queries/overview", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/queries/overview")>()),
  useDashboardSnapshot: vi.fn(() => idleQuery()),
}));

vi.mock("../lib/queries/sessions", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/queries/sessions")>()),
  useSessionDetails: vi.fn(() => idleQuery()),
}));

vi.mock("../lib/queries/memory", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/queries/memory")>()),
  useAgentKvMemory: vi.fn(() => idleQuery()),
}));

vi.mock("../lib/queries/providers", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/queries/providers")>()),
  useProviders: vi.fn(() => idleQuery()),
}));

vi.mock("../lib/queries/models", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/queries/models")>()),
  useModels: vi.fn(() => idleQuery()),
}));

vi.mock("../lib/queries/skills", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/queries/skills")>()),
  useSkills: vi.fn(() => idleQuery()),
}));

vi.mock("../lib/queries/mcp", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/queries/mcp")>()),
  useMcpServers: vi.fn(() => idleQuery()),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
}));

const addToastMock = vi.fn();
vi.mock("../lib/store", () => ({
  useUIStore: (
    selector: (s: {
      addToast: typeof addToastMock;
      hiddenModelKeys: string[];
    }) => unknown,
  ) => selector({ addToast: addToastMock, hiddenModelKeys: [] }),
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

// ---------------------------------------------------------------------------
// History tab — agent manifest version history (#8231).
//
// The tab shipped on `fork/feat/tui-manifest-history` and the branch is an
// ancestor of this one, but its `AgentsPage.tsx` hunk was dropped while a
// merge conflict was resolved: the hook, the query key factory and the route
// all survived, and only the caller vanished, so nothing failed to compile
// and the page silently rendered one tab fewer. These tests pin the three
// properties that regression removed.
const getAgentDetailMock = getAgentDetail as unknown as ReturnType<typeof vi.fn>;
const getAgentManifestHistoryMock = getAgentManifestHistory as unknown as ReturnType<typeof vi.fn>;
const useDashboardSnapshotMock = useDashboardSnapshot as unknown as ReturnType<typeof vi.fn>;

const HISTORY = [
  {
    id: 2,
    agent_id: "agent-1",
    agent_name: "alpha",
    timestamp: "2026-09-12 08:30:00",
    manifest_toml: 'name = "alpha"\nmodel = "claude-opus-4"\n',
    change_source: "api_patch",
  },
  {
    id: 1,
    agent_id: "agent-1",
    agent_name: "alpha",
    timestamp: "2026-09-11 07:15:00",
    manifest_toml: 'name = "alpha"\nmodel = "claude-sonnet-4"\n',
    change_source: "spawn",
  },
];

async function renderAgentsPageAndSelectAgent() {
  const user = userEvent.setup();
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0, gcTime: 0 } },
  });
  render(
    <QueryClientProvider client={qc}>
      <AgentsPage />
    </QueryClientProvider>,
  );
  // The tab strip only exists once an agent is selected, and selection is
  // what supplies the id the history request is keyed on.
  await user.click(await screen.findByRole("button", { name: /alpha/ }));
  await screen.findByRole("button", { name: /Conversation/ });
  return user;
}

describe("AgentsPage History tab (#8231)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    usePatchAgentMock.mockReturnValue({ mutate: vi.fn(), isPending: false });
    useBindMock.mockReturnValue({ mutate: vi.fn(), isPending: false });
    usePromptVersionsMock.mockReturnValue({ data: [], isLoading: false, isError: false });
    useAgentChannelsMock.mockReturnValue({ data: undefined, isLoading: false });
    useSetAgentChannelsMock.mockReturnValue({ mutate: vi.fn(), isPending: false });
    useDashboardSnapshotMock.mockReturnValue({
      data: {
        agents: [
          { id: "agent-1", name: "alpha", state: "running", is_hand: false },
        ],
      },
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    });
    getAgentDetailMock.mockResolvedValue({
      id: "agent-1",
      name: "alpha",
      state: "running",
    });
    getAgentManifestHistoryMock.mockResolvedValue(HISTORY);
  });

  it("offers a History tab in the detail panel", async () => {
    await renderAgentsPageAndSelectAgent();
    expect(screen.getByRole("button", { name: /History/ })).toBeInTheDocument();
  });

  it("does not request the manifest history while the History tab is inactive", async () => {
    const user = await renderAgentsPageAndSelectAgent();

    // Anchor on the tab existing first. "Never fetched" is also true of a
    // page that has no History tab at all, so without this the assertions
    // below would still pass against the very regression they guard.
    expect(screen.getByRole("button", { name: /History/ })).toBeInTheDocument();

    // Selecting the agent alone must not fetch it…
    expect(getAgentManifestHistoryMock).not.toHaveBeenCalled();

    // …and neither must visiting a sibling tab. Without the `enabled` gate
    // every drawer open would pay for a payload almost nobody reads.
    await user.click(screen.getByRole("button", { name: /Logs/ }));
    await screen.findByText(/events · tail/);
    expect(getAgentManifestHistoryMock).not.toHaveBeenCalled();
  });

  it("fetches the history and renders one entry per manifest version once activated", async () => {
    const user = await renderAgentsPageAndSelectAgent();

    await user.click(screen.getByRole("button", { name: /History/ }));

    await waitFor(() => expect(getAgentManifestHistoryMock).toHaveBeenCalledTimes(1));
    expect(getAgentManifestHistoryMock).toHaveBeenCalledWith("agent-1");

    // Header carries the count, and each version is its own expandable row
    // labelled with the change source and holding the stored TOML.
    expect(await screen.findByText(/Manifest history/)).toBeInTheDocument();
    expect(screen.getByText(/· api_patch/)).toBeInTheDocument();
    expect(screen.getByText(/· spawn/)).toBeInTheDocument();
    expect(screen.getByText(/claude-opus-4/)).toBeInTheDocument();
    expect(screen.getByText(/claude-sonnet-4/)).toBeInTheDocument();
  });

  it("surfaces the empty state when the agent has no recorded versions", async () => {
    getAgentManifestHistoryMock.mockResolvedValue([]);
    const user = await renderAgentsPageAndSelectAgent();

    await user.click(screen.getByRole("button", { name: /History/ }));

    expect(
      await screen.findByText("No config changes recorded yet."),
    ).toBeInTheDocument();
  });
});
