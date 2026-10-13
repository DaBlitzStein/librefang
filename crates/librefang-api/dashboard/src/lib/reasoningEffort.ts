/**
 * Reasoning effort levels — the shared vocabulary between the composer menu,
 * the agent editor and the wire.
 *
 * The wire spelling is the product's: `"none" | "low" | "medium" | "high" |
 * "xhigh" | "max"`. A model may accept a narrower (or wider — `minimal`) set;
 * the catalog declares it as `reasoning_efforts` on the model entry. Nothing
 * in the UI hard-codes a level menu of its own: it asks the model, and falls
 * back to the canonical six only when the catalog has no answer, because an
 * empty menu would read as "reasoning does not exist" rather than "nobody
 * said". Unrecognized levels stay reachable for the same reason — the kernel's
 * vocabulary can grow ahead of the dashboard.
 */

/** The fallback set, and the levels every agreed surface understands. */
export const CANONICAL_REASONING_EFFORTS = [
  "none",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
] as const;

/**
 * Technical names, deliberately not translated: the level a model documents is
 * the level the API takes, and an operator debugging a request wants to see
 * the string they sent. `xHigh` is the one casing the providers themselves use.
 */
const LEVEL_LABELS: Record<string, string> = {
  none: "None",
  minimal: "Minimal",
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "xHigh",
  max: "Max",
};

/** Display label for a level: the technical name, or the raw value capitalized. */
export function reasoningEffortLabel(level: string): string {
  const trimmed = level.trim();
  const known = LEVEL_LABELS[trimmed.toLowerCase()];
  if (known) return known;
  if (!trimmed) return trimmed;
  return trimmed.charAt(0).toUpperCase() + trimmed.slice(1);
}

/** The subset of a catalog entry this module reads. */
export interface ReasoningEffortSource {
  provider?: string;
  id?: string;
  /** Levels the model accepts. Absent = the canonical set applies. */
  reasoning_efforts?: string[];
}

/**
 * The levels a model accepts.
 *
 * Absent, empty, or cleaned-to-empty all mean "nobody said": the canonical set
 * applies. A declared list wins as given, minus blanks and duplicates, in the
 * declared order.
 */
export function resolveReasoningEfforts(
  model?: ReasoningEffortSource | null,
): string[] {
  const declared = model?.reasoning_efforts;
  if (Array.isArray(declared)) {
    const cleaned: string[] = [];
    for (const level of declared) {
      if (typeof level !== "string") continue;
      const trimmed = level.trim();
      if (!trimmed || cleaned.includes(trimmed)) continue;
      cleaned.push(trimmed);
    }
    if (cleaned.length > 0) return cleaned;
  }
  return [...CANONICAL_REASONING_EFFORTS];
}

/**
 * The levels for the model an agent runs on, resolved from the catalog.
 *
 * Matches on provider and id together first — the same id can exist under two
 * providers with different level sets — then on id alone, for the callers that
 * know the model name but not where it is served from. Both may be spelled
 * `provider/model` in manifests, so that form matches either field.
 */
export function modelReasoningEfforts(
  models: readonly ReasoningEffortSource[],
  model: string,
  provider: string,
): string[] {
  const name = model?.trim() ?? "";
  const vendor = provider?.trim() ?? "";
  if (!name) return resolveReasoningEfforts(undefined);

  const bareName = name.includes("/") ? name.slice(name.lastIndexOf("/") + 1) : name;
  const match =
    models.find((m) => m.id === name && (!vendor || m.provider === vendor)) ??
    models.find((m) => m.id === bareName && (!vendor || m.provider === vendor)) ??
    models.find((m) => `${m.provider}/${m.id}` === name) ??
    models.find((m) => m.id === bareName);
  return resolveReasoningEfforts(match);
}
