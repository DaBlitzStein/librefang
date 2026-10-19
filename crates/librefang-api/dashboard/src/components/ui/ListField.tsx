import { useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";

/**
 * Free-form list editor: a row of removable chips plus a text box.
 *
 * A value is committed on Enter, on comma, or when the field loses focus.
 * Pasting text splits it on commas, semicolons and newlines so a
 * comma-separated list lands as individual chips. Duplicates are dropped
 * while preserving the order they were entered in.
 */
export function ListField({
  value,
  onChange,
  placeholder,
  disabled = false,
  id,
  ariaLabel,
}: {
  value: string[];
  onChange: (next: string[]) => void;
  placeholder?: string;
  disabled?: boolean;
  id?: string;
  ariaLabel?: string;
}) {
  const { t } = useTranslation();
  const [inputValue, setInputValue] = useState("");

  const commit = (raw: string): void => {
    const cleaned = splitList(raw);
    if (cleaned.length === 0) return;
    setInputValue("");
    const next = [...value];
    for (const item of cleaned) {
      if (!next.includes(item)) next.push(item);
    }
    if (next.length !== value.length) onChange(next);
  };

  return (
    <div
      className={`flex flex-wrap items-center gap-1.5 rounded-lg border border-border-subtle bg-main px-2 py-1.5 focus-within:border-brand ${
        disabled ? "opacity-60" : ""
      }`}
    >
      {value.map((item) => (
        <span
          key={item}
          className="inline-flex items-center gap-1 rounded-md bg-surface px-1.5 py-0.5 text-[11px] font-medium text-text"
        >
          {item}
          {!disabled && (
            <button
              type="button"
              onClick={() => onChange(value.filter((v) => v !== item))}
              className="text-text-dim hover:text-error"
              aria-label={`${t("common.remove")} ${item}`}
            >
              <X className="h-3 w-3" />
            </button>
          )}
        </span>
      ))}
      <input
        id={id}
        type="text"
        value={inputValue}
        disabled={disabled}
        aria-label={ariaLabel}
        onChange={(e) => setInputValue(e.target.value)}
        placeholder={value.length === 0 ? placeholder : undefined}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === ",") {
            e.preventDefault();
            commit(inputValue);
          } else if (e.key === "Backspace" && !inputValue && value.length > 0) {
            onChange(value.slice(0, -1));
          }
        }}
        onPaste={(e) => {
          const pasted = e.clipboardData.getData("text");
          if (!/[,\n;]/.test(pasted)) return;
          e.preventDefault();
          commit(inputValue + pasted);
        }}
        onBlur={() => {
          if (inputValue) {
            commit(inputValue);
          }
        }}
        className="flex-1 min-w-[100px] bg-transparent text-xs outline-none placeholder:text-text-dim/40 disabled:cursor-not-allowed"
      />
    </div>
  );
}

/** Split a free-form list on commas, semicolons and newlines, dropping blanks. */
function splitList(raw: string): string[] {
  return raw
    .split(/[,;\n]+/)
    .map((item) => item.trim())
    .filter(Boolean);
}
