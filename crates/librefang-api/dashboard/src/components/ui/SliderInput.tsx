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
        <div className={`flex justify-between text-[9px] text-text-dim/50 font-mono px-0.5 ${dim}`}>
          {ticks.map((t) => (
            <span key={t}>{formatTick ? formatTick(t) : t}</span>
          ))}
        </div>
      ) : null}
    </div>
  );
}
