import { beforeAll, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { i18nReady } from "../../lib/i18n";
import { CollapsibleSection } from "./CollapsibleSection";
import { Field } from "./Field";

// The badge is a translation, and it is the translated text that gets asserted:
// that is also what proves the plural forms resolve rather than falling back to
// a raw key.
beforeAll(async () => {
  await i18nReady;
});

describe("CollapsibleSection", () => {
  it("starts folded so a long form does not open as a wall", () => {
    render(
      <CollapsibleSection title="Routing">
        <p>body</p>
      </CollapsibleSection>,
    );
    expect(screen.getByText("Routing").closest("details")).not.toHaveAttribute("open");
  });

  it("opens when asked", () => {
    render(
      <CollapsibleSection title="Routing" defaultOpen>
        <p>body</p>
      </CollapsibleSection>,
    );
    expect(screen.getByText("Routing").closest("details")).toHaveAttribute("open");
  });

  it("forces itself open when invalid, because a hidden error reads as no error", () => {
    render(
      <CollapsibleSection title="Routing" invalid>
        <p>body</p>
      </CollapsibleSection>,
    );
    const details = screen.getByText("Routing").closest("details");
    expect(details).toHaveAttribute("open");
    expect(screen.getByText("Routing").closest("summary")).toHaveAttribute(
      "aria-invalid",
      "true",
    );
  });

  it("is a real details/summary, so the keyboard and toggle behaviour come free", () => {
    const { container } = render(
      <CollapsibleSection title="Routing">
        <p>body</p>
      </CollapsibleSection>,
    );
    expect(container.querySelector("details")).not.toBeNull();
    expect(container.querySelector("summary")).not.toBeNull();
  });

  it("keeps the body mounted while folded, so form state survives", () => {
    render(
      <CollapsibleSection title="Routing">
        <input aria-label="threshold" defaultValue="100" />
      </CollapsibleSection>,
    );
    // The value is still in the DOM: folding must not unmount a control, or
    // the field would silently reset every time the section was collapsed.
    expect(screen.getByLabelText("threshold")).toHaveValue("100");
  });
});

/** A section whose field appears on a click, standing in for the real ones. */
function Growing() {
  const [extra, setExtra] = useState(false);
  return (
    <CollapsibleSection title="Routing">
      <Field label="first">
        <input />
      </Field>
      {extra && (
        <Field label="second">
          <input />
        </Field>
      )}
      <button type="button" onClick={() => setExtra(true)}>
        add
      </button>
    </CollapsibleSection>
  );
}

// A folded section and an empty one look alike from the outside, which an
// operator reads as "the editor has no fields for this" — the section bar says
// how many it holds instead.
describe("CollapsibleSection — field count", () => {
  it("says how many fields the section holds", () => {
    render(
      <CollapsibleSection title="Routing">
        <Field label="first">
          <input />
        </Field>
        <Field label="second">
          <input />
        </Field>
        <Field label="third">
          <input />
        </Field>
      </CollapsibleSection>,
    );

    expect(screen.getByText("3 fields")).toBeInTheDocument();
  });

  it("counts the fields folded inside an advanced group, not only the visible ones", () => {
    render(
      <CollapsibleSection title="Routing">
        <Field label="visible">
          <input />
        </Field>
        <details data-advanced>
          <Field label="folded">
            <input />
          </Field>
          <Field label="also folded">
            <input />
          </Field>
        </details>
      </CollapsibleSection>,
    );

    expect(screen.getByText("3 fields")).toBeInTheDocument();
  });

  it("reads one field as one field", () => {
    render(
      <CollapsibleSection title="Routing">
        <Field label="only">
          <input />
        </Field>
      </CollapsibleSection>,
    );

    expect(screen.getByText("1 field")).toBeInTheDocument();
  });

  it("counts a widget nested in a field once, not once per wrapper", () => {
    // `Field > Field` is the shape a widget that marks itself inside a labelled
    // wrapper produces; two markers, one field.
    render(
      <CollapsibleSection title="Routing">
        <Field label="outer">
          <Field label="inner">
            <input />
          </Field>
        </Field>
      </CollapsibleSection>,
    );

    expect(screen.getByText("1 field")).toBeInTheDocument();
  });

  it("draws no badge when the section holds no fields", () => {
    // A repeatable editor with no rows has nothing to report, and "0 fields"
    // would be the same empty-looking bar this badge exists to remove.
    render(
      <CollapsibleSection title="Routing">
        <p>nothing configured yet</p>
      </CollapsibleSection>,
    );

    expect(screen.queryByText(/field/)).not.toBeInTheDocument();
    expect(screen.getByText("Routing")).toBeInTheDocument();
  });

  it("keeps counting fields that appear after the first render", async () => {
    const user = userEvent.setup();
    render(<Growing />);

    expect(screen.getByText("1 field")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "add" }));
    expect(await screen.findByText("2 fields")).toBeInTheDocument();
  });

  it("carries the count into the summary's accessible name", () => {
    render(
      <CollapsibleSection title="Routing">
        <Field label="first">
          <input />
        </Field>
        <Field label="second">
          <input />
        </Field>
      </CollapsibleSection>,
    );

    // Not aria-hidden: "is this section empty?" is the question the badge
    // answers, and a screen-reader operator needs the answer too. The unit
    // keeps the name a phrase rather than "Routing 2".
    expect(screen.getByText("Routing").closest("summary")).toHaveTextContent(
      "Routing 2 fields",
    );
  });
});
