import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, it, expect, vi } from "vitest";
import {
  AgentManifestForm,
  type ManifestCatalogEntry,
  type ManifestSectionId,
  MANIFEST_SECTION_IDS,
  sectionForInvalidField,
} from "./AgentManifestForm";
import {
  emptyManifestExtras,
  emptyManifestForm,
  type ManifestFormState,
} from "../lib/agentManifest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, opts?: { defaultValue?: string } | Record<string, unknown>) => {
      if (opts && typeof opts === "object" && "defaultValue" in opts) {
        return (opts as { defaultValue?: string }).defaultValue ?? _key;
      }
      return _key;
    },
  }),
}));

interface HarnessModel {
  provider: string;
  id: string;
  context_window?: number;
  max_output_tokens?: number;
  limits_known?: boolean;
}

function Harness({
  skillCatalog,
  toolCatalog,
  mcpCatalog,
  initialState,
  invalidFields = new Set(),
  models = [{ provider: "openai", id: "gpt-4o" }],
  providers = [{ name: "openai" }],
  nameField,
  sections,
}: {
  skillCatalog?: ManifestCatalogEntry[];
  toolCatalog?: ManifestCatalogEntry[];
  mcpCatalog?: ManifestCatalogEntry[];
  initialState?: ManifestFormState;
  invalidFields?: Set<string>;
  models?: HarnessModel[];
  providers?: { name: string }[];
  nameField?: "editable" | "readonly" | "hidden";
  sections?: ManifestSectionId[];
}) {
  const [state, setState] = useState<ManifestFormState>(() => initialState ?? emptyManifestForm());
  return (
    <AgentManifestForm
      value={state}
      onChange={setState}
      providers={providers}
      models={models}
      invalidFields={invalidFields}
      extras={emptyManifestExtras()}
      skillCatalog={skillCatalog}
      toolCatalog={toolCatalog}
      mcpCatalog={mcpCatalog}
      nameField={nameField}
      sections={sections}
    />
  );
}

describe("AgentManifestForm — complexity routing tiers", () => {
  const MODELS = [
    { provider: "openai", id: "gpt-4o" },
    { provider: "anthropic", id: "claude-sonnet-5" },
  ];

  async function openRouting(user: ReturnType<typeof userEvent.setup>) {
    await user.click(screen.getByText("agents.form.routing"));
    await user.click(screen.getByLabelText("agents.form.routing_enabled"));
  }

  // The tier fields hold a bare model name — the daemon resolves it against the
  // global catalog via `ModelCatalog::find_model`, so a `provider/model` string
  // would not resolve. The picker speaks in pairs, and the adapter between the
  // two is exactly where a provider could leak into the stored value.
  it("stores the model name alone when a tier is picked", async () => {
    const user = userEvent.setup();
    render(<Harness models={MODELS} />);
    await openRouting(user);

    await user.click(screen.getByRole("button", { name: "agents.form.simple_model: None" }));
    await user.click(screen.getByRole("button", { name: "anthropic/claude-sonnet-5" }));

    // The trigger reads back from the form state, so this fails if either the
    // provider leaked in or the name never reached `simple_model`.
    expect(
      screen.getByRole("button", { name: "agents.form.simple_model: claude-sonnet-5" }),
    ).toBeInTheDocument();
  });

  it("offers every model in one flat list, with no provider step", async () => {
    const user = userEvent.setup();
    render(<Harness models={MODELS} />);
    await openRouting(user);

    await user.click(screen.getByRole("button", { name: "agents.form.medium_model: None" }));

    // Both providers' models are reachable without drilling in, which is the
    // point of the flat shape for a field that cannot hold a provider.
    expect(screen.getByRole("button", { name: "openai/gpt-4o" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "anthropic/claude-sonnet-5" })).toBeInTheDocument();
  });

  it("accepts a model the catalog has never seen", async () => {
    const user = userEvent.setup();
    render(<Harness models={MODELS} />);
    await openRouting(user);

    await user.click(screen.getByRole("button", { name: "agents.form.complex_model: None" }));
    await user.click(screen.getByRole("button", { name: "Custom" }));
    // No provider field in this shape: requiring one would make a valid entry
    // impossible to commit.
    await user.type(screen.getByLabelText("Model"), "llama-3.3-70b");
    await user.click(screen.getByRole("button", { name: "Confirm" }));

    expect(
      screen.getByRole("button", { name: "agents.form.complex_model: llama-3.3-70b" }),
    ).toBeInTheDocument();
  });
});

describe("AgentManifestForm — provider selection", () => {
  // The caller passes only providers that can serve a request, so an agent
  // assigned to one whose key was rejected (or whose local service is down) is
  // not in that list. The control has to offer it anyway: an operator who
  // cannot see the provider their agent runs on cannot change the model
  // without first moving the agent somewhere it is not.
  //
  // This is deliberately asserted here rather than left to the picker's own
  // suite. The picker only knows the list it is handed; adding the current
  // provider back is `providerOptions`, which is this component's job.
  async function openModelPicker() {
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /^agents\.form\.model:/ }));
    return user;
  }

  it("offers the provider the agent already uses even when it is not selectable anew", async () => {
    const state = emptyManifestForm();
    state.model = { ...state.model, provider: "deepseek", model: "deepseek-chat" };

    render(<Harness initialState={state} providers={[{ name: "openai" }]} />);
    await openModelPicker();

    expect(screen.getByRole("button", { name: "deepseek" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "openai" })).toBeInTheDocument();
  });

  it("does not list a current provider that is already offered, twice", async () => {
    const state = emptyManifestForm();
    state.model = { ...state.model, provider: "openai", model: "gpt-4o" };

    render(<Harness initialState={state} providers={[{ name: "openai" }]} />);
    await openModelPicker();

    expect(screen.getAllByRole("button", { name: "openai" })).toHaveLength(1);
  });
});

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

  it("opens the (defaultOpen=false) Shared Folders section and reddens its title on a validation error (#8013)", () => {
    const state = emptyManifestForm();
    state.workspaces.push({ _uid: "w1", name: "shared", path: "../escape", mode: "rw" });

    render(
      <Harness initialState={state} invalidFields={new Set(["workspaces.w1.path"])} />,
    );

    const pathInput = screen.getByPlaceholderText("agents.form.folder_path");
    expect(pathInput).toHaveAttribute("aria-invalid", "true");
    expect(pathInput.closest("details")).toHaveAttribute("open");
    expect(pathInput.closest("details")?.querySelector("summary")).toHaveAttribute(
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

describe("AgentManifestForm — inference parameters", () => {
  /** The four knobs an agent could not reach before (#7781). */
  it("lets the agent set every sampling preference on the shared ladder", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    for (const label of [
      "model_param.temperature",
      "model_param.top_p",
      "model_param.frequency_penalty",
      "model_param.presence_penalty",
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }

    // The control is shared; the rungs are not. A sampling parameter's ladder
    // carries its own values, so a token count appearing here would mean the
    // shared object had been handed the wrong ladder.
    const tempField = screen.getByText("model_param.temperature").closest("div") as HTMLElement;
    for (const rung of ["0", "0.2", "0.5", "0.7", "1", "1.5", "2"]) {
      expect(within(tempField).getByRole("button", { name: rung })).toBeInTheDocument();
    }
    expect(within(tempField).queryByRole("button", { name: "8K" })).not.toBeInTheDocument();

    const topPField = screen.getByText("model_param.top_p").closest("div") as HTMLElement;
    await user.click(within(topPField).getByRole("button", { name: "0.9" }));
    expect(
      within(topPField).getByRole("button", { name: "0.9", pressed: true }),
    ).toBeInTheDocument();
  });

  it("starts every knob on inherit rather than on a number nobody chose", () => {
    render(<Harness />);
    // Every parameter the form sets, token counts and sampling alike, lands on
    // the inherit rung — the agent states no opinion until someone gives it one.
    for (const param of [
      "context_window",
      "max_tokens",
      "temperature",
      "top_p",
      "frequency_penalty",
      "presence_penalty",
    ]) {
      const field = screen.getByText(`model_param.${param}`).closest("div") as HTMLElement;
      expect(within(field).getByRole("button", { name: "model_param.inherit" })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
    }
  });

  it("replaces the response-length slider with a ladder plus a custom entry", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    // Scoped to the response-length field: the form also renders the context
    // ladder, which legitimately offers 2M. An unscoped query would be asking
    // whether 2M appears anywhere on the page, which is a different question.
    const lengthField = screen.getByText("model_param.max_tokens").closest("div") as HTMLElement;

    // The output ladder stops at 128K. 1M / 2M are context figures, and no
    // model emits a million tokens of reply.
    for (const label of ["1K", "4K", "8K", "16K", "32K", "64K", "128K"]) {
      expect(within(lengthField).getByRole("button", { name: label })).toBeInTheDocument();
    }
    expect(within(lengthField).queryByRole("button", { name: "2M" })).not.toBeInTheDocument();
    expect(within(lengthField).queryByRole("button", { name: "1M" })).not.toBeInTheDocument();

    await user.click(within(lengthField).getByRole("button", { name: "8K" }));
    expect(
      within(lengthField).getByRole("button", { name: "8K", pressed: true }),
    ).toBeInTheDocument();
  });

  it("offers the context ladder up to 2M, which the output ladder must not", () => {
    render(<Harness />);
    const contextField = screen
      .getByText("model_param.context_window")
      .closest("div") as HTMLElement;
    expect(within(contextField).getByRole("button", { name: "2M" })).toBeInTheDocument();
    expect(within(contextField).getByRole("button", { name: "1M" })).toBeInTheDocument();
  });

  it("opens a custom field for a value that is not on the ladder", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    // Scoped to the response-length field: the form now renders seven ladders,
    // so an unscoped "first custom button" is whichever one the layout happens
    // to put first.
    const lengthField = screen.getByText("model_param.max_tokens").closest("div") as HTMLElement;
    await user.click(within(lengthField).getByRole("button", { name: "model_param.custom" }));

    const field = within(lengthField).getByRole("spinbutton", {
      name: "model_param.max_tokens — model_param.custom",
    });
    await user.clear(field);
    await user.type(field, "50000");
    expect(field).toHaveValue(50000);
  });

  /**
   * Warn, do not clamp. A silent truncation leaves the operator debugging a
   * number they never chose — worse than an explicit provider error when the
   * catalog figure is the thing that is wrong.
   */
  it("flags an over-limit response length without changing it", async () => {
    const state = emptyManifestForm();
    state.model.provider = "openai";
    state.model.model = "gpt-4o";
    state.model.max_tokens = "65536";

    render(
      <Harness
        initialState={state}
        models={[
          {
            provider: "openai",
            id: "gpt-4o",
            context_window: 200_000,
            max_output_tokens: 16_384,
            limits_known: true,
          },
        ]}
      />,
    );

    expect(screen.getByText(/agents\.form\.over_limit_warning/)).toBeInTheDocument();
    // The value is untouched, and the field is not marked invalid.
    expect(screen.getByRole("button", { name: "model_param.custom", pressed: true })).toBeInTheDocument();
  });

  /**
   * An inferred limit is a guess, not a ceiling. Warning against one is noise,
   * and noise is what makes operators stop reading warnings (#7780).
   */
  it("stays silent when the model's limits were never sourced", () => {
    const state = emptyManifestForm();
    state.model.provider = "openai";
    state.model.model = "gpt-4o";
    state.model.max_tokens = "65536";

    render(
      <Harness
        initialState={state}
        models={[
          {
            provider: "openai",
            id: "gpt-4o",
            context_window: 131_072,
            max_output_tokens: 16_384,
            limits_known: false,
          },
        ]}
      />,
    );

    expect(screen.queryByText(/agents\.form\.over_limit_warning/)).not.toBeInTheDocument();
  });

  it("hides ladder rungs above a limit the model actually declared", () => {
    const state = emptyManifestForm();
    state.model.provider = "openai";
    state.model.model = "gpt-4o";

    render(
      <Harness
        initialState={state}
        models={[
          {
            provider: "openai",
            id: "gpt-4o",
            context_window: 200_000,
            max_output_tokens: 16_384,
            limits_known: true,
          },
        ]}
      />,
    );

    const lengthField = screen.getByText("model_param.max_tokens").closest("div") as HTMLElement;
    expect(within(lengthField).getByRole("button", { name: "16K" })).toBeInTheDocument();
    expect(within(lengthField).queryByRole("button", { name: "32K" })).not.toBeInTheDocument();
  });
});

// #8028: the agent-type editor drives its own Name input (create) or pins
// identity to a URL segment (edit), and either way this form's own Name
// field must not offer a second, disagreeing way to set it.
describe("AgentManifestForm — nameField", () => {
  it("renders an editable Name field by default", () => {
    render(<Harness />);
    expect(screen.getByRole("textbox", { name: "agents.form.name" })).toBeEnabled();
  });

  it("hides the Name field entirely when nameField is 'hidden'", () => {
    render(<Harness nameField="hidden" />);
    expect(screen.queryByRole("textbox", { name: "agents.form.name" })).not.toBeInTheDocument();
  });

  it("renders the Name field disabled when nameField is 'readonly', pre-filled from the manifest", () => {
    const state = emptyManifestForm();
    state.name = "existing-type";
    render(<Harness initialState={state} nameField="readonly" />);

    const input = screen.getByRole("textbox", { name: "agents.form.name" });
    expect(input).toBeDisabled();
    expect(input).toHaveValue("existing-type");
  });
});

// The agent drawer hosts this form *inside* its tabs, so the same editor
// backs "Conversation", "Routing", "Tools" … rather than living in a second
// "Edit full configuration" drawer. That only works if a caller can name the
// sections it wants, and if naming a subset actually drops the rest — an
// ignored `sections` prop would render the whole manifest on every tab and
// look, to a reader, exactly like the feature working.
describe("AgentManifestForm — section addressing", () => {
  const renderedSections = (container: HTMLElement): string[] =>
    Array.from(container.querySelectorAll("[data-section]")).map(
      (el) => el.getAttribute("data-section") ?? "",
    );

  it("renders every section when the caller passes no list", () => {
    const { container } = render(<Harness />);
    expect(renderedSections(container)).toEqual([...MANIFEST_SECTION_IDS]);
  });

  it("renders only the sections the caller asked for", () => {
    const { container } = render(<Harness sections={["routing"]} />);
    expect(renderedSections(container)).toEqual(["routing"]);
  });

  it("renders one section per tab set, in the order asked", () => {
    // The Routing tab. Each of these used to sit elsewhere: the tiers were
    // two drawers deep and the fallback chain was its own collapsed block in
    // the other surface.
    const routingTab: ManifestSectionId[] = [
      "model",
      "fallback_models",
      "thinking",
      "routing",
    ];
    const { container } = render(<Harness sections={routingTab} />);
    expect(renderedSections(container)).toEqual(routingTab);
  });

  it("renders nothing, not everything, for an empty list", () => {
    // The failure mode worth guarding: an empty array is falsy-ish in the
    // places a caller might spread it, and falling back to "all sections"
    // would put the entire manifest on a tab that asked for none of it.
    const { container } = render(<Harness sections={[]} />);
    expect(renderedSections(container)).toEqual([]);
  });
});

// A validation message that names a field on a tab the operator is not
// looking at is indistinguishable from no message at all, and with the
// sections split across tabs that became possible for the first time.
// `sectionForInvalidField` is what lets the caller jump to the right tab, and
// it is only correct while it covers everything the validator can report.
describe("AgentManifestForm — validation paths are routable", () => {
  it("maps every field path validateManifestForm can report to a section", () => {
    const source = readFileSync(
      join(__dirname, "..", "lib", "agentManifest.ts"),
      "utf8",
    );
    // Every `errors.push("…")` in the validator. Template literals included:
    // `workspaces.${ws._uid}.name` is captured with its placeholder intact,
    // which still matches the `workspaces.` prefix.
    const reported = [...source.matchAll(/errors\.push\(\s*["`]([^"`]+)["`]/g)].map(
      (m) => m[1],
    );

    expect(reported.length).toBeGreaterThan(0);

    const unrouted = reported.filter((path) => sectionForInvalidField(path) === undefined);
    expect(
      unrouted,
      `validateManifestForm reports field paths that no section claims, so an ` +
        `operator who trips one would be told to fix a field the editor cannot ` +
        `navigate to. Add the prefix to FIELD_PREFIX_TO_SECTION.\n\n` +
        `Unrouted: ${unrouted.join(", ")}`,
    ).toEqual([]);
  });
});
