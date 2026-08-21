import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SliderInput } from "./SliderInput";

/**
 * A row is inactive whenever the model carries no override for that field,
 * which is the default for every field of every freshly-listed model. So the
 * inactive state is not an edge case here — it is what the form looks like
 * before anyone touches it, and the switch is the only way out of it.
 *
 * The regression these tests pin: the dimming used to sit on the row's
 * container, so it swept up the switch as well. An operator opening the model
 * form saw every control faded, including the control that un-fades them,
 * drawn in the faintest colour token in the palette. The form read as broken
 * rather than as opt-in.
 */
describe("SliderInput", () => {
  const base = { label: "Context window", value: 8192, min: 1024, max: 131072 };

  it("keeps the toggle at full opacity while the row is inactive", async () => {
    render(
      <SliderInput {...base} enabled={false} onChange={() => {}} onToggle={() => {}} />,
    );

    const toggle = screen.getByRole("switch", { name: "Context window" });
    expect(toggle.className).not.toMatch(/opacity-/);

    // The values are dimmed — that part is intended, and asserting it here
    // stops a future fix from simply deleting the dimming altogether.
    expect(screen.getByRole("slider").className).toMatch(/opacity-40/);
    expect(screen.getByRole("spinbutton").className).toMatch(/opacity-40/);
  });

  it("draws the off state in a colour that reads as a control, not a divider", () => {
    render(
      <SliderInput {...base} enabled={false} onChange={() => {}} onToggle={() => {}} />,
    );

    const toggle = screen.getByRole("switch", { name: "Context window" });
    // `border-subtle` is the hairline token used for dividers; a switch that
    // borrows it disappears against the surface.
    expect(toggle.className).not.toMatch(/bg-border-subtle/);
  });

  it("activates an inactive row from a single click on the switch", async () => {
    const onToggle = vi.fn();
    render(
      <SliderInput {...base} enabled={false} onChange={() => {}} onToggle={onToggle} />,
    );

    await userEvent.click(screen.getByRole("switch", { name: "Context window" }));
    expect(onToggle).toHaveBeenCalledWith(true);
  });

  it("reports its state to assistive technology", () => {
    const { rerender } = render(
      <SliderInput {...base} enabled={false} onChange={() => {}} onToggle={() => {}} />,
    );
    expect(screen.getByRole("switch")).toHaveAttribute("aria-checked", "false");

    rerender(
      <SliderInput {...base} enabled onChange={() => {}} onToggle={() => {}} />,
    );
    expect(screen.getByRole("switch")).toHaveAttribute("aria-checked", "true");
  });

  it("edits the value once the row is active", async () => {
    const onChange = vi.fn();
    render(<SliderInput {...base} enabled onChange={onChange} onToggle={() => {}} />);

    const field = screen.getByRole("spinbutton");
    expect(field).not.toBeDisabled();
    await userEvent.clear(field);
    await userEvent.type(field, "16384");
    expect(onChange).toHaveBeenCalled();
  });

  it("clamps a typed value to the declared range", async () => {
    const onChange = vi.fn();
    render(<SliderInput {...base} enabled onChange={onChange} onToggle={() => {}} />);

    await userEvent.clear(screen.getByRole("spinbutton"));
    await userEvent.type(screen.getByRole("spinbutton"), "999999999");
    // Every recorded call stays inside [min, max] — the field cannot be used
    // to smuggle a context window the provider will reject.
    for (const [v] of onChange.mock.calls) {
      expect(v).toBeGreaterThanOrEqual(base.min);
      expect(v).toBeLessThanOrEqual(base.max);
    }
  });

  it("renders no switch when the row is not meant to be toggled", () => {
    render(<SliderInput {...base} enabled onChange={() => {}} />);
    expect(screen.queryByRole("switch")).toBeNull();
  });
});
