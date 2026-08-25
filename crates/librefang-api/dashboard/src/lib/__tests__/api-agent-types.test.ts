import { beforeEach, describe, expect, it, vi } from "vitest";
import { listAgentTypes } from "../../api";

/**
 * `GET /api/agent-types` answers `{ templates: [...] }`.
 *
 * `listAgentTypes` read `data.items` — the paginated shape used elsewhere —
 * and coalesced the resulting `undefined` to `[]`. The request succeeded, no
 * error surfaced, and the Agent Types page rendered an empty catalog with 34
 * types sitting on disk.
 *
 * Mocking `fetch` rather than `http/client` is the point: the defect was in
 * how this module reads the response body, which a client-level mock steps
 * over entirely.
 */
describe("listAgentTypes", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  function mockJson(body: unknown) {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: async () => body,
      }),
    );
  }

  it("reads the entries out of `templates`", async () => {
    mockJson({
      templates: [
        { name: "researcher", source: "agent-type", editable: true },
        { name: "Profesor", source: "agent", editable: false },
      ],
    });

    const types = await listAgentTypes();

    expect(types.map((t) => t.name)).toEqual(["researcher", "Profesor"]);
  });

  it("returns an empty list only when the catalog is genuinely empty", async () => {
    mockJson({ templates: [] });
    await expect(listAgentTypes()).resolves.toEqual([]);
  });
});
