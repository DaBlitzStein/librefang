// Patch-builder for the agent "model" inline edit form on AgentsPage.
//
// `max_tokens` / `temperature` are tri-state, and the empty string is the third state: it means
// "this agent has no opinion", so the per-model override supplies the value and, failing that, the
// system default.
// The form seeds an empty field from a `null` on the wire and sends `null` back to clear one.
//
// This used to seed the draft with the compiled kernel defaults (4096 / 0.7) and compare against
// the same baseline, so a provider-only edit would not silently PATCH those numbers into an agent
// the user never touched (#5917).
// That was a workaround for a type with no inherit state: every agent carried a concrete number, so
// "unset" had to be simulated by matching against the default.
// With the field genuinely nullable the workaround is gone — an untouched field stays empty, and
// an emptied field is a deliberate "hand this back to the model's setting" that reaches the
// backend as `null` instead of being silently indistinguishable from no edit at all.

/**
 * The numeric fields, with the range `PATCH /api/agents/{id}/config` accepts
 * for each.
 *
 * One table rather than five copies of the same three lines, and it is the
 * single place the client's idea of a valid range lives. The bounds mirror the
 * doc comments on `AgentConfigPatch` in `routes/agents/config.rs`: sending a
 * value outside them is a 400, so catching it here is the difference between a
 * disabled Save and a failed request.
 */
const NUMERIC_FIELDS = {
  max_tokens: { min: 1, max: Number.POSITIVE_INFINITY, integer: true },
  temperature: { min: 0, max: 2, integer: false },
  top_p: { min: 0, max: 1, integer: false },
  frequency_penalty: { min: -2, max: 2, integer: false },
  presence_penalty: { min: -2, max: 2, integer: false },
  context_window: { min: 1, max: Number.POSITIVE_INFINITY, integer: true },
  max_output_tokens: { min: 1, max: Number.POSITIVE_INFINITY, integer: true },
} as const;

export type ModelNumericField = keyof typeof NUMERIC_FIELDS;

export const MODEL_NUMERIC_FIELDS = Object.keys(NUMERIC_FIELDS) as ModelNumericField[];

export interface PersistedModel {
  provider?: string;
  model?: string;
  /** `null` / absent means the agent inherits rather than pinning a number. */
  max_tokens?: number | null;
  temperature?: number | null;
  top_p?: number | null;
  frequency_penalty?: number | null;
  presence_penalty?: number | null;
  context_window?: number | null;
  max_output_tokens?: number | null;
}

export interface ModelDraft {
  provider: string;
  model: string;
  /** `""` is the inherit state, not zero. */
  max_tokens: string;
  temperature: string;
  top_p: string;
  frequency_penalty: string;
  presence_penalty: string;
  context_window: string;
  max_output_tokens: string;
}

export interface ModelConfigPatch {
  provider?: string;
  model?: string;
  /** `null` clears the agent's own value. */
  max_tokens?: number | null;
  temperature?: number | null;
  top_p?: number | null;
  frequency_penalty?: number | null;
  presence_penalty?: number | null;
  context_window?: number | null;
  max_output_tokens?: number | null;
}

/** Every numeric field in its inherit state — the shape a fresh draft starts in. */
export function emptyModelNumerics(): Pick<ModelDraft, ModelNumericField> {
  return Object.fromEntries(MODEL_NUMERIC_FIELDS.map((f) => [f, ""])) as Pick<
    ModelDraft,
    ModelNumericField
  >;
}

/**
 * Seed the numeric half of a draft from what the daemon returned.
 *
 * A `null` on the wire becomes `""`, not the compiled default: an untouched
 * field has to look untouched, or opening the drawer and saving would pin every
 * inherited value as a deliberate choice (#5917).
 */
export function seedModelNumerics(
  persisted: PersistedModel | undefined,
): Pick<ModelDraft, ModelNumericField> {
  return Object.fromEntries(
    MODEL_NUMERIC_FIELDS.map((f) => [f, persisted?.[f] == null ? "" : String(persisted[f])]),
  ) as Pick<ModelDraft, ModelNumericField>;
}

export interface BuildModelConfigPatchResult {
  /** null when the draft fails validation (caller should not submit). */
  patch: ModelConfigPatch | null;
}

/**
 * Parse a tri-state numeric draft field.
 *
 * Returns `null` for the inherit state, a number for a pinned value, and
 * `undefined` when the text is not a number this field accepts — which the
 * caller treats as an invalid draft.
 */
function parseTriState(
  raw: string,
  parse: (s: string) => number,
  valid: (n: number) => boolean,
): number | null | undefined {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  if (Number.isNaN(Number(trimmed))) return undefined;
  const parsed = parse(trimmed);
  return Number.isNaN(parsed) || !valid(parsed) ? undefined : parsed;
}

// Build the PATCH payload from the draft, including a field only when the user
// actually changed it. Returns `{ patch: null }` when the draft is invalid so
// the caller can bail without re-implementing the validation.
export function buildModelConfigPatch(
  draft: ModelDraft,
  persisted: PersistedModel | undefined,
): BuildModelConfigPatchResult {
  const trimmedProvider = draft.provider.trim();
  const trimmedModel = draft.model.trim();
  if (!trimmedProvider || !trimmedModel) return { patch: null };

  const parsed = {} as Record<ModelNumericField, number | null>;
  for (const field of MODEL_NUMERIC_FIELDS) {
    const { min, max, integer } = NUMERIC_FIELDS[field];
    const value = parseTriState(
      draft[field],
      integer ? (s) => parseInt(s, 10) : parseFloat,
      (n) => n >= min && n <= max,
    );
    // One invalid field invalidates the whole draft: a partial PATCH would
    // save some of what the operator typed and silently drop the rest.
    if (value === undefined) return { patch: null };
    parsed[field] = value;
  }

  const patch: ModelConfigPatch = {};

  const persistedModel = persisted?.model?.trim() ?? "";
  const persistedProvider = persisted?.provider?.trim() ?? "";
  const modelChanged = trimmedModel !== persistedModel;
  const providerChanged = trimmedProvider !== persistedProvider;
  if (providerChanged) {
    // PATCH /config applies provider changes only while processing `model`,
    // so a provider edit must carry the current model as its trigger.
    patch.model = trimmedModel;
    patch.provider = trimmedProvider;
  } else if (modelChanged) {
    patch.model = trimmedModel;
  }

  for (const field of MODEL_NUMERIC_FIELDS) {
    const next = parsed[field];
    // `?? null` rather than `|| null`: a persisted explicit `0` is a real
    // value, not an absent one.
    const current = persisted?.[field] ?? null;
    if (next === current) continue;
    // Switching provider already resets these server-side, so a `null` here
    // would be an edit the operator did not make — it is the new provider's
    // inherit state arriving as if it were a deliberate clear.
    if (providerChanged && next === null) continue;
    patch[field] = next;
  }

  return { patch };
}
