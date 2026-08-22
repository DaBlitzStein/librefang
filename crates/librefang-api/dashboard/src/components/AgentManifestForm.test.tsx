import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, it, expect, vi } from "vitest";
import { AgentManifestForm, type ManifestCatalogEntry } from "./AgentManifestForm";
import {
  emptyManifestExtras,
  emptyManifestForm,
  type ManifestFormState,
} from "../lib/agentManifest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, opts?: { defaultValue?: string } | Record<string, unknown>) => {
      if (opts && typeof opts === "object" && "defaultValue" in opts) {
        const template = (opts as { defaultValue?: string }).defaultValue ?? _key;
        // Interpolate `{{name}}` from the same options bag, as i18next does.
        // Returning the raw template would let a test assert on placeholder
        // text and pass while the real UI renders a different string.
        return template.replace(/\{\{(\w+)\}\}/g, (whole, name: string) => {
          const v = (opts as Record<string, unknown>)[name];
          return v === undefined ? whole : String(v);
        });
      }
      return _key;
    },
  }),
}));

function Harness({
  skillCatalog,
  toolCatalog,
  mcpCatalog,
  initialState,
  invalidFields = new Set(),
  systemDefaultModel,
}: {
  skillCatalog?: ManifestCatalogEntry[];
  toolCatalog?: ManifestCatalogEntry[];
  mcpCatalog?: ManifestCatalogEntry[];
  initialState?: ManifestFormState;
  invalidFields?: Set<string>;
  systemDefaultModel?: { provider?: string; model?: string };
}) {
  const [state, setState] = useState<ManifestFormState>(() => initialState ?? emptyManifestForm());
  return (
    <AgentManifestForm
      value={state}
      onChange={setState}
      providers={[{ name: "openai" }]}
      models={[{ provider: "openai", id: "gpt-4o" }]}
      invalidFields={invalidFields}
      extras={emptyManifestExtras()}
      skillCatalog={skillCatalog}
      toolCatalog={toolCatalog}
      mcpCatalog={mcpCatalog}
      systemDefaultModel={systemDefaultModel}
    />
  );
}

describe("AgentManifestForm — validation feedback", () => {
  it("opens scheduling errors and exposes the cron error to assistive technology", () => {
    const state = emptyManifestForm();
    state.schedule = { mode: "periodic", cron: "" };

    render(<Harness initialState={state} invalidFields={new Set(["schedule.cron"])} />);

    const input = screen.getByRole("textbox", { name: "agents.form.cron" });
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(input).toHaveAttribute("aria-required", "true");
    expect(input).toHaveAccessibleDescription("agents.form.cron_required_error");
    expect(input.closest("details")).toHaveAttribute("open");
    expect(input.closest("details")?.querySelector("summary")).toHaveAttribute(
      "aria-invalid",
      "true",
    );
  });

  it("opens scheduling errors and exposes an invalid continuous interval", () => {
    const state = emptyManifestForm();
    state.schedule = { mode: "continuous", check_interval_secs: "0" };

    render(
      <Harness
        initialState={state}
        invalidFields={new Set(["schedule.check_interval_secs"])}
      />,
    );

    const input = screen.getByRole("spinbutton", {
      name: "agents.form.check_interval_secs",
    });
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(input).toHaveAttribute("aria-required", "true");
    expect(input).toHaveAccessibleDescription("agents.detail.schedule_invalid_interval");
    expect(input.closest("details")).toHaveAttribute("open");
    expect(input.closest("details")?.querySelector("summary")).toHaveAttribute(
      "aria-invalid",
      "true",
    );
  });

  it("opens response-format errors and exposes the schema error to assistive technology", () => {
    const state = emptyManifestForm();
    state.response_format = { mode: "json_schema", name: "response", schema: "", strict: false };

    render(
      <Harness
        initialState={state}
        invalidFields={new Set(["response_format.schema"])}
      />,
    );

    const textarea = screen.getByRole("textbox", { name: "agents.form.schema_body" });
    expect(textarea).toHaveAttribute("aria-invalid", "true");
    expect(textarea).toHaveAttribute("aria-required", "true");
    expect(textarea).toHaveAccessibleDescription("agents.form.schema_invalid_error");
    expect(textarea.closest("details")).toHaveAttribute("open");
    expect(textarea.closest("details")?.querySelector("summary")).toHaveAttribute(
      "aria-invalid",
      "true",
    );
  });
});

describe("AgentManifestForm — tools/skills/mcp selection (#5246)", () => {
  it("clicking a tool option from the dropdown adds it as a chip", async () => {
    const user = userEvent.setup();
    render(
      <Harness
        toolCatalog={[
          { name: "read_file", description: "Read a file" },
          { name: "write_file", description: "Write a file" },
        ]}
      />,
    );

    // Open the tools combobox: target the search input by its placeholder.
    const toolsInput = screen.getByPlaceholderText("Search tools…");
    await user.click(toolsInput);

    // Wait for the option to appear, then click it.
    const option = await screen.findByText("read_file");
    await user.click(option);

    // Chip should appear; remove button is the canonical signal.
    expect(
      screen.getByRole("button", { name: "Remove read_file" }),
    ).toBeInTheDocument();
  });

  it("clicking a skill option from the dropdown adds it as a chip", async () => {
    const user = userEvent.setup();
    render(
      <Harness
        skillCatalog={[
          { name: "summarise", description: "Summarise text" },
          { name: "translate", description: "Translate text" },
        ]}
      />,
    );

    const skillsInput = screen.getByPlaceholderText("Search installed skills…");
    await user.click(skillsInput);

    const option = await screen.findByText("summarise");
    await user.click(option);

    expect(
      screen.getByRole("button", { name: "Remove summarise" }),
    ).toBeInTheDocument();
  });

  it("clicking an MCP server option adds it as a chip (#5246)", async () => {
    const user = userEvent.setup();
    render(
      <Harness
        mcpCatalog={[
          { name: "filesystem", description: "Local filesystem MCP" },
          { name: "github", description: "GitHub MCP" },
        ]}
      />,
    );

    // The MCP field should render a combobox, not a free-text TagInput.
    const mcpInput = screen.getByPlaceholderText("Search MCP servers…");
    await user.click(mcpInput);

    const option = await screen.findByText("github");
    await user.click(option);

    expect(
      screen.getByRole("button", { name: "Remove github" }),
    ).toBeInTheDocument();
  });

  it("when no MCP catalog is supplied, falls back to a tag input (no crash)", async () => {
    render(<Harness />);
    // The mcp_servers Field always exists; without a catalog the TagInput is used
    // — verified by the absence of the cmdk search placeholder.
    expect(screen.queryByPlaceholderText("Search MCP servers…")).not.toBeInTheDocument();
  });

  it("tool dropdown options are within a listbox region after focus", async () => {
    const user = userEvent.setup();
    render(
      <Harness
        toolCatalog={[
          { name: "read_file" },
          { name: "write_file" },
        ]}
      />,
    );
    const toolsInput = screen.getByPlaceholderText("Search tools…");
    await user.click(toolsInput);

    const list = await screen.findByRole("listbox");
    expect(within(list).getByText("read_file")).toBeInTheDocument();
    expect(within(list).getByText("write_file")).toBeInTheDocument();
  });
});

describe("AgentManifestForm — compact controls", () => {
  it("clears duplicate text submitted to a tag input", async () => {
    const user = userEvent.setup();
    const state = emptyManifestForm();
    state.mcp_servers = ["filesystem"];
    render(<Harness initialState={state} />);

    const removeButton = screen.getByRole("button", { name: "remove filesystem" });
    const input = removeButton.parentElement?.parentElement?.querySelector("input");
    expect(input).toBeInstanceOf(HTMLInputElement);
    if (!(input instanceof HTMLInputElement)) return;

    await user.type(input, "filesystem{Enter}");
    expect(input).toHaveValue("");
    expect(screen.getAllByRole("button", { name: "remove filesystem" })).toHaveLength(1);
  });

  it("gives the stream-thinking checkbox an accessible name", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByRole("checkbox", { name: "agents.form.thinking_enabled" }));

    expect(
      screen.getByRole("checkbox", { name: "agents.form.stream_thinking" }),
    ).toBeInTheDocument();
  });
});

/**
 * A manifest may carry the literal string `"default"` for provider and model,
 * meaning "inherit the kernel's `[default_model]`". The form used to have no
 * matching `<option>`, so the select rendered an unmatched value: the editor
 * showed `default` — a word naming neither a provider nor a model — while
 * every other screen showed the real one. Two screens disagreeing about the
 * same agent reads as a bug even when the manifest is correct.
 *
 * This is the exact confusion a user hit on a live deployment: "cuando edito
 * el agente de profesor me aparece que el modelo es default. Cuando en todo lo
 * demás aparece que el modelo es el litellm blablabla-high".
 */
describe("AgentManifestForm — inherited model", () => {
  const inheriting = (): ManifestFormState => {
    const state = emptyManifestForm();
    state.model = { ...state.model, provider: "default", model: "default" };
    return state;
  };

  // `Field` wraps its label in a <span>, not a <label>, so these selects carry
  // no accessible name. Reach them through the option instead — which is also
  // what the assertion is really about.
  const selectOffering = (optionName: string): HTMLSelectElement => {
    const option = screen.getByRole("option", { name: optionName });
    const select = option.closest("select");
    if (!select) throw new Error(`option "${optionName}" is not inside a select`);
    return select as HTMLSelectElement;
  };

  it("offers the inherit sentinel as a real option instead of an unmatched value", () => {
    render(<Harness initialState={inheriting()} />);

    // Without a matching <option> a select shows its first entry, so the
    // stored sentinel silently became "select a provider".
    expect(selectOffering("Inherit system default").value).toBe("default");
  });

  it("names the provider and model the agent will actually run", () => {
    render(
      <Harness
        initialState={inheriting()}
        systemDefaultModel={{ provider: "litellm", model: "sensor-model-generic-high" }}
      />,
    );

    expect(screen.getByText("Currently: litellm")).toBeInTheDocument();
    expect(screen.getByText("Currently: sensor-model-generic-high")).toBeInTheDocument();
  });

  it("says nothing extra when the agent pins its own model", () => {
    const state = emptyManifestForm();
    state.model = { ...state.model, provider: "openai", model: "gpt-4o" };

    render(
      <Harness
        initialState={state}
        systemDefaultModel={{ provider: "litellm", model: "sensor-model-generic-high" }}
      />,
    );

    // Showing the system default beside an explicit choice would imply the
    // choice is being ignored.
    expect(screen.queryByText(/^Currently: /)).toBeNull();
  });

  it("labels the sentinel but invents no value when the default is unknown", () => {
    render(<Harness initialState={inheriting()} />);

    expect(selectOffering("Inherit system default").value).toBe("default");
    expect(screen.queryByText(/^Currently: /)).toBeNull();
  });
});
