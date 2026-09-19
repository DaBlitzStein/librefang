import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown } from "lucide-react";

/**
 * How many fields the subtree rooted at `root` holds.
 *
 * Counts `data-field` markers rather than controls: a preset ladder is one
 * field and eight buttons, so counting `input`/`select`/`textarea` would report
 * twenty where the operator sees three. `ui/Field` marks a wrapped field, and
 * the widgets that are a field in their own right — `StepLadderInput`, a
 * labelled `ModelPicker` or `Toggle`, one row of a repeatable editor — mark
 * their own root.
 *
 * Only the outermost marker of a nested pair is counted. A `Field` inside
 * another `Field`, or a widget inside the `Field` that wraps it, is part of its
 * parent's control — a stepper's custom box, a composite widget's sub-label —
 * not a second field to tick off.
 */
function countFields(root: HTMLElement): number {
  let count = 0;
  for (const field of root.querySelectorAll("[data-field]")) {
    if (!field.parentElement?.closest("[data-field]")) count += 1;
  }
  return count;
}

/**
 * The number of fields the subtree behind `bodyRef` holds, kept in step with
 * the DOM — the one mechanism behind every count the editor shows.
 *
 * Measured rather than declared. The fields are not the fold's direct children
 * — they arrive through the components the form is built from (`TriStateField`,
 * the finders, the model pickers) — so there is no list to take `length` of,
 * and a number each call site had to pass would be one nothing keeps true: it
 * would go stale the first time a field was added, which is the same silent lie
 * as a fold that reads as empty.
 *
 * `useLayoutEffect` rather than `useEffect`, so the first paint already carries
 * the count — a badge that appears a frame after the fold does reads as a
 * flicker.
 *
 * The observer covers fields that arrive after that first paint: a fallback row
 * the operator adds, a catalog that finishes loading. Without it the bar would
 * freeze at its first answer and under-report for the rest of the session.
 *
 * The body is what gets observed, not the `<details>`: the badge sits in the
 * summary, outside the observed subtree, so rendering a new count cannot feed
 * back into the count.
 */
export function useFieldCount() {
  const bodyRef = useRef<HTMLDivElement>(null);
  const [count, setCount] = useState<number | null>(null);

  useLayoutEffect(() => {
    const body = bodyRef.current;
    if (!body) return;
    const sync = (): void => {
      const next = countFields(body);
      // Returning the previous value for an unchanged count is what keeps the
      // observer from looping: React bails out instead of re-rendering, so the
      // badge is not rewritten and no further mutation is produced.
      setCount((previous) => (previous === next ? previous : next));
    };
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(body, { childList: true, subtree: true });
    return () => observer.disconnect();
  }, []);

  return { bodyRef, count };
}

/**
 * A titled section that folds away, for grouping a long form.
 *
 * Native `<details>`/`<summary>` rather than `useState` + `aria-expanded`, which
 * is what the app's other collapsing panels use. The native element brings the
 * keyboard behaviour, the expanded/collapsed state and the click target with it,
 * and the chevron animates from CSS (`group-open:rotate-180`) so there is no
 * per-panel state to keep in sync.
 *
 * `invalid` forces the section open: a validation error the operator cannot see
 * is indistinguishable from no error at all.
 *
 * The summary also carries how many fields the section holds, because a folded
 * section and an empty one look identical otherwise — the operator reads a bar
 * with nothing behind it as "this section has no fields", and concludes the
 * editor is missing them. The count is part of the summary's accessible name on
 * purpose: "Routing 12 fields" is the same fact a sighted operator gets from
 * the badge, and hiding it from the name would leave a screen reader with
 * exactly the empty-looking section this exists to prevent. The unit travels
 * with the number — `config.fields_unit` and its plural forms — so the name is
 * a phrase rather than a title with a digit stuck to it, and so a
 * single-field section reads "1 field" rather than "1 fields".
 *
 * Nothing is drawn at zero. A repeatable editor with no rows yet has no fields
 * to report, and "0 fields" would be the same emptiness this exists to remove,
 * with a number attached to make it look authoritative — while a section whose
 * fields the counter cannot see would claim the same. No badge is the honest
 * answer in both cases, and it is what every section showed before.
 *
 * `AgentManifestForm.AdvancedFields` counts its own body with the same hook and
 * renders the same badge, so a count reads the same wherever the editor folds
 * something away.
 */
export interface CollapsibleSectionProps {
  title: string;
  children: ReactNode;
  defaultOpen?: boolean;
  invalid?: boolean;
  /**
   * Identity of the section as a whole, emitted as `data-section`.
   *
   * `title` is display text and therefore neither stable nor unique — two
   * sections may legitimately share a title with a field inside them — so
   * callers that need to address a section (the manifest editor's tab
   * routing, and the tests that guard it) carry the id here instead.
   */
  sectionId?: string;
}

export function CollapsibleSection({
  title,
  children,
  defaultOpen,
  invalid,
  sectionId,
}: CollapsibleSectionProps) {
  const { t } = useTranslation();
  const { bodyRef, count } = useFieldCount();

  return (
    <details
      data-section={sectionId}
      className="group overflow-hidden rounded-xl border border-border-subtle/60 bg-surface/40"
      open={defaultOpen || invalid}
    >
      <summary
        aria-invalid={invalid || undefined}
        className="flex cursor-pointer list-none items-center justify-between gap-2 p-3 select-none"
      >
        <span
          className={`text-[10px] font-bold uppercase tracking-widest ${
            invalid ? "text-error" : "text-text-dim"
          }`}
        >
          {title}
        </span>{" "}
        {/* The space separates the title from the count in the summary's
            accessible name and in its text content. Flexbox does not render a
            whitespace-only child, so it costs nothing on screen. */}
        <span className="flex shrink-0 items-center gap-2">
          {count !== null && count > 0 && (
            <span className="text-[10px] font-bold tracking-widest text-text-dim/60">
              {t("config.fields_unit", { count })}
            </span>
          )}
          <ChevronDown className="h-4 w-4 text-text-dim transition-transform group-open:rotate-180" />
        </span>
      </summary>
      <div ref={bodyRef} className="space-y-2.5 px-3 pb-3">
        {children}
      </div>
    </details>
  );
}
