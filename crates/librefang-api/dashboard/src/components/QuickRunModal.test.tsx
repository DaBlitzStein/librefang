import { beforeEach, describe, it, expect, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import React from "react";
import { QuickRunModal } from "./QuickRunModal";
import { useAgents } from "../lib/queries/agents";
import { useSpawnEphemeral } from "../lib/mutations/agents";

// `motion/react` ships browser-only animation primitives that jsdom can't
// drive. Same shim as Modal.test — render children inline and turn `motion.foo`
// into the corresponding host tag.
vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => (
    <>{children}</>
  ),
  motion: new Proxy(
    {},
    {
      get: (_target: unknown, prop: string) =>
        ({
          children,
          ...rest
        }: { children?: React.ReactNode } & Record<string, unknown>) =>
          React.createElement(prop, rest, children),
    },
  ),
}));

// Return the key as the rendered string so the assertions below name the exact
// i18n keys this dialog reads. A key that gets renamed shows up as a diff here
// rather than as a silently-missing label at runtime.
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: "en" } }),
}));

vi.mock("../lib/queries/agents", () => ({ useAgents: vi.fn() }));
vi.mock("../lib/mutations/agents", () => ({ useSpawnEphemeral: vi.fn() }));

const addToastMock = vi.fn();
vi.mock("../lib/store", () => ({
  useUIStore: (selector: (s: { addToast: typeof addToastMock }) => unknown) =>
    selector({ addToast: addToastMock }),
}));

const useAgentsMock = useAgents as unknown as ReturnType<typeof vi.fn>;
const useSpawnEphemeralMock = useSpawnEphemeral as unknown as ReturnType<typeof vi.fn>;

const mutateAsync = vi.fn();

const AGENTS = [
  { id: "agent-a", name: "Alpha", is_hand: false },
  { id: "agent-b", name: "Bravo", is_hand: false },
  { id: "hand-1", name: "Greeter", is_hand: true },
];

function setAgents(data: unknown[] = AGENTS, isLoading = false) {
  useAgentsMock.mockReturnValue({ data, isLoading });
}

function submitButton() {
  return screen.getByRole("button", { name: "agents.quick_run_submit" });
}

function taskField() {
  return screen.getByRole("textbox", { name: "agents.quick_run_task" });
}

beforeEach(() => {
  vi.clearAllMocks();
  mutateAsync.mockResolvedValue({
    name: "ephemeral-worker",
    response: "All done.",
    iterations: 3,
    cost_usd: 0.0125,
    tools: ["read_file", "write_file"],
  });
  useSpawnEphemeralMock.mockReturnValue({ mutateAsync, isPending: false });
  setAgents();
});

/**
 * The Quick Run dialog is the only way the dashboard reaches
 * `POST /api/agents/spawn-ephemeral`. #8384/#8385 deleted it along with the
 * control that opened it and left the endpoint live but unreachable from the
 * webui; these tests are what would have failed.
 */
describe("QuickRunModal", () => {
  it("preselects the agent it was opened from", () => {
    render(<QuickRunModal initialParent="agent-b" onClose={() => {}} />);

    expect(screen.getByRole("combobox", { name: "agents.quick_run_parent" })).toHaveValue(
      "agent-b",
    );
  });

  it("offers no hand as a parent, since a hand owns no budget to bill", () => {
    render(<QuickRunModal initialParent="agent-a" onClose={() => {}} />);

    const options = screen.getAllByRole("option").map((o) => o.textContent);
    expect(options).toEqual(["Alpha", "Bravo"]);
  });

  // A stale id — the agent was turned into a hand, or deleted since the list
  // was fetched — must leave a usable select, not a blank one.
  it("falls back to the first candidate when the opening agent cannot be a parent", () => {
    render(<QuickRunModal initialParent="hand-1" onClose={() => {}} />);

    expect(screen.getByRole("combobox", { name: "agents.quick_run_parent" })).toHaveValue(
      "agent-a",
    );
  });

  it("runs the worker on the parent's behalf and shows what came back", async () => {
    render(<QuickRunModal initialParent="agent-b" onClose={() => {}} />);

    fireEvent.change(taskField(), { target: { value: "Summarise the release notes" } });
    fireEvent.click(submitButton());

    await waitFor(() => expect(mutateAsync).toHaveBeenCalledTimes(1));
    expect(mutateAsync).toHaveBeenCalledWith({
      parent: "agent-b",
      message: "Summarise the release notes",
    });

    expect(await screen.findByText("All done.")).toBeTruthy();
    expect(screen.getByText("ephemeral-worker")).toBeTruthy();
    expect(screen.getByText("agents.quick_run_ephemeral_note")).toBeTruthy();
  });

  it("refuses to submit until a task is written", () => {
    render(<QuickRunModal initialParent="agent-a" onClose={() => {}} />);

    expect(submitButton()).toBeDisabled();

    fireEvent.change(taskField(), { target: { value: "   " } });
    expect(submitButton()).toBeDisabled();

    fireEvent.change(taskField(), { target: { value: "do the thing" } });
    expect(submitButton()).not.toBeDisabled();
  });

  it("surfaces a failed run as a toast instead of an empty result panel", async () => {
    // `toastErr` prefers the thrown message, so an empty one is the case that
    // reaches the fallback this dialog supplies — and pins the key it names.
    mutateAsync.mockRejectedValue("");
    render(<QuickRunModal initialParent="agent-a" onClose={() => {}} />);

    fireEvent.change(taskField(), { target: { value: "do the thing" } });
    fireEvent.click(submitButton());

    await waitFor(() => expect(addToastMock).toHaveBeenCalled());
    expect(addToastMock.mock.calls[0][0]).toBe("agents.quick_run_failed");
    expect(addToastMock.mock.calls[0][1]).toBe("error");
    expect(screen.queryByText("agents.quick_run_result")).toBeNull();
  });

  it("says so, rather than offering an empty picker, when no agent can be a parent", () => {
    setAgents([]);
    render(<QuickRunModal onClose={() => {}} />);

    expect(screen.getByText("agents.quick_run_no_agents")).toBeTruthy();
    expect(screen.queryByRole("combobox")).toBeNull();
  });
});
