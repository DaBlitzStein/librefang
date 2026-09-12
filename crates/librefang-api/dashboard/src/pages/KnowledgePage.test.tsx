// Knowledge page tests (#8327).
//
// The two pure helpers are tested directly, and the sharing flow is tested
// through the rendered page because the thing worth pinning is the *payload*:
// sharing is expressed as the complete holder set, so an edit that turned it
// into a per-agent toggle would leave a half-applied grant behind and no unit
// test of a helper would notice.

import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { KnowledgePage, formatBytes, isValidSegment } from "./KnowledgePage";
import * as api from "../api";

// Spread the real module rather than replacing it: this page pulls in
// `lib/store`, which initialises i18n and needs `initReactI18next` to exist.
vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, options?: Record<string, unknown>) =>
        (options?.defaultValue as string) ?? key,
      i18n: { language: "en" },
    }),
  };
});

const BASE: api.KnowledgeBase = {
  name: "handbook",
  path: "knowledge/handbook",
  document_count: 2,
  total_bytes: 2048,
  agents: [
    { agent_id: "11111111-1111-4111-8111-111111111111", agent_name: "reader", alias: "handbook", mode: "r" },
  ],
};

function renderPage() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <KnowledgePage />
    </QueryClientProvider>,
  );
}

describe("formatBytes", () => {
  it("keeps the unit readable at each scale", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MB");
  });
});

describe("isValidSegment", () => {
  it("mirrors the server's rule, including the traversal cases", () => {
    // Not the authority — `is_valid_segment` in routes/knowledge.rs is — but a
    // client check that drifted looser would send requests the server refuses.
    for (const hostile of ["..", ".", "../etc", "a/b", "a\\b", ".hidden", "-lead", "", "with space"]) {
      expect(isValidSegment(hostile), hostile).toBe(false);
    }
    for (const ok of ["handbook", "team-notes", "v2.1_specs", "A9"]) {
      expect(isValidSegment(ok), ok).toBe(true);
    }
    expect(isValidSegment("a".repeat(64))).toBe(true);
    expect(isValidSegment("a".repeat(65))).toBe(false);
  });
});

describe("KnowledgePage", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.spyOn(api, "listKnowledgeBases").mockResolvedValue([BASE]);
    vi.spyOn(api, "listKnowledgeDocuments").mockResolvedValue([]);
    vi.spyOn(api, "listAgents").mockResolvedValue([
      { id: "11111111-1111-4111-8111-111111111111", name: "reader" },
      { id: "22222222-2222-4222-8222-222222222222", name: "writer" },
    ] as never);
  });

  it("shows who holds a base without opening anything", async () => {
    // The listing is half of the point: #8321 was a binding that worked and was
    // only visible from one side.
    renderPage();
    expect(await screen.findByText("handbook")).toBeInTheDocument();
    expect(screen.getByText("reader")).toBeInTheDocument();
  });

  it("sends the complete holder set, not just the agent that was toggled", async () => {
    const setHolders = vi.spyOn(api, "setKnowledgeHolders").mockResolvedValue([]);
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("button", { name: /Share/ }));
    // `reader` already holds it and stays ticked; ticking `writer` must produce
    // a payload naming both, because the route replaces the set it is given.
    const writerRow = await screen.findByText("writer");
    await user.click(writerRow);
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(setHolders).toHaveBeenCalledTimes(1));
    const [name, agents] = setHolders.mock.calls[0];
    expect(name).toBe("handbook");
    expect(agents).toHaveLength(2);
    expect(agents.map((a) => a.agent_id).sort()).toEqual([
      "11111111-1111-4111-8111-111111111111",
      "22222222-2222-4222-8222-222222222222",
    ]);
    // An agent added by ticking the box defaults to read-only: a knowledge base
    // is a thing to read, and write access is a separate, deliberate tick.
    expect(agents.find((a) => a.agent_id.startsWith("2222"))?.mode).toBe("r");
  });

  it("unticking an agent removes it from the payload rather than leaving it", async () => {
    const setHolders = vi.spyOn(api, "setKnowledgeHolders").mockResolvedValue([]);
    const user = userEvent.setup();
    renderPage();

    await user.click(await screen.findByRole("button", { name: /Share/ }));
    // Scoped to the modal: "reader" is also the holder badge on the card behind it.
    const dialog = within(await screen.findByRole("dialog"));
    await user.click(dialog.getByText("reader"));
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(setHolders).toHaveBeenCalledTimes(1));
    expect(setHolders.mock.calls[0][1]).toEqual([]);
  });
});
