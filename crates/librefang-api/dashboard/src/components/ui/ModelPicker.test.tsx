import { beforeAll, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import type { ModelItem, ProviderItem } from "../../api";
import i18n, { i18nReady } from "../../lib/i18n";
import { ModelPicker } from "./ModelPicker";

// The component renders translated strings, so the singleton has to be up
// before any assertion that reads one. Most ui tests skip this and assert only
// on roles; this one needs the Back control, whose name *is* the translation.
beforeAll(async () => {
  await i18nReady;
});

const model = (provider: string, id: string, display_name?: string): ModelItem =>
  ({ provider, id, display_name }) as ModelItem;

const provider = (id: string, extra: Partial<ProviderItem> = {}): ProviderItem =>
  ({ id, ...extra }) as ProviderItem;

/** Open the popover with a click on the trigger. */
function open(label = "Agent model") {
  fireEvent.click(screen.getByRole("button", { name: new RegExp(`^${label}:`) }));
}

describe("ModelPicker", () => {
  it("reports the provider alongside the model, because the id alone is not unique", () => {
    const onChange = vi.fn();
    render(
      <ModelPicker
        label="Agent model"
        value={{ provider: "openai", model: "gpt-4" }}
        onChange={onChange}
        models={[model("anthropic", "claude-sonnet-5"), model("openai", "gpt-5")]}
        providers={[provider("anthropic"), provider("openai")]}
      />,
    );

    open();
    fireEvent.click(screen.getByRole("button", { name: "openai" }));
    fireEvent.click(screen.getByRole("button", { name: "openai/gpt-5" }));

    expect(onChange).toHaveBeenCalledWith({ provider: "openai", model: "gpt-5" });
  });

  it("lists providers alphabetically however the caller ordered them", () => {
    render(
      <ModelPicker
        label="Agent model"
        value={{ provider: "openai", model: "gpt-4" }}
        onChange={() => {}}
        models={[model("openai", "gpt-4"), model("anthropic", "claude-sonnet-5"), model("gemini", "flash-2")]}
        providers={[provider("openai"), provider("gemini"), provider("anthropic")]}
      />,
    );

    open();
    const ids = ["anthropic", "gemini", "openai"];
    const rows = screen
      .getAllByRole("button")
      .filter((b) => ids.includes(b.getAttribute("aria-label") ?? ""))
      .map((b) => b.getAttribute("aria-label"));

    expect(rows).toEqual(["anthropic", "gemini", "openai"]);
  });

  it("does not mark a model active when only the id matches, not the provider", () => {
    // `claude-sonnet-5` served by two providers is two different choices. A
    // check on the id alone would light up the wrong row — and, because the
    // click handler returns early on an active row, would make the other
    // provider's copy unselectable.
    const onChange = vi.fn();
    render(
      <ModelPicker
        label="Agent model"
        value={{ provider: "anthropic", model: "claude-sonnet-5" }}
        onChange={onChange}
        models={[model("anthropic", "claude-sonnet-5"), model("openai", "claude-sonnet-5")]}
        providers={[provider("anthropic"), provider("openai")]}
      />,
    );

    open();

    fireEvent.click(screen.getByRole("button", { name: "anthropic" }));
    expect(screen.getByRole("button", { name: "anthropic/claude-sonnet-5" })).toHaveAttribute(
      "aria-current",
      "true",
    );

    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    fireEvent.click(screen.getByRole("button", { name: "openai" }));
    const other = screen.getByRole("button", { name: "openai/claude-sonnet-5" });

    expect(other).not.toHaveAttribute("aria-current");
    fireEvent.click(other);
    expect(onChange).toHaveBeenCalledWith({ provider: "openai", model: "claude-sonnet-5" });
  });

  it("narrows the model list to the search term, on id or display name", () => {
    render(
      <ModelPicker
        label="Agent model"
        value={{ provider: "openai", model: "gpt-4" }}
        onChange={() => {}}
        models={[
          model("openai", "gpt-4"),
          model("openai", "o3-mini", "o3 Mini (fast)"),
          model("anthropic", "claude-sonnet-5"),
        ]}
        providers={[provider("openai"), provider("anthropic")]}
      />,
    );

    open();
    fireEvent.click(screen.getByRole("button", { name: "openai" }));

    const search = screen.getByRole("textbox");
    fireEvent.change(search, { target: { value: "mini" } });

    expect(screen.getByRole("button", { name: "openai/o3-mini" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "openai/gpt-4" })).not.toBeInTheDocument();
  });

  it("returns to the provider list from Back, dropping the search with it", () => {
    render(
      <ModelPicker
        label="Agent model"
        value={{ provider: "openai", model: "gpt-4" }}
        onChange={() => {}}
        models={[model("openai", "gpt-4"), model("anthropic", "claude-sonnet-5")]}
        providers={[provider("openai"), provider("anthropic")]}
      />,
    );

    open();
    fireEvent.click(screen.getByRole("button", { name: "openai" }));
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "gpt" } });
    fireEvent.click(screen.getByRole("button", { name: i18n.t("common.back") }));

    // Both providers are reachable again — a search term left behind would
    // have filtered the top level down to whatever matched "gpt".
    expect(screen.getByRole("button", { name: "anthropic" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "openai" })).toBeInTheDocument();
  });

  it("closes on Escape even with the search box focused", () => {
    render(
      <ModelPicker
        label="Agent model"
        value={{ provider: "openai", model: "gpt-4" }}
        onChange={() => {}}
        models={[model("openai", "gpt-4")]}
        providers={[provider("openai")]}
      />,
    );

    const trigger = screen.getByRole("button", { name: /^Agent model:/ });
    expect(trigger).toHaveAttribute("aria-expanded", "false");

    open();
    expect(trigger).toHaveAttribute("aria-expanded", "true");

    fireEvent.keyDown(document, { key: "Escape" });
    expect(trigger).toHaveAttribute("aria-expanded", "false");
  });

  it("will not drill into a provider the catalog cannot serve", () => {
    render(
      <ModelPicker
        label="Agent model"
        value={null}
        onChange={() => {}}
        models={[model("openai", "gpt-4")]}
        providers={[provider("openai"), provider("groq", { reachable: false })]}
      />,
    );

    open();
    const groq = screen.getByRole("button", { name: "groq" });
    expect(groq).toBeDisabled();

    fireEvent.click(groq);
    // Still on the provider level: no model rows, no Back control.
    expect(screen.queryByRole("button", { name: i18n.t("common.back") })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "openai" })).toBeInTheDocument();
  });
});
