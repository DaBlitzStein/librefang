import { beforeAll, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { i18nReady } from "../lib/i18n";
import { AdvancedFields } from "./AgentManifestForm";
import { Field } from "./ui/Field";

// The badge is a translation, and it is the translated text that gets asserted
// — which is also what proves the plural forms resolve rather than falling back
// to a raw key. The form's own test file stubs `react-i18next` with a `t` that
// echoes keys and drops interpolation values, so the count would be invisible
// there; this file uses the real singleton instead.
//
// This file covers the fold's count; the section bar's own is covered in
// `ui/CollapsibleSection.test.tsx`, and both run off the same hook.
beforeAll(async () => {
  await i18nReady;
});

const field = (label: string) => (
  <Field label={label}>
    <input />
  </Field>
);

describe("AdvancedFields — the fold's own field count", () => {
  it("says how many fields the fold hides", () => {
    render(
      <AdvancedFields>
        {field("first")}
        {field("second")}
        {field("third")}
      </AdvancedFields>,
    );

    // Closed on mount, so these three are three fields the operator cannot see:
    // a bare "Advanced" reads as "nothing worth opening" exactly the way a bare
    // section bar did.
    expect(screen.getByText("3 fields")).toBeInTheDocument();
  });

  it("keeps the count on the summary when the fold is opened", async () => {
    const user = userEvent.setup();
    const { container } = render(
      <AdvancedFields>
        {field("first")}
        {field("second")}
      </AdvancedFields>,
    );

    expect(screen.getByText("2 fields")).toBeInTheDocument();
    await user.click(container.querySelector("summary")!);

    // The badge belongs to the bar, not the body: opening the fold must not
    // move it, drop it, or count the fields a second time.
    expect(container.querySelector("details")).toHaveAttribute("open");
    expect(screen.getByText("2 fields")).toBeInTheDocument();
    expect(screen.queryByText("4 fields")).not.toBeInTheDocument();
  });

  it("reads a single hidden field as one field", () => {
    render(<AdvancedFields>{field("only")}</AdvancedFields>);

    expect(screen.getByText("1 field")).toBeInTheDocument();
  });

  it("counts its own fields, not the ones behind a fold inside it", () => {
    render(
      <AdvancedFields>
        {field("outer")}
        <AdvancedFields>
          {field("inner one")}
          {field("inner two")}
        </AdvancedFields>
      </AdvancedFields>,
    );

    // The outer fold hides three fields — the inner fold's two are behind two
    // clicks, not one — and the inner hides two. Both numbers are on screen at
    // once, which is what keeps them from having to add up.
    expect(screen.getByText("3 fields")).toBeInTheDocument();
    expect(screen.getByText("2 fields")).toBeInTheDocument();
  });

  it("draws no badge when the fold holds no fields", () => {
    render(
      <AdvancedFields>
        <p>nothing configured yet</p>
      </AdvancedFields>,
    );

    expect(screen.queryByText(/field/)).not.toBeInTheDocument();
    expect(screen.getByText("Advanced")).toBeInTheDocument();
  });

  it("carries the count into the summary's accessible name", () => {
    render(
      <AdvancedFields>
        {field("first")}
        {field("second")}
      </AdvancedFields>,
    );

    // Not aria-hidden, for the same reason the section bar's count is not: the
    // number is the answer to "is there anything behind this?", and a
    // screen-reader operator needs the answer too.
    expect(screen.getByText("Advanced").closest("summary")).toHaveTextContent(
      "Advanced 2 fields",
    );
  });
});
