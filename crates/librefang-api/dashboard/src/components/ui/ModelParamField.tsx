import { useTranslation } from "react-i18next";
import { StepLadderInput } from "./StepLadderInput";
import { CONTEXT_WINDOW_LADDER, MAX_OUTPUT_TOKENS_LADDER } from "../../lib/modelParamLadders";

/**
 * The model parameters that more than one editor can set.
 *
 * Each is configurable from at least two places — the agent manifest, the
 * model's own settings, and the per-provider override — and each of those used
 * to render it differently: a rung ladder in one, a 1024-step slider from 1 Ki
 * to 2 Mi in another, a bare number box in the third. Same parameter, same
 * units, three controls and three vocabularies.
 */
export type ModelParamName = "context_window" | "max_output_tokens" | "max_tokens";

const LADDERS: Record<ModelParamName, readonly number[]> = {
  context_window: CONTEXT_WINDOW_LADDER,
  // An output cap and a per-request `max_tokens` are the same quantity seen
  // from two sides, so they share rungs.
  max_output_tokens: MAX_OUTPUT_TOKENS_LADDER,
  max_tokens: MAX_OUTPUT_TOKENS_LADDER,
};

const LABEL_KEYS: Record<ModelParamName, string> = {
  context_window: "model_param.context_window",
  max_output_tokens: "model_param.max_output_tokens",
  max_tokens: "model_param.max_tokens",
};

const PLACEHOLDER_KEYS: Record<ModelParamName, string> = {
  context_window: "model_param.context_window_placeholder",
  max_output_tokens: "model_param.max_output_tokens_placeholder",
  max_tokens: "model_param.max_tokens_placeholder",
};

interface ModelParamFieldProps {
  param: ModelParamName;
  /** Form value in tokens. `""` means "no opinion here, inherit". */
  value: string;
  onChange: (next: string) => void;
  /**
   * A ceiling some source vouched for, used to trim rungs the endpoint cannot
   * honour. Leave undefined for a limit that was never measured — an unknown
   * limit is not a ceiling (#7780).
   */
  cap?: number;
  /** Advisory shown under the control, e.g. an over-limit warning. */
  warning?: string;
  /** Explanatory line under the control, for editors that need the context. */
  hint?: string;
  /**
   * Overrides the shared label. Use only where the surrounding page gives the
   * parameter a different meaning — not to rename it for decoration.
   */
  label?: string;
}

/**
 * One parameter, one control, wherever it is configured.
 *
 * This is the object every editor renders for these parameters: it owns the
 * rungs, the wording for "inherit" and "custom", and the placeholder, so the
 * agent editor, the model settings and the per-provider override cannot drift
 * into three different experiences for the same field again.
 *
 * `""` is the inherit state everywhere, which is also what each of the three
 * call sites means by its own idiom — an unset manifest field, an unticked
 * override toggle, a cleared box.
 */
export function ModelParamField({
  param,
  value,
  onChange,
  cap,
  warning,
  hint,
  label,
}: ModelParamFieldProps) {
  const { t } = useTranslation();
  return (
    <div>
      <StepLadderInput
        label={label ?? t(LABEL_KEYS[param])}
        value={value}
        onChange={onChange}
        ladder={LADDERS[param]}
        cap={cap}
        inheritLabel={t("model_param.inherit")}
        customLabel={t("model_param.custom")}
        customPlaceholder={t(PLACEHOLDER_KEYS[param])}
        warning={warning}
      />
      {hint && <p className="mt-1 text-[10px] text-text-dim/70 leading-snug">{hint}</p>}
    </div>
  );
}
