import { StrictMode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { useUIStore } from "./lib/store";
import {
  checkDashboardAuthMode,
  dashboardLogin,
  getStatus,
  getVersionInfo,
  getWhoami,
  verifyStoredAuth,
} from "./api";

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next",
  );
  // `t` and the object wrapping it must be referentially stable: `navGroups`
  // memoises on `t`, and a fresh identity each render re-runs the
  // `pruneCollapsedNavGroups` effect into an update loop.
  const translation = { t: (key: string) => key };
  return {
    ...actual,
    useTranslation: () => translation,
  };
});

vi.mock("motion/react", async () => {
  const React = await import("react");
  const MotionDiv = ({ children, ...props }: React.HTMLAttributes<HTMLDivElement>) =>
    React.createElement("div", props, children);
  return {
    AnimatePresence: ({ children }: { children: React.ReactNode }) => children,
    MotionConfig: ({ children }: { children: React.ReactNode }) => children,
    motion: { div: MotionDiv },
  };
});

vi.mock("@tanstack/react-router", async () => {
  const React = await import("react");
  return {
    Link: ({
      children,
      to,
      ...rest
    }: { children: React.ReactNode; to?: string } & Record<string, unknown>) =>
      React.createElement("a", { href: to, ...rest }, children),
    Outlet: () => null,
    useNavigate: () => vi.fn(),
    useRouterState: () => ({ location: { pathname: "/overview" } }),
  };
});

// The sidebar/topbar chrome pulls in TanStack Query consumers that are not
// under test here; the bootstrap effect they surround is.
vi.mock("./components/NotificationCenter", () => ({
  NotificationCenter: () => null,
}));
vi.mock("./components/OfflineBanner", () => ({ OfflineBanner: () => null }));
vi.mock("./components/EveryApiPartnerLink", () => ({
  EveryApiPartnerLink: () => null,
}));
vi.mock("./components/ui/CommandPalette", () => ({
  CommandPalette: () => null,
  useCommandPalette: () => ({ isOpen: false, setIsOpen: vi.fn() }),
}));
vi.mock("./components/ui/PushDrawer", () => ({ PushDrawer: () => null }));

vi.mock("./api", () => ({
  changePassword: vi.fn(),
  checkDashboardAuthMode: vi.fn(),
  clearApiKey: vi.fn(),
  dashboardLogin: vi.fn(),
  dashboardLogout: vi.fn(),
  getDashboardUsername: vi.fn(),
  getStatus: vi.fn(),
  getVersionInfo: vi.fn(),
  getWhoami: vi.fn(),
  isPasskeySupported: vi.fn(() => false),
  loginWithPasskey: vi.fn(),
  setApiKey: vi.fn(),
  setOnUnauthorized: vi.fn(),
  verifyStoredAuth: vi.fn(),
}));

async function logIn() {
  const user = userEvent.setup();
  await user.type(
    await screen.findByPlaceholderText("auth.username_placeholder"),
    "operator",
  );
  await user.type(
    screen.getByPlaceholderText("auth.password_placeholder"),
    "password",
  );
  await user.click(screen.getByRole("button", { name: "auth.submit" }));
}

// The shell reads the caller's own emoji and avatar through `whoami`, so it
// needs query context the same way every page does. It did not before #8339 —
// the shell had no react-query hook at all.
function renderApp({ strict = false }: { strict?: boolean } = {}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  const app = strict ? (
    <StrictMode>
      <App />
    </StrictMode>
  ) : (
    <App />
  );
  render(<QueryClientProvider client={queryClient}>{app}</QueryClientProvider>);
}

describe("DashboardApp authed bootstrap", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUIStore.setState({ terminalEnabled: null });
    window.history.pushState({}, "", "/overview");
    vi.mocked(checkDashboardAuthMode).mockResolvedValue("credentials");
    // Unauthenticated on mount, authenticated once the login succeeds.
    vi.mocked(verifyStoredAuth).mockResolvedValueOnce(false).mockResolvedValue(true);
    vi.mocked(dashboardLogin).mockResolvedValue({ ok: true });
    // `/api/version` never sends `hostname` in production (it's public and
    // deliberately omits it) — the real value comes from the authenticated
    // `/api/status`, so that's the only place this test supplies one.
    vi.mocked(getVersionInfo).mockResolvedValue({ version: "test" });
    vi.mocked(getStatus).mockResolvedValue({
      terminal_enabled: true,
      hostname: "myhost",
    } as Awaited<ReturnType<typeof getStatus>>);
    // `/api/auth/dashboard-check` never echoes the username to a caller
    // either — the daemon's identity comes from the authenticated
    // `/api/authz/whoami` instead.
    vi.mocked(getWhoami).mockResolvedValue({ name: "daemon-user", role: "owner" });
  });

  // The assertion this file exists for is the *absence* here. `whoami` sits
  // behind the auth gate, and `parseError` reads any 401 as an expired session
  // — it clears the credential and fires `setOnUnauthorized`, which is the
  // dialog the operator is already looking at. Asking before they log in
  // therefore logs them out of a session they never had.
  it("does not ask who the caller is before they have logged in", async () => {
    renderApp();

    await screen.findByPlaceholderText("auth.username_placeholder");

    expect(vi.mocked(getWhoami)).not.toHaveBeenCalled();
  });

  it("shows the daemon's username in the avatar after logging in", async () => {
    renderApp();

    await logIn();

    // Asserted by accessible name rather than by the glyphs drawn inside it.
    // The avatar is labelled with the daemon's `daemon-user`, which is what
    // separates it from the "operator" typed into the form — the distinction
    // this test exists for, and one that survives whichever glyph the fallback
    // draws. The shell used to take the first two characters (`daemon-user` →
    // "DA") while `getInitials` drew one; the same account therefore showed
    // "RO" beside the menu and "R" inside the Users drawer (#8339). Both now
    // go through `getInitials`.
    await waitFor(() =>
      expect(screen.getAllByRole("img", { name: "daemon-user" }).length).toBeGreaterThan(0),
    );
    expect(screen.queryByRole("img", { name: "operator" })).not.toBeInTheDocument();
  });

  // The test above passes whether or not the emoji ever reaches the shell:
  // `UserAvatar` carries `role="img"` and the name as its label in every
  // state, the initials fallback included. This is the one that fails when the
  // emoji is dropped — which is the whole bug: the daemon stored it, the Users
  // drawer drew it, and the shell the operator was looking at did not.
  it("draws the caller's emoji in the shell, not only their initials", async () => {
    vi.mocked(getWhoami).mockResolvedValue({
      name: "daemon-user",
      role: "owner",
      emoji: "🐙",
    });

    renderApp();

    await logIn();

    await waitFor(() => expect(screen.getAllByText("🐙").length).toBeGreaterThan(0));
  });

  // The hostname is the third of the three values this fix is about, and the
  // only one nothing read back: `getStatus` was mocked with it and no test
  // opened the menu that renders it, so moving it back onto `/api/version` —
  // which never sends it — would not have failed anything.
  it("shows the daemon's hostname in the user menu after logging in", async () => {
    renderApp();

    await logIn();

    // `roleLine` joins the auth mode and the hostname and is rendered by both
    // `UserMenuPanel` sites, hence `getAllByText`. An empty hostname is dropped
    // by the `.filter(Boolean)`, leaving the bare mode — the pre-fix rendering.
    await waitFor(() =>
      expect(screen.getAllByText("credentials · myhost").length).toBeGreaterThan(0),
    );
    expect(screen.queryByText("credentials")).not.toBeInTheDocument();
  });

  it("does not re-run the auth probe after a login succeeds", async () => {
    renderApp();

    await logIn();

    await waitFor(() =>
      expect(screen.getAllByRole("img", { name: "daemon-user" }).length).toBeGreaterThan(0),
    );

    // The probe ran once, on mount, to decide the login dialog needed to
    // show at all. Re-running it on every login (the bootstrap effect used
    // to depend on `authEpoch` directly) races a transient 401/500/timeout
    // from `verifyStoredAuth()` against a session that was just
    // established, and can bounce the user straight back to the dialog it
    // took real credentials to get past.
    expect(verifyStoredAuth).toHaveBeenCalledTimes(1);
    expect(
      screen.queryByPlaceholderText("auth.username_placeholder"),
    ).not.toBeInTheDocument();
  });

  it("applies the daemon's terminal policy after logging in", async () => {
    renderApp();

    await logIn();

    await waitFor(() =>
      expect(screen.getByText("nav.console")).toBeInTheDocument(),
    );
  });

  // `main.tsx` wraps the app in `<React.StrictMode>`, which in development
  // mounts, unmounts and remounts every component once. The mounted-guard
  // ref survives that simulated remount — it is the same fiber — so a
  // cleanup that only ever sets it to `false` leaves it `false` for the rest
  // of the session, and every `fetchAuthedBootstrap` continuation returns
  // early. The avatar, the hostname and the terminal policy then stay on
  // their placeholders: exactly the symptom this PR fixes, reappearing under
  // `vite dev` while production (a single mount) looks fine.
  it("still fills the avatar under StrictMode's double mount", async () => {
    renderApp({ strict: true });

    await logIn();

    await waitFor(() =>
      expect(screen.getAllByRole("img", { name: "daemon-user" }).length).toBeGreaterThan(0),
    );
  });
});
