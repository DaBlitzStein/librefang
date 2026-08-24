import { useId } from "react";
import { useTranslation } from "react-i18next";

interface SliderInputProps {
  label: string;
  value: number;
  onChange: (v: number) => void;
  min: number;
  max: number;
  step?: number;
  enabled?: boolean;
  onToggle?: (enabled: boolean) => void;
  /** Format function for display ticks */
  formatTick?: (v: number) => string;
  /** Tick positions to display below the slider */
  ticks?: number[];
  /**
   * Fixed values the control snaps to, in ascending order.
   *
   * A context window is not a continuous quantity you dial in: it is one of a
   * handful of sizes a model actually comes in. A free slider over a
   * 1K–2M range invites 1,234,567, which no model has, and makes the sizes
   * that do exist nearly impossible to land on — 128K sits at 6% of the
   * track's width.
   *
   * With `steps`, the slider travels by index: every position is a real size,
   * and one extra position past the end is Custom, which reveals the number
   * field for the case the list does not cover.
   */
  steps?: number[];
}

export function SliderInput({
  label,
  value,
  onChange,
  min,
  max,
  step = 1,
  enabled = true,
  onToggle,
  formatTick,
  ticks,
  steps,
}: SliderInputProps) {
  const { t } = useTranslation();
  const id = useId();

  // Bounds arrive from a model catalog, so they can be inverted or absent, and
  // the value can be outside them or not a number at all (refs #7444). Normalize
  // once, here, so every position, attribute, and emitted value downstream is
  // derived from a range that is known to be well-ordered.
  const lowerBound = Math.min(min, max);
  const upperBound = Math.max(min, max);
  const clamp = (nextValue: number) =>
    Math.min(upperBound, Math.max(lowerBound, nextValue));
  const boundedValue = Number.isFinite(value) ? clamp(value) : lowerBound;
  const emitValue = (rawValue: string) => {
    const nextValue = Number.parseFloat(rawValue);
    if (Number.isFinite(nextValue)) onChange(clamp(nextValue));
  };

  // Index-addressed when `steps` is given: the last position is Custom, so the
  // track runs 0..steps.length inclusive.
  const customIndex = steps ? steps.length : -1;
  const stepIndex = steps
    ? (() => {
        const exact = steps.indexOf(boundedValue);
        // A value that is not one of the fixed sizes is Custom by definition —
        // including one typed into the number field, so the control never
        // silently rounds an operator's deliberate number to the nearest step.
        return exact >= 0 ? exact : customIndex;
      })()
    : 0;
  const isCustom = steps ? stepIndex === customIndex : false;

  const sliderMin = steps ? 0 : lowerBound;
  const sliderMax = steps ? customIndex : upperBound;
  const sliderStep = steps ? 1 : step;
  const sliderValue = steps ? stepIndex : boundedValue;

  const pct =
    sliderMax === sliderMin
      ? 0
      : ((sliderValue - sliderMin) / (sliderMax - sliderMin)) * 100;

  // Dim the *values* of an inactive row, never the switch that reactivates it.
  //
  // This used to be one `opacity-40` on the whole container, which swept up
  // the toggle too. Since a row starts inactive whenever the model carries no
  // override for that field, the default state of the form was every control
  // faded — including the only affordance that could undo the fade, itself
  // drawn in the faintest token available. The form read as broken rather
  // than as opt-in, and the way out was invisible.
  const dim = !enabled ? "opacity-40" : "";

  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between gap-2">
        <label htmlFor={id} className={`text-xs font-bold text-text-dim ${dim}`}>
          {label}
        </label>
        <div className="flex items-center gap-2">
          <input
            type="number"
            value={boundedValue}
            onChange={(e) => emitValue(e.target.value)}
            min={lowerBound}
            max={upperBound}
            step={step}
            disabled={!enabled}
            className={`w-20 rounded-lg border border-border-subtle bg-main px-2 py-1 text-xs text-right font-mono outline-none focus:border-brand disabled:cursor-not-allowed ${dim}`}
          />
          {onToggle ? (
            <button
              type="button"
              role="switch"
              aria-checked={enabled}
              aria-label={label}
              onClick={() => onToggle(!enabled)}
              // `bg-text-dim` for the off state, not `bg-border-subtle`: a
              // switch is an interactive control, so it needs contrast against
              // the surface in both states, not the hairline treatment a
              // divider gets.
              // Geometry that contains the knob by construction rather than by
              // arithmetic that has to be re-derived every time a size changes:
              // the track carries the 2px inset as padding (w-9 = 36px, p-0.5
              // leaves 32px of usable width), the knob is 16px, so its travel
              // is exactly 32 - 16 = 16px = translate-x-4. Off is translate-x-0,
              // flush against the padding. Nothing can spill past either edge.
              className={`flex items-center w-9 h-5 p-0.5 shrink-0 rounded-full transition-colors outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand ${
                enabled ? "bg-brand" : "bg-text-dim hover:bg-text-dim/80"
              }`}
            >
              <span
                className={`w-4 h-4 rounded-full bg-white shadow transition-transform ${
                  enabled ? "translate-x-4" : "translate-x-0"
                }`}
              />
            </button>
          ) : null}
        </div>
      </div>
      <input
        id={id}
        type="range"
        min={sliderMin}
        max={sliderMax}
        step={sliderStep}
        value={sliderValue}
        onChange={(e) => {
          if (!steps) {
            // Same clamp as the number field: a range input can still report an
            // out-of-bounds value when it is driven programmatically.
            emitValue(e.target.value);
            return;
          }
          const raw = Number.parseFloat(e.target.value);
          if (!Number.isFinite(raw)) return;
          // Landing on Custom keeps whatever value is already there rather
          // than snapping: Custom means "I will type it", not a value.
          if (raw >= customIndex) return;
          const index = Math.min(steps.length - 1, Math.max(0, Math.round(raw)));
          onChange(steps[index]);
        }}
        disabled={!enabled}
        className={`w-full h-1.5 rounded-full appearance-none cursor-pointer disabled:cursor-not-allowed accent-brand ${dim}`}
        style={{
          background: enabled
            ? `linear-gradient(to right, var(--color-brand) ${pct}%, var(--color-border-subtle) ${pct}%)`
            : undefined,
        }}
      />
      {isCustom && (
        // Custom is the only position where the number field is the control
        // rather than a readout, so say so instead of leaving the slider
        // parked at the end with no explanation.
        <p className="text-[10px] text-text-dim">
          {t("common.slider_custom_hint", {
            defaultValue: "Type an exact value in the field above.",
          })}
        </p>
      )}
      {steps ? (
        <div className={`relative h-3 text-[9px] text-text-dim/50 font-mono ${dim}`}>
          {[
            ...steps.map((v, i) => ({
              i,
              text: formatTick ? formatTick(v) : String(v),
            })),
            { i: customIndex, text: t("common.custom", { defaultValue: "Custom" }) },
          ].map(({ i, text }) => {
            const p = (i / customIndex) * 100;
            const align =
              p <= 0 ? "translate-x-0" : p >= 100 ? "-translate-x-full" : "-translate-x-1/2";
            return (
              <span
                key={i}
                className={`absolute whitespace-nowrap ${align} ${
                  i === stepIndex ? "text-brand font-bold" : ""
                }`}
                style={{ left: `${p}%` }}
              >
                {text}
              </span>
            );
          })}
        </div>
      ) : ticks ? (
        // Each tick sits at the position its own value maps to, using the same
        // formula as the filled track above — so the legend and the thumb agree.
        //
        // This used to be `flex justify-between`, which spaces labels evenly
        // regardless of what they say. On a range whose ticks are not evenly
        // spaced that is actively misleading: with min=1024 max=2097152 and
        // ticks 32K/128K/512K/1M, the "1M" label was drawn hard right while
        // 1M actually falls at the halfway point, and "128K" sat at a third of
        // the width against a true position of 6%. Reading a value off the
        // legend gave an answer that was wrong by an order of magnitude.
        <div className={`relative h-3 text-[9px] text-text-dim/50 font-mono ${dim}`}>
          {ticks.map((tick, index) => {
            const p =
              upperBound === lowerBound
                ? 0
                : ((tick - lowerBound) / (upperBound - lowerBound)) * 100;
            const clamped = Math.min(100, Math.max(0, p));
            // Centre each label on its mark, except at the ends, where centring
            // would push half the text outside the track.
            const align =
              clamped <= 0
                ? "translate-x-0"
                : clamped >= 100
                  ? "-translate-x-full"
                  : "-translate-x-1/2";
            return (
              // A caller may legitimately repeat a tick value, so the index has
              // to be part of the key — the value alone is not unique.
              <span
                key={`${tick}-${index}`}
                className={`absolute whitespace-nowrap ${align}`}
                style={{ left: `${clamped}%` }}
              >
                {formatTick ? formatTick(tick) : tick}
              </span>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
