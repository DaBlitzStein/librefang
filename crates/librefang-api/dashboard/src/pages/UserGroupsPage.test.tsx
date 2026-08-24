import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { UserGroupsPage } from "./UserGroupsPage";
import { useUserGroups } from "../lib/queries/userGroups";
import type { UserGroupItem } from "../lib/http/client";

// ---------------------------------------------------------------------------
// Mocks (#7745 — read-only user groups page).
// ---------------------------------------------------------------------------

vi.mock("../lib/queries/userGroups", () => ({
  useUserGroups: vi.fn(),
}));

// Echo the inline English `defaultValue` so assertions can match on the copy
// an operator actually sees, and append the interpolated count for the plural
// key so a member tally is assertable without matching a raw `{{count}}`.
vi.mock("react-i18next", async () => {
  const actual =
    await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: Record<string, unknown>) => {
        if (!opts) return key;
        if ("count" in opts) return `${key}:${String(opts.count)}`;
        if (typeof opts.defaultValue === "string") return opts.defaultValue;
        return key;
      },
    }),
  };
});

// `vi.mocked(useUserGroups)` would preserve the TanStack Query
// `UseQueryResult<UserGroupItem[], Error>` return type, a 15+ field union that
// a partial mock cannot satisfy under strict typecheck. Casting to a generic
// vi.fn shape is the same escape hatch UsersPage.test.tsx uses.
const useUserGroupsMock = useUserGroups as unknown as ReturnType<typeof vi.fn>;

function makeGroup(overrides: Partial<UserGroupItem> = {}): UserGroupItem {
  const members = overrides.members ?? ["mia", "paco"];
  return {
    id: "support",
    name: "Support",
    description: "First-line support rota",
    members,
    member_count: members.length,
    ...overrides,
  };
}

function setGroups(
  items: UserGroupItem[] | undefined,
  opts: { isPending?: boolean; isError?: boolean } = {},
) {
  const isPending = opts.isPending ?? false;
  useUserGroupsMock.mockReturnValue({
    data: items,
    isPending,
    isLoading: isPending,
    isError: opts.isError ?? false,
    isFetching: false,
    refetch: vi.fn(),
  });
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("UserGroupsPage", () => {
  it("renders each declared group with its members", () => {
    setGroups([
      makeGroup(),
      makeGroup({
        id: "platform",
        name: "Platform",
        description: "",
        members: ["paco"],
      }),
    ]);

    render(<UserGroupsPage />);

    expect(screen.getByText("Support")).toBeInTheDocument();
    expect(screen.getByText("Platform")).toBeInTheDocument();
    // The stable id is shown next to the display name, because that is what
    // ownership records point at.
    expect(screen.getByText("support")).toBeInTheDocument();
    expect(screen.getByText("platform")).toBeInTheDocument();
    expect(screen.getByText("First-line support rota")).toBeInTheDocument();
    expect(screen.getAllByText("paco")).toHaveLength(2);
    expect(screen.getByText("mia")).toBeInTheDocument();
  });

  it("shows the member tally per group", () => {
    setGroups([makeGroup(), makeGroup({ id: "audit", name: "Audit", members: [] })]);

    render(<UserGroupsPage />);

    expect(screen.getByText("userGroups.memberCount:2")).toBeInTheDocument();
    expect(screen.getByText("userGroups.memberCount:0")).toBeInTheDocument();
  });

  it("says a group has no members rather than rendering an empty row", () => {
    setGroups([makeGroup({ members: [] })]);

    render(<UserGroupsPage />);

    expect(screen.getByText("No members declared")).toBeInTheDocument();
  });

  // The page is read-only by design, not by omission: membership is derived
  // from config, so a write control would have nothing durable to write to.
  // The note is what stops an operator hunting for a missing "New group"
  // button, so its absence should fail the suite.
  it("explains that groups are edited in config, not here", () => {
    setGroups([makeGroup()]);

    render(<UserGroupsPage />);

    expect(
      screen.getByText(/declared in config\.toml under \[\[user_groups\]\]/),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /new group|create|add/i }),
    ).not.toBeInTheDocument();
  });

  it("filters by group name, id and member name", async () => {
    const user = userEvent.setup();
    setGroups([
      makeGroup(),
      makeGroup({
        id: "platform",
        name: "Platform",
        description: "",
        members: ["zoe"],
      }),
    ]);

    render(<UserGroupsPage />);
    const box = screen.getByPlaceholderText("Search groups or members…");

    // By member: only the group that contains them survives.
    await user.type(box, "zoe");
    expect(screen.getByText("Platform")).toBeInTheDocument();
    expect(screen.queryByText("Support")).not.toBeInTheDocument();

    // By id.
    await user.clear(box);
    await user.type(box, "support");
    expect(screen.getByText("Support")).toBeInTheDocument();
    expect(screen.queryByText("Platform")).not.toBeInTheDocument();
  });

  it("distinguishes no groups at all from no search matches", async () => {
    const user = userEvent.setup();
    setGroups([makeGroup()]);

    render(<UserGroupsPage />);
    await user.type(
      screen.getByPlaceholderText("Search groups or members…"),
      "nothing-matches-this",
    );
    expect(screen.getByText("No matching groups")).toBeInTheDocument();

    setGroups([]);
    render(<UserGroupsPage />);
    expect(screen.getByText("No user groups declared")).toBeInTheDocument();
    // An operator with no groups needs to be told where to declare one.
    expect(
      screen.getByText(/Add a \[\[user_groups\]\] block to config\.toml/),
    ).toBeInTheDocument();
  });

  it("surfaces a load failure instead of an empty list", () => {
    setGroups(undefined, { isError: true });

    render(<UserGroupsPage />);

    expect(screen.getByText("Could not load user groups")).toBeInTheDocument();
    expect(
      screen.queryByText("No user groups declared"),
    ).not.toBeInTheDocument();
  });
});
