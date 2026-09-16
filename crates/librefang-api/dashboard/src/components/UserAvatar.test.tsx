import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { UserAvatar } from "./UserAvatar";
import { fetchAuthenticatedImage } from "../api";

vi.mock("../api", () => ({
  fetchAuthenticatedImage: vi.fn(),
  currentUserAvatarPath: vi.fn(() => "/api/users/me/avatar"),
}));

function renderAvatar(props: {
  name: string;
  fallback?: string;
  emoji?: string;
  hasAvatar?: boolean;
}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <UserAvatar {...props} />
    </QueryClientProvider>,
  );
  return queryClient;
}

describe("UserAvatar", () => {
  beforeEach(() => {
    vi.mocked(fetchAuthenticatedImage).mockReset();
    vi.mocked(fetchAuthenticatedImage).mockResolvedValue(
      new Blob(["x"], { type: "image/png" }),
    );
  });

  // The shell draws this before it knows who the caller is. An empty name must
  // mean "no request", not "request the avatar of a user called ''" — the
  // endpoint is authenticated, so that request is a 401, and a 401 is what the
  // API layer reads as an expired session.
  it("asks for nothing when there is no name yet", async () => {
    renderAvatar({ name: "", fallback: "U" });

    await waitFor(() => expect(screen.getByRole("img", { name: "U" })).toBeInTheDocument());
    expect(vi.mocked(fetchAuthenticatedImage)).not.toHaveBeenCalled();
  });

  // `fallback` exists to be drawn, not to identify anybody. Splitting it out is
  // what lets the empty-name case be free: before, the shell handed the
  // placeholder in as the name, which filed a cache entry under a user who does
  // not exist. `useDeleteUserAvatar` only removes the real name's entry, so the
  // placeholder's copy outlived a delete and kept serving the picture from a
  // 300-second `staleTime`.
  it("files no cache entry for the placeholder when there is no name", async () => {
    const queryClient = renderAvatar({ name: "", fallback: "U" });

    await waitFor(() => expect(screen.getByRole("img", { name: "U" })).toBeInTheDocument());

    const keys = queryClient.getQueryCache().getAll().map((q) => q.queryKey);
    // A disabled query still registers an entry, so the assertion is about
    // which name it files under — "U" is the placeholder, and it must not be
    // what the cache remembers.
    expect(keys).not.toContainEqual(["users", "avatar", "U"]);
    expect(vi.mocked(fetchAuthenticatedImage)).not.toHaveBeenCalled();
  });

  // The placeholder only replaces initials that would otherwise be drawn. A
  // caller that passes a real name and no placeholder must still get that
  // name's initials — the two props answer different questions.
  it("derives the initials from the name when no placeholder is given", async () => {
    renderAvatar({ name: "alice", hasAvatar: false });

    await waitFor(() => expect(screen.getByRole("img", { name: "alice" })).toBeInTheDocument());
    expect(screen.getByText("A")).toBeInTheDocument();
  });

  it("draws the emoji instead of the initials when there is one", async () => {
    renderAvatar({ name: "daemon-user", fallback: "U", emoji: "\u{1F419}", hasAvatar: false });

    await waitFor(() => expect(screen.getByText("\u{1F419}")).toBeInTheDocument());
    // `has_avatar: false` is the daemon saying there is no picture — the one
    // answer that must skip the request rather than pay a 404 for it.
    expect(vi.mocked(fetchAuthenticatedImage)).not.toHaveBeenCalled();
  });
});
