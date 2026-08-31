import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cloneResultNotice, SystemPromptSection } from "./AgentsPage";
import { usePatchAgent } from "../lib/mutations/agents";

vi.mock("../lib/mutations/agents", () => ({
  usePatchAgent: vi.fn(),
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

  beforeEach(() => {
    vi.clearAllMocks();
    patchMutate = vi.fn();
    usePatchAgentMock.mockReturnValue({ mutate: patchMutate, isPending: false });
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
});

