import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
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

  /**
   * The knob used to be absolutely positioned inside a 32px track and nudged
   * with translate values that had to be re-derived by hand whenever a size
   * changed; the user saw it spill past the right edge of its own track.
   * The track now carries the inset as padding, so containment is arithmetic
   * anyone can check: 36px track - 4px padding = 32px usable, 16px knob,
   * 16px travel. These pin that relationship.
   */
  describe("toggle geometry", () => {
    it("moves the knob by exactly the room the track leaves it", () => {
      const { rerender } = render(
        <SliderInput {...base} enabled={false} onChange={() => {}} onToggle={() => {}} />,
      );
      const knobOf = () =>
        screen.getByRole("switch").firstElementChild as HTMLElement;

      // Off: flush against the padding, no negative or fractional offset.
      expect(knobOf().className).toMatch(/(^|\s)translate-x-0(\s|$)/);

      rerender(<SliderInput {...base} enabled onChange={() => {}} onToggle={() => {}} />);
      // On: 32px usable - 16px knob = 16px = translate-x-4. Any larger value
      // pushes the knob past the track's right edge.
      expect(knobOf().className).toMatch(/(^|\s)translate-x-4(\s|$)/);
    });

    it("keeps track and knob sizes in the ratio the travel assumes", () => {
      render(<SliderInput {...base} enabled onChange={() => {}} onToggle={() => {}} />);
      const toggle = screen.getByRole("switch");

      // w-9 (36px) with p-0.5 (2px each side) and a w-4 (16px) knob.
      expect(toggle.className).toMatch(/(^|\s)w-9(\s|$)/);
      expect(toggle.className).toMatch(/(^|\s)p-0\.5(\s|$)/);
      expect((toggle.firstElementChild as HTMLElement).className).toMatch(/(^|\s)w-4(\s|$)/);
    });
  });

  /**
   * The legend is a reading aid for the track above it, so a label has to sit
   * over the position its own value occupies. Even spacing (`justify-between`)
   * looks tidy and lies: on the real context-window row — min 1024, max 2M,
   * ticks 32K/128K/512K/1M — it drew "1M" hard right when 1M is the midpoint,
   * and "128K" a third of the way across when its true position is 6%.
   */
  describe("tick legend", () => {
    const ladder = {
      label: "Context window",
      value: 131072,
      min: 1024,
      max: 2097152,
      ticks: [32768, 131072, 524288, 1048576],
      formatTick: (v: number) =>
        v >= 1048576 ? `${Math.round(v / 1048576)}M` : `${Math.round(v / 1024)}K`,
    };

    const leftOf = (text: string) =>
      parseFloat((screen.getByText(text) as HTMLElement).style.left);

    it("places each label at the position its value maps to", () => {
      render(<SliderInput {...ladder} enabled onChange={() => {}} />);

      // (value - min) / (max - min), the same formula the filled track uses.
      expect(leftOf("32K")).toBeCloseTo(1.51, 1);
      expect(leftOf("128K")).toBeCloseTo(6.2, 1);
      expect(leftOf("512K")).toBeCloseTo(24.96, 1);
      expect(leftOf("1M")).toBeCloseTo(49.97, 1);
    });

    it("does not space labels evenly regardless of value", () => {
      render(<SliderInput {...ladder} enabled onChange={() => {}} />);

      // The exact regression: evenly spaced would be 0 / 33 / 66 / 100.
      expect(leftOf("1M")).toBeLessThan(60);
      expect(leftOf("128K")).toBeLessThan(20);
    });

    it("keeps end labels inside the track", () => {
      render(
        <SliderInput
          {...ladder}
          ticks={[1024, 2097152]}
          formatTick={(v) => String(v)}
          enabled
          onChange={() => {}}
        />,
      );

      // Centring a label on 0% or 100% would hang half the text off the edge.
      expect(screen.getByText("1024").className).toMatch(/translate-x-0/);
      expect(screen.getByText("2097152").className).toMatch(/-translate-x-full/);
    });
  });
});

/**
 * A context window is not a continuous quantity you dial in: it is one of a
 * handful of sizes models actually come in. The free slider it used to be
 * ranged 1K–2M in 1K increments, which invites 1,234,567 (no model has that)
 * and makes the real sizes nearly impossible to land on — 128K sits at 6% of
 * the track. `steps` snaps travel to the listed sizes, with one position past
 * the end for Custom.
 */
describe("SliderInput — fixed steps", () => {
  const K = 1024;
  const sizes = [8 * K, 32 * K, 64 * K, 96 * K, 128 * K, 192 * K, 256 * K, 1024 * K, 2048 * K];
  const stepped = {
    label: "Context window",
    min: 1024,
    max: 2097152,
    steps: sizes,
    formatTick: (v: number) =>
      v >= 1048576 ? `${Math.round(v / 1048576)}M` : `${Math.round(v / 1024)}K`,
  };

  const track = () => screen.getByRole("slider") as HTMLInputElement;

  it("travels by position, not by token", () => {
    render(<SliderInput {...stepped} value={128 * K} enabled onChange={() => {}} />);

    // 9 sizes plus Custom => positions 0..9, one step apart.
    expect(track().min).toBe("0");
    expect(track().max).toBe("9");
    expect(track().step).toBe("1");
    // 128K is the 5th size.
    expect(track().value).toBe("4");
  });

  it("reports the size, never the position", () => {
    const onChange = vi.fn();
    render(<SliderInput {...stepped} value={8 * K} enabled onChange={onChange} />);

    fireEvent.change(track(), { target: { value: "6" } });
    expect(onChange).toHaveBeenCalledWith(256 * K);
  });

  it("cannot land on a size that is not in the list", () => {
    const onChange = vi.fn();
    render(<SliderInput {...stepped} value={8 * K} enabled onChange={onChange} />);

    for (let i = 0; i < sizes.length; i++) {
      fireEvent.change(track(), { target: { value: String(i) } });
    }
    for (const [v] of onChange.mock.calls) {
      expect(sizes).toContain(v);
    }
  });

  it("labels every size and the custom position", () => {
    render(<SliderInput {...stepped} value={128 * K} enabled onChange={() => {}} />);

    for (const label of ["8K", "32K", "64K", "96K", "128K", "192K", "256K", "1M", "2M"]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
    expect(screen.getByText("Custom")).toBeInTheDocument();
  });

  it("treats a value outside the list as custom rather than rounding it", () => {
    // An operator who types 123904 means 123904. Snapping it to 128K would
    // silently change a deliberate number.
    render(<SliderInput {...stepped} value={123904} enabled onChange={() => {}} />);

    expect(track().value).toBe("9");
    expect(screen.getByRole("spinbutton")).toHaveValue(123904);
    expect(screen.getByText(/Type an exact value/)).toBeInTheDocument();
  });

  it("does not snap when the slider is dragged onto Custom", () => {
    const onChange = vi.fn();
    render(<SliderInput {...stepped} value={128 * K} enabled onChange={onChange} />);

    fireEvent.change(track(), { target: { value: "9" } });
    // Custom is not a value — it is an invitation to type one.
    expect(onChange).not.toHaveBeenCalled();
  });

  it("stays continuous when no steps are given", () => {
    render(
      <SliderInput label="Temperature" value={0.7} min={0} max={2} step={0.1} enabled onChange={() => {}} />,
    );

    expect(track().min).toBe("0");
    expect(track().max).toBe("2");
    expect(track().value).toBe("0.7");
    expect(screen.queryByText("Custom")).toBeNull();
  });
});

/**
 * Bounds and value both arrive from a model catalog, so neither can be trusted
 * to be well-formed: a catalog entry can carry min > max, or a value that sits
 * outside the range it declares, or no number at all. Every position the
 * control derives — the attributes it puts on its inputs, the fill percentage,
 * the value it emits back — has to survive that (refs #7444).
 */
describe("SliderInput — bounds normalization and clamping", () => {
  it("normalizes inverted bounds for both controls and the track fill", () => {
    render(
      <SliderInput
        label="Temperature"
        value={7}
        min={10}
        max={0}
        onChange={() => {}}
      />,
    );

    const numberInput = screen.getByRole("spinbutton");
    const rangeInput = screen.getByRole("slider");
    expect(numberInput).toHaveAttribute("min", "0");
    expect(numberInput).toHaveAttribute("max", "10");
    expect(rangeInput).toHaveAttribute("min", "0");
    expect(rangeInput).toHaveAttribute("max", "10");
    expect(rangeInput.style.background).toContain("70%");
  });

  it("clamps values from either input path", () => {
    const onChange = vi.fn();
    render(
      <SliderInput label="Temperature" value={5} min={0} max={10} onChange={onChange} />,
    );

    fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "12" } });
    expect(onChange).toHaveBeenLastCalledWith(10);

    fireEvent.change(screen.getByRole("slider"), { target: { value: "-4" } });
    expect(onChange).toHaveBeenLastCalledWith(0);
  });

  it("ignores non-finite number input and renders duplicate ticks", () => {
    const onChange = vi.fn();
    render(
      <SliderInput
        label="Temperature"
        value={5}
        min={0}
        max={10}
        ticks={[5, 5]}
        onChange={onChange}
      />,
    );

    fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "Infinity" } });
    expect(onChange).not.toHaveBeenCalled();
    // A caller may repeat a tick value; the legend must still draw both rather
    // than collapse them into one React child.
    expect(screen.getAllByText("5")).toHaveLength(2);
  });

  describe("inherited (disabled) state", () => {
    const renderInherited = (onToggle = vi.fn()) => {
      render(
        <SliderInput
          label="Temperature"
          value={0.7}
          min={0}
          max={2}
          step={0.1}
          ticks={[0, 1, 2]}
          enabled={false}
          onToggle={onToggle}
          onChange={() => {}}
        />,
      );
      return onToggle;
    };

    it("leaves the switch undimmed and clickable so the row can be activated", () => {
      const onToggle = renderInherited();
      const toggle = screen.getByRole("switch");

      expect(toggle).toBeEnabled();
      expect(toggle).toHaveAttribute("aria-checked", "false");
      // No ancestor may fade the switch either: the row container used to carry
      // the opacity, which dimmed the only control able to leave this state.
      expect(toggle.closest("[class*='opacity-']")).toBeNull();
      expect(toggle.className).not.toMatch(/\bopacity-/);
      // The off track needs a control-weight token, not the divider hairline.
      expect(toggle.className).not.toMatch(/bg-border-subtle/);

      fireEvent.click(toggle);
      expect(onToggle).toHaveBeenCalledWith(true);
    });

    it("still dims the values the row is inheriting", () => {
      renderInherited();

      expect(screen.getByText("Temperature").className).toMatch(/opacity-40/);
      expect(screen.getByRole("spinbutton").className).toMatch(/opacity-40/);
      expect(screen.getByRole("slider").className).toMatch(/opacity-40/);
      expect(screen.getByText("1").parentElement?.className).toMatch(/opacity-40/);
    });

    it("undims the values and checks the switch once the row is active", () => {
      render(
        <SliderInput
          label="Temperature"
          value={0.7}
          min={0}
          max={2}
          step={0.1}
          ticks={[0, 1, 2]}
          enabled
          onToggle={vi.fn()}
          onChange={() => {}}
        />,
      );

      expect(screen.getByRole("switch")).toHaveAttribute("aria-checked", "true");
      expect(screen.getByText("Temperature").className).not.toMatch(/opacity-40/);
      expect(screen.getByRole("spinbutton").className).not.toMatch(/opacity-40/);
      expect(screen.getByRole("slider").className).not.toMatch(/opacity-40/);
    });
  });
});
