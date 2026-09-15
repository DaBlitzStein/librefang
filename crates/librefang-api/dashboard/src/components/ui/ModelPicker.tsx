import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft, ArrowRight, ChevronDown, Loader2 } from "lucide-react";
import type { ModelItem, ProviderItem } from "../../api";
import { cn } from "../../lib/cn";

/**
 * Pick a provider and a model out of the live catalog.
 *
 * Extracted from the switcher that lived inline in `ChatPage`, which was the
 * only searchable single-select in the dashboard: every other model field in
 * the app is an `<input type="text">` where the operator is expected to know an
 * id by heart. The agent's own model, the three complexity tiers, the fallbacks
 * and the per-modality routes all use this instead, so the same choice looks
 * and behaves the same wherever it is made.
 *
 * Two levels rather than one flat list because the catalog runs to hundreds of
 * ids across dozens of providers and `id` is only unique *within* a provider —
 * the pair is the identity, so the caller gets both back.
 *
 * The search box lives inside, because it is what makes the list navigable: a
 * caller that had to own it would end up re-implementing this component.
 *
 * Strings come from the existing `chat.*` keys rather than a fresh
 * `modelPicker.*` set. They are literally the same sentences as the chat
 * switcher's, and every locale already carries them — a new namespace would
 * have to be translated into all of them before it could render as anything but
 * a raw key.
 */
export interface ModelPickerValue {
  provider: string;
  model: string;
}

export interface ModelPickerProps {
  /** The current pair, or `null` when nothing is chosen yet. */
  value: ModelPickerValue | null;
  onChange: (next: ModelPickerValue) => void;
  /** The catalog to choose from. The caller decides what is filtered out. */
  models: ModelItem[];
  /**
   * Providers for the first level. When omitted, the level is derived from
   * `models`, so a caller with no provider list still gets a working picker.
   */
  providers?: ProviderItem[];
  disabled?: boolean;
  /** A write is in flight: the list is frozen and the active row spins. */
  busy?: boolean;
  /** True while the catalog is still arriving. */
  isFetching?: boolean;
  /** A catalog fetch failure to show, with its retry. */
  error?: string | null;
  onRetry?: () => void;
  /** What this control chooses, e.g. "Agent model". Used as its accessible name. */
  label: string;
  /** Shown on the trigger when `value` is null. */
  placeholder?: string;
  className?: string;
  /** Rendered inside the popover, under the list — for a hint or a warning. */
  footer?: ReactNode;
}

interface ProviderEntry {
  id: string;
  count: number;
  /** The catalog cannot serve this provider right now — shown, not selectable. */
  unavailable?: boolean;
}

export function ModelPicker({
  value,
  onChange,
  models,
  providers,
  disabled = false,
  busy = false,
  isFetching = false,
  error = null,
  onRetry,
  label,
  placeholder,
  className,
  footer,
}: ModelPickerProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [drilldown, setDrilldown] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const rootRef = useRef<HTMLDivElement>(null);

  // A click anywhere else closes it. Not `onBlur`: the trigger and the list are
  // separate nodes, so moving focus between them would close the popover the
  // operator is trying to use.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  // Escape closes, and it closes from anywhere inside — including while the
  // search box has focus, which is where focus lands on open.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open]);

  // Each open starts at the top level, never drilled into the last provider
  // browsed: landing in a provider you did not choose reads as the wrong list
  // rather than a remembered one.
  useEffect(() => {
    if (!open) {
      setDrilldown(null);
      setSearch("");
    }
  }, [open]);

  const entries: ProviderEntry[] = useMemo(() => {
    const counts = new Map<string, number>();
    for (const m of models) counts.set(m.provider, (counts.get(m.provider) ?? 0) + 1);
    const list: ProviderEntry[] =
      providers && providers.length
        ? providers.map((p) => ({
            id: p.id,
            count: counts.get(p.id) ?? p.model_count ?? 0,
            unavailable: p.reachable === false || p.auth_status === "missing",
          }))
        : [...counts.entries()].map(([id, count]) => ({ id, count }));
    // Alphabetical, matching the chat switcher — a list that reorders itself
    // between the two places the same provider appears is the thing that makes
    // an operator hunt for a row that "moved".
    return list.sort((a, b) => a.id.localeCompare(b.id));
  }, [models, providers]);

  const filteredProviders = useMemo(() => {
    const q = search.trim().toLowerCase();
    return q ? entries.filter((p) => p.id.toLowerCase().includes(q)) : entries;
  }, [entries, search]);

  const filteredModels = useMemo(() => {
    const q = search.trim().toLowerCase();
    const inProvider = models.filter((m) => m.provider === drilldown);
    if (!q) return inProvider;
    return inProvider.filter(
      (m) =>
        m.id.toLowerCase().includes(q) || (m.display_name ?? "").toLowerCase().includes(q),
    );
  }, [models, drilldown, search]);

  const trigger = value?.model ? `${value.provider} / ${value.model}` : (placeholder ?? "");

  return (
    <div ref={rootRef} className={cn("relative", className)}>
      <button
        type="button"
        disabled={disabled}
        // Composed, not replaced: naming the control "Agent model" alone would
        // hide the current selection from anyone not looking at the screen.
        aria-label={`${label}: ${trigger || t("common.none", { defaultValue: "None" })}`}
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={() => setOpen((o) => !o)}
        className={cn(
          "flex w-full items-center justify-between gap-2 rounded-xl border border-border-subtle bg-main px-3 py-2 text-left text-sm",
          "focus:border-brand focus:outline-none disabled:cursor-not-allowed disabled:opacity-50",
        )}
      >
        <span className={cn("truncate", !value?.model && "text-text-dim")}>
          {trigger || t("common.none", { defaultValue: "None" })}
        </span>
        <ChevronDown className={cn("h-3 w-3 shrink-0 transition-transform", open && "rotate-180")} />
      </button>

      {open && (
        <div
          aria-label={label}
          className="absolute left-0 top-full z-50 mt-1 w-80 overflow-hidden rounded-xl border border-border-subtle bg-surface shadow-xl"
        >
          <div className="flex items-center gap-2 border-b border-border-subtle/50 p-2">
            {drilldown && (
              <button
                type="button"
                aria-label={t("common.back", { defaultValue: "Back" })}
                onClick={() => {
                  setDrilldown(null);
                  setSearch("");
                }}
                className="rounded p-0.5 transition-colors hover:bg-surface-hover"
              >
                <ArrowLeft className="h-3.5 w-3.5 text-text-dim" />
              </button>
            )}
            <span className="px-1 text-[10px] font-semibold uppercase tracking-wider text-text-dim/50">
              {drilldown ?? t("chat.select_provider", { defaultValue: "Select Provider" })}
            </span>
          </div>

          <div className="border-b border-border-subtle/50 p-2">
            <input
              autoFocus
              type="text"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              aria-label={
                drilldown
                  ? t("chat.search_models", { defaultValue: "Search models" })
                  : t("chat.search_providers", { defaultValue: "Search providers" })
              }
              placeholder={
                drilldown
                  ? t("chat.search_models", { defaultValue: "Search models..." })
                  : t("chat.search_providers", { defaultValue: "Search providers..." })
              }
              className="w-full rounded-lg border border-border-subtle bg-main px-2.5 py-1.5 text-xs focus:border-brand focus:outline-none"
            />
            {error && <p className="mt-1.5 px-1 text-[10px] text-error">{error}</p>}
          </div>

          <div
            className={cn(
              "max-h-64 space-y-0.5 overflow-y-auto p-1.5",
              busy && "pointer-events-none opacity-60",
            )}
          >
            {isFetching && (
              <div className="flex items-center gap-2 px-2.5 py-2 text-xs text-text-dim">
                <Loader2 className="h-3 w-3 animate-spin" />
                {t("chat.loading_models", { defaultValue: "Loading models..." })}
              </div>
            )}
            {!isFetching && error && onRetry && (
              <button
                type="button"
                onClick={onRetry}
                className="px-2.5 py-1 text-[10px] text-brand hover:underline"
              >
                {t("chat.retry", { defaultValue: "Retry" })}
              </button>
            )}

            {!isFetching && !drilldown && filteredProviders.length === 0 && (
              <p className="px-2.5 py-2 text-xs text-text-dim">
                {t("chat.no_models_found", { defaultValue: "No models found" })}
              </p>
            )}
            {!isFetching &&
              !drilldown &&
              filteredProviders.map((p) => {
                const isCurrent = p.id === value?.provider;
                return (
                  <button
                    key={p.id}
                    type="button"
                    disabled={p.unavailable}
                    // The count is part of the visible text; naming the row
                    // exactly by its provider keeps "openai 12" from being read
                    // out as the provider's name.
                    aria-label={p.id}
                    aria-current={isCurrent ? "true" : undefined}
                    onClick={() => {
                      setDrilldown(p.id);
                      setSearch("");
                    }}
                    className={cn(
                      "flex w-full items-center justify-between rounded-lg px-2.5 py-2 text-left transition-colors",
                      p.unavailable
                        ? "cursor-not-allowed text-text-dim/40"
                        : isCurrent
                          ? "bg-brand/10 text-brand"
                          : "text-text-dim hover:bg-surface-hover",
                    )}
                  >
                    <span className="flex items-center gap-2">
                      {isCurrent && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-success" />}
                      <span className="text-xs font-medium">{p.id}</span>
                    </span>
                    <span className="flex items-center gap-1.5">
                      {/*
                        The count alone, where the chat switcher spells out
                        "N models" in English — a hardcoded string in a UI with
                        five locales. Reading as a count next to a chevron is
                        unambiguous, and it adds no untranslated surface.
                      */}
                      <span className="text-[10px] text-text-dim/40">{p.count}</span>
                      <ArrowRight className="h-3 w-3 text-text-dim/30" />
                    </span>
                  </button>
                );
              })}

            {!isFetching && drilldown && filteredModels.length === 0 && (
              <p className="px-2.5 py-2 text-xs text-text-dim">
                {t("chat.no_models_found", { defaultValue: "No models found" })}
              </p>
            )}
            {!isFetching &&
              drilldown &&
              filteredModels.map((m) => {
                const isActive = m.id === value?.model && m.provider === value?.provider;
                return (
                  <button
                    key={`${m.provider}/${m.id}`}
                    type="button"
                    // Qualified by provider: the same model id under two
                    // providers is two different rows, and an ambiguous name
                    // would make them indistinguishable to a screen reader too.
                    aria-label={`${m.provider}/${m.id}`}
                    aria-current={isActive ? "true" : undefined}
                    onClick={() => {
                      if (isActive) return;
                      onChange({ provider: m.provider, model: m.id });
                      setOpen(false);
                    }}
                    className={cn(
                      "flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left transition-colors",
                      isActive ? "bg-brand/10 text-brand" : "text-text-dim hover:bg-surface-hover",
                    )}
                  >
                    {isActive && busy ? (
                      <Loader2 className="h-3 w-3 shrink-0 animate-spin" />
                    ) : (
                      isActive && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-success" />
                    )}
                    <span className="truncate text-xs font-medium">{m.display_name || m.id}</span>
                  </button>
                );
              })}
          </div>

          {footer && <div className="border-t border-border-subtle/50 p-2">{footer}</div>}
        </div>
      )}
    </div>
  );
}
