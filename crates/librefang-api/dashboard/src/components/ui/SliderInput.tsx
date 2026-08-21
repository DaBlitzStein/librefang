import { useId } from "react";

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
}: SliderInputProps) {
  const id = useId();
  const pct = max === min ? 0 : ((value - min) / (max - min)) * 100;

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
            value={value}
            onChange={(e) => {
              const v = parseFloat(e.target.value);
              if (!isNaN(v)) onChange(Math.min(max, Math.max(min, v)));
            }}
            min={min}
            max={max}
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
              className={`relative w-8 h-[18px] shrink-0 rounded-full transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand ${
                enabled ? "bg-brand" : "bg-text-dim hover:bg-text-dim/80"
              }`}
            >
              <span
                className={`absolute top-0.5 w-3.5 h-3.5 rounded-full bg-white shadow transition-transform ${
                  enabled ? "translate-x-4" : "translate-x-0.5"
                }`}
              />
            </button>
          ) : null}
        </div>
      </div>
      <input
        id={id}
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        disabled={!enabled}
        className={`w-full h-1.5 rounded-full appearance-none cursor-pointer disabled:cursor-not-allowed accent-brand ${dim}`}
        style={{
          background: enabled
            ? `linear-gradient(to right, var(--color-brand) ${pct}%, var(--color-border-subtle) ${pct}%)`
            : undefined,
        }}
      />
      {ticks ? (
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
          {ticks.map((t) => {
            const p = max === min ? 0 : ((t - min) / (max - min)) * 100;
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
              <span
                key={t}
                className={`absolute whitespace-nowrap ${align}`}
                style={{ left: `${clamped}%` }}
              >
                {formatTick ? formatTick(t) : t}
              </span>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
