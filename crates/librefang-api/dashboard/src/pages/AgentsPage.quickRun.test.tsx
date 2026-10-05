import { beforeEach, describe, it, expect, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import React from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AgentsPage } from "./AgentsPage";
import { useDrawerStore } from "../lib/drawerStore";

/**
 * The receiving half of the agent-types Run round trip (#8385).
 *
 * `/agent-types`' Run control navigates to `/agents?template=<name>`; this is
 * the only harness where the seed effect runs against a real page — what the
 * create modal actually opens on, what an unknown name reports, and that
 * closing hands the URL back. The pure `resolveDrawerSeed` mapping is pinned in
 * `AgentsPage.test.tsx`.
 *
 * The create flow is the inline `<Modal>` (the drawer era is gone in the
 * unified editor), so the assertions below address the modal's own DOM rather
 * than a drawer slot.
 */

// `vi.mock` factories are hoisted above the module body, so the fixtures they
// close over have to be hoisted too.
const { AGENTS, navigateMock, routerSearch, templatesResult } = vi.hoisted(() => ({
  AGENTS: [
    { id: "agent-a", name: "Alpha", is_hand: false, state: "running" },
    { id: "agent-b", name: "Bravo", is_hand: false, state: "running" },
  ],
  navigateMock: vi.fn(),
  // Mutable so a case can arrive with `?template=` in the URL; the render
  // harness is the only place the seed effect runs under test.
  routerSearch: { current: {} as { template?: string } },
  templatesResult: {
    current: {
      data: undefined as unknown[] | undefined,
      isLoading: false,
      isPending: false,
      isError: false,
    },
  },
}));

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  motion: new Proxy(
    {},
    {
      get: (_target: unknown, prop: string) =>
        ({ children, ...rest }: { children?: React.ReactNode } & Record<string, unknown>) =>
          React.createElement(prop, rest, children),
    },
  ),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: "en" } }),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigateMock,
  useSearch: () => routerSearch.current,
  Link: ({ children, ...rest }: { children?: React.ReactNode } & Record<string, unknown>) =>
    React.createElement("a", rest, children),
}));

// The Agents list is projected from the overview snapshot, not from
// `useAgents` — the page calls `useDashboardSnapshot()` for it.
vi.mock("../lib/queries/overview", () => ({
  useDashboardSnapshot: () => ({ data: { agents: AGENTS }, isLoading: false }),
}));
vi.mock("../lib/queries/sessions", () => ({
  useSessionDetails: () => ({ data: undefined, isLoading: false }),
  useSessions: () => ({ data: [], isLoading: false }),
}));
vi.mock("../lib/queries/memory", () => ({
  useAgentKvMemory: () => ({ data: undefined, isLoading: false }),
  useMemorySearchOrList: () => ({ data: undefined, isLoading: false }),
}));
vi.mock("../lib/queries/providers", () => ({
  useProviders: () => ({ data: [], isLoading: false }),
}));
vi.mock("../lib/queries/models", () => ({
  useModels: () => ({ data: [], isLoading: false }),
}));
vi.mock("../lib/queries/skills", () => ({
  useSkills: () => ({ data: [], isLoading: false }),
}));
vi.mock("../lib/queries/mcp", () => ({
  useMcpServers: () => ({ data: { configured: [] }, isLoading: false }),
}));
vi.mock("../lib/queries/config", () => ({
  useModelRoutingInertReason: () => ({ data: null, isLoading: false }),
}));
vi.mock("../lib/queries/modelRouter", () => ({
  useModelRouterProfiles: () => ({ data: undefined, isLoading: false }),
}));
vi.mock("../lib/queries/authz", () => ({
  useWhoami: () => ({ data: { role: "owner" } }),
}));

vi.mock("../lib/queries/agents", () => ({
  agentQueries: {
    detail: (id: string) => ({
      queryKey: ["agents", "detail", id],
      queryFn: () => Promise.resolve(AGENTS.find((a) => a.id === id) ?? AGENTS[0]),
    }),
  },
  useAgents: () => ({ data: AGENTS, isLoading: false, isError: false }),
  useAgentEvents: () => ({ data: [], isLoading: false }),
  useAgentSessions: () => ({ data: [], isLoading: false }),
  useAgentStats: () => ({ data: undefined, isLoading: false }),
  useAgentTemplates: () => templatesResult.current,
  useAgentTools: () => ({ data: [], isLoading: false }),
  useAgentSkills: () => ({ data: [], isLoading: false }),
  useAgentMcpServers: () => ({ data: [], isLoading: false }),
  usePromptVersions: () => ({ data: [], isLoading: false }),
  useTools: () => ({ data: [], isLoading: false }),
  useAgentAvatarUrl: () => ({ data: undefined, isLoading: false }),
  useAgentChannels: () => ({ data: [], isLoading: false }),
  useAgentManifest: () => ({ data: undefined, isLoading: false }),
}));

vi.mock("../lib/mutations/agents", () => ({
  useSpawnAgent: () => ({ mutate: vi.fn(), reset: vi.fn(), isPending: false }),
  useCloneAgent: () => ({ mutate: vi.fn(), isPending: false }),
  useDeleteAgent: () => ({ mutate: vi.fn(), mutateAsync: vi.fn(), isPending: false }),
  usePatchAgent: () => ({ mutate: vi.fn(), isPending: false }),
  useResetAgentSession: () => ({ mutate: vi.fn(), mutateAsync: vi.fn(), isPending: false }),
  useResumeAgent: () => ({ mutate: vi.fn(), mutateAsync: vi.fn(), isPending: false }),
  useSuspendAgent: () => ({ mutate: vi.fn(), mutateAsync: vi.fn(), isPending: false }),
  useUpdateAgentTools: () => ({ mutate: vi.fn(), isPending: false }),
  useSetAgentSkills: () => ({ mutate: vi.fn(), isPending: false }),
  useAgentTemplateToml: () => ({ mutate: vi.fn(), isPending: false }),
  useSetAgentMcpServers: () => ({ mutate: vi.fn(), isPending: false }),
  useSetAgentChannels: () => ({ mutate: vi.fn(), isPending: false }),
  useDeleteAgentAvatar: () => ({ mutate: vi.fn(), mutateAsync: vi.fn(), isPending: false }),
  useUpdateAgentIdentity: () => ({ mutate: vi.fn(), isPending: false }),
  useUploadAgentAvatar: () => ({ mutate: vi.fn(), isPending: false }),
}));
vi.mock("../lib/mutations/prompts", () => ({
  useBindPromptVersionToAgent: () => ({ mutate: vi.fn(), isPending: false }),
}));

const addToastMock = vi.fn();
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: { addToast: typeof addToastMock }) => unknown) =>
    selector({ addToast: addToastMock }),
}));

function renderPage() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <AgentsPage />
      <DrawerSlot />
    </QueryClientProvider>,
  );
}

// The create surface is a `DrawerPanel`, which pushes its body into the store
// rather than rendering it inline — so the slot renders it alongside the page.
function DrawerSlot() {
  const content = useDrawerStore((s) => s.content);
  const isOpen = useDrawerStore((s) => s.isOpen);
  if (!isOpen || !content) return null;
  return <div data-testid="drawer-slot">{content.body}</div>;
}

beforeEach(() => {
  vi.clearAllMocks();
  useDrawerStore.setState({ isOpen: false, content: null });
  routerSearch.current = {};
  templatesResult.current = { data: [], isLoading: false, isPending: false, isError: false };
  // The page auto-selects the first agent only on a wide viewport (the effect
  // bails under the 1000px breakpoint). jsdom has no layout, so this is the
  // switch that decides whether the detail panel renders at all.
  (window.matchMedia("(min-width: 1000px)") as unknown as { matches: boolean }).matches = true;
});

describe("AgentsPage create drawer seed from ?template= (#8385)", () => {
  const RESEARCHER = {
    name: "researcher",
    description: "",
    provider: "",
    model: "",
    source: "user",
    editable: true,
  };

  function seedTemplates() {
    templatesResult.current = {
      data: [RESEARCHER],
      isLoading: false,
      isPending: false,
      isError: false,
    };
  }

  it("opens the create modal on the template the URL names", async () => {
    seedTemplates();
    routerSearch.current = { template: "researcher" };

    renderPage();

    // The custom-name field's placeholder is the selected template's name, so
    // it only renders once the modal is on the Template tab with the type set.
    const drawer = await screen.findByTestId("drawer-slot");
    expect(within(drawer).getByPlaceholderText("researcher")).toBeTruthy();
  });

  it("reports an unknown name and falls back to the blank form", async () => {
    seedTemplates();
    routerSearch.current = { template: "ghost" };

    renderPage();

    await waitFor(() =>
      expect(addToastMock).toHaveBeenCalledWith("agents.template_not_found", "error"),
    );
    // Template mode never rendered: no placeholder from a selected type.
    const drawer = await screen.findByTestId("drawer-slot");
    expect(within(drawer).queryByPlaceholderText("researcher")).toBeNull();
  });

  it("does not report a name as gone when the type list itself failed to load", async () => {
    templatesResult.current = {
      data: undefined,
      isLoading: false,
      isPending: false,
      isError: true,
    };
    routerSearch.current = { template: "researcher" };

    renderPage();

    // The modal stays shut rather than opening on a notice that blames the
    // type for the fetch that failed.
    await waitFor(() =>
      expect(addToastMock).not.toHaveBeenCalledWith("agents.template_not_found", "error"),
    );
    expect(screen.queryByTestId("drawer-slot")).toBeNull();
  });

  it("still seeds from cached types when a background refetch fails", async () => {
    // TanStack Query keeps the last successful `data` when a refetch fails and
    // only flips `status` to error; the cached list can still resolve the name,
    // so a failed refetch must not suppress the seed.
    templatesResult.current = {
      data: [RESEARCHER],
      isLoading: false,
      isPending: false,
      isError: true,
    };
    routerSearch.current = { template: "researcher" };

    renderPage();

    const drawer = await screen.findByTestId("drawer-slot");
    expect(within(drawer).getByPlaceholderText("researcher")).toBeTruthy();
    expect(addToastMock).not.toHaveBeenCalledWith("agents.template_not_found", "error");
  });

  it("drops the template param when the seeded modal closes", async () => {
    seedTemplates();
    routerSearch.current = { template: "researcher" };

    renderPage();

    const drawer = await screen.findByTestId("drawer-slot");
    expect(within(drawer).getByPlaceholderText("researcher")).toBeTruthy();
    fireEvent.click(within(drawer).getByRole("button", { name: "common.cancel" }));

    // Without this a second Run press on the same type navigates to a URL that
    // already matches and the seed effect never fires again.
    expect(navigateMock).toHaveBeenCalledWith({ to: "/agents", search: {}, replace: true });
  });
});
