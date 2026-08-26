/**
 * Cover for #7751 — an installed skill links back to the page it came from.
 *
 * `skillHubs.test.ts` pins what `skillHubUrl` returns. This pins that the page
 * acts on it, which is the half that can silently stop working: the card reads
 * `source.type` and `source.slug` off the installed-skill payload, so a rename
 * on either side leaves the function correct and the link gone.
 *
 * Both branches are here on purpose. A hub with no addressable page must render
 * no link at all rather than a dead one — that is the whole reason `skillUrl`
 * returns `string | null` instead of a string.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { createTestQueryClient } from "../lib/test/query-client";
import { SkillsPage } from "./SkillsPage";
import * as httpClient from "../lib/http/client";

vi.mock("../lib/http/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/http/client")>();
  return {
    ...actual,
    listSkills: vi.fn(),
    listHands: vi.fn(),
    listPendingCandidates: vi.fn(),
    fanghubListSkills: vi.fn(),
    skillhubBrowse: vi.fn(),
    skillhubSearch: vi.fn(),
    clawhubSearch: vi.fn(),
    clawhubCnSearch: vi.fn(),
  };
});

// Keys rather than English copy, matching `SkillsPage.hubAvailability.test.tsx`:
// the assertion is about the link's target, not about the wording of its title.
vi.mock("react-i18next", async () => {
  const actual =
    await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: Record<string, unknown>) => {
        const params = Object.entries(opts ?? {}).filter(
          ([k]) => k !== "defaultValue",
        );
        return params.length
          ? `${key}(${params.map(([k, v]) => `${k}=${String(v)}`).join(",")})`
          : key;
      },
    }),
  };
});

function installedSkill(
  name: string,
  source?: { type: string; slug: string },
) {
  return {
    name,
    description: `${name} description`,
    version: "1.0.0",
    tags: [],
    is_installed: true,
    ...(source ? { source } : {}),
  } as never;
}

/**
 * The page opens on `browse`; installed cards — the ones carrying an install
 * source — only render under `?tab=installed`. Without this the assertions
 * would pass on an empty document, which is the failure mode this file exists
 * to rule out.
 */
function renderPage() {
  window.history.replaceState({}, "", "/?tab=installed");
  return render(
    <QueryClientProvider client={createTestQueryClient()}>
      <SkillsPage />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(httpClient.listHands).mockResolvedValue([]);
  vi.mocked(httpClient.listPendingCandidates).mockResolvedValue([]);
  vi.mocked(httpClient.fanghubListSkills).mockResolvedValue({
    skills: [],
    total: 0,
  });
  vi.mocked(httpClient.clawhubSearch).mockResolvedValue({ items: [] });
  vi.mocked(httpClient.clawhubCnSearch).mockResolvedValue({ items: [] });
  vi.mocked(httpClient.skillhubBrowse).mockResolvedValue({ items: [] });
  vi.mocked(httpClient.skillhubSearch).mockResolvedValue({ items: [] });
});

describe("SkillsPage marketplace link", () => {
  it("links an installed skill back to its hub page", async () => {
    vi.mocked(httpClient.listSkills).mockResolvedValue([
      installedSkill("pdf-tools", { type: "clawhub", slug: "pdf-tools" }),
    ]);

    renderPage();

    const link = await waitFor(() =>
      screen.getByRole("link", { name: /pdf-tools/ }),
    );
    expect(link).toHaveAttribute("href", "https://clawhub.ai/skills/pdf-tools");
    // Opening a third-party page from the dashboard must not hand it a live
    // `window.opener` back into the daemon's origin.
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", expect.stringContaining("noopener"));
  });

  it("renders no link for a hub with no addressable page", async () => {
    vi.mocked(httpClient.listSkills).mockResolvedValue([
      installedSkill("local-helper", { type: "fanghub", slug: "local-helper" }),
    ]);

    renderPage();

    await waitFor(() =>
      expect(screen.getByText("local-helper")).toBeInTheDocument(),
    );
    expect(
      screen.queryByRole("link", { name: /local-helper/ }),
    ).not.toBeInTheDocument();
  });

  it("renders no link for a skill that records no install source", async () => {
    vi.mocked(httpClient.listSkills).mockResolvedValue([
      installedSkill("hand-written"),
    ]);

    renderPage();

    await waitFor(() =>
      expect(screen.getByText("hand-written")).toBeInTheDocument(),
    );
    expect(
      screen.queryByRole("link", { name: /hand-written/ }),
    ).not.toBeInTheDocument();
  });
});
