// Draft rules for the per-model limit overrides edited on the providers page.
//
// Extracted for the same reason `agentModelPatch.ts` exists: the rule below
// decides both what gets saved and whether Save lights up, and keeping one copy
// is what stops those two from drifting.
//
// The fields edit a **preference** — how long a reply to ask this model for, how
// much context to send it. They are not the model's catalog figures, which the
// provider card reports and which nothing here moves. The one place capacity
// matters is that an absent `context_window` override resolves *to* the catalog
// figure, while an absent `max_tokens` override does not; `resolveLimitDraft`
// takes `catalogValue` for exactly that case and for no other.

export interface LimitDraft {
  /** What to persist: a number sets the override, `null` clears it. */
  value: number | null;
  /** True when the text is not a usable positive whole number. */
  invalid: boolean;
  /** True when saving would change the stored state. */
  dirty: boolean;
}

/**
 * Resolve the draft state for a limit-override field.
 *
 * `input` is the raw text; an empty field means "no preference here", which
 * clears the override and lets the resolution chain supply a value.
 *
 * `catalogValue` is the figure an *absent* override resolves to for this field,
 * or `undefined` when the chain does not fall through to the catalog. The two
 * fields this editor drives resolve differently, so only one of them passes it:
 *
 * - `max_tokens`: absent falls through to the kernel default,
 *   `DEFAULT_MODEL_MAX_TOKENS` (4096) — not the catalog capacity. A typed value
 *   equal to the catalog figure is therefore a real preference, and treating it
 *   as "the default" silently discarded a deliberate setting and left the model
 *   somewhere the operator never chose. `catalogValue` stays `undefined`, so no
 *   value is ever cleared for matching it.
 * - `context_window`: absent resolves to the catalog figure
 *   (`resolve_context_window` ranks agent manifest → `model_overrides.json` →
 *   `ModelCatalog`). A typed value equal to the catalog is a redundant override
 *   that *pins* the window: a later registry or discovery correction
 *   (131072 → 200000) would be silently shadowed. Passing `catalogValue` here
 *   makes an equality clear the override, so the field keeps following the
 *   catalog unless the operator deliberately picks a different number.
 *
 * Capacity is still not consulted for the dirty/save decision beyond that:
 * the display path (`effective`) keeps its own `catalogValue`.
 */
export function resolveLimitDraft(
  input: string,
  storedOverride: number | undefined,
  catalogValue?: number,
): LimitDraft {
  const trimmed = input.trim();
  const parsed = trimmed === "" ? null : Number(trimmed);
  const invalid = parsed !== null && (!Number.isInteger(parsed) || parsed <= 0);
  // Equal to the figure an absent override would resolve to, so there is
  // nothing to store: clearing it is not discarding a preference, it is
  // declining to pin a value the chain already produces.
  const value =
    parsed != null && catalogValue != null && parsed === catalogValue ? null : parsed;
  return {
    value,
    invalid,
    dirty: value !== (storedOverride ?? null),
  };
}
