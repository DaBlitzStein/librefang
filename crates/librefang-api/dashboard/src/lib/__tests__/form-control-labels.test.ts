import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { join, relative } from "node:path";

// A control that draws its own label must not sit inside a `Field` that draws
// the same one. The operator sees the text twice, stacked, in two type scales —
// which is what the Límites y coste group shipped: a small-caps `LLM TOKENS /
// HOUR` from `Field` and the ladder's own `LLM Tokens / Hour` right underneath.
//
// The two halves are invisible to every other kind of test here. The controls
// render correctly in isolation (their own unit tests pass, because the label
// they draw is the one being asserted), the page renders without error, and no
// type is wrong — it takes a human looking at the pixels, or this scan.
//
// Scanned rather than rendered for the reason `manifest-field-coverage.test.ts`
// gives for reading the Rust struct: the manifest form has no render harness
// (some twenty hooks), so the rule has to be checked against the source.

const SRC = join(__dirname, "..", "..");

/**
 * Controls that render their own visible label, so wrapping them in a `Field`
 * with the same text draws it twice.
 *
 * The distinction is not "takes a `label` prop" — `ModelPicker` takes one and
 * uses it as an `aria-label` with no visible text, which is exactly right
 * inside a `Field`. It is "puts that text on screen".
 */
const DRAWS_ITS_OWN_LABEL = ["StepLadderInput"];

function sourceFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(join(SRC, dir), { withFileTypes: true })) {
    const rel = `${dir}/${entry.name}`;
    if (entry.isDirectory()) sourceFiles(rel, out);
    else if (entry.name.endsWith(".tsx") && !entry.name.includes(".test.")) {
      out.push(join(SRC, rel));
    }
  }
  return out;
}

/**
 * `<Field label={X}>` immediately wrapping `<Control label={X}>`, for a control
 * that draws its own label. Whitespace and attribute order are incidental; the
 * repeated expression is the whole signal.
 */
function doubledLabels(source: string): string[] {
  const found: string[] = [];
  const opener = /<Field\s+([^>]*?)>\s*<([A-Z]\w*)\s+([^>]*?)>/gs;
  for (const match of source.matchAll(opener)) {
    const [, fieldAttrs, control, controlAttrs] = match;
    if (!DRAWS_ITS_OWN_LABEL.includes(control)) continue;
    const outer = /label=\{([^}]*)\}/.exec(fieldAttrs)?.[1];
    const inner = /label=\{([^}]*)\}/.exec(controlAttrs)?.[1];
    if (outer && inner && outer === inner) found.push(`${control} ← ${outer}`);
  }
  return found;
}

describe("form control labels", () => {
  // The list above is a claim about someone else's source, and a stale claim
  // fails open: if `StepLadderInput` stopped drawing its label, every hit this
  // scan reports would be a false positive and the scan would be noise people
  // learn to ignore. So the claim is checked before it is used.
  it.each(DRAWS_ITS_OWN_LABEL)("%s really does render its label", (control) => {
    const source = readFileSync(join(SRC, "components", "ui", `${control}.tsx`), "utf8");
    const renders =
      new RegExp(`>\\s*\\{label\\}`, "s").test(source) ||
      new RegExp(`\\{label\\}\\s*<`, "s").test(source);
    expect(
      renders,
      `${control} is listed as drawing its own label, but its source no longer ` +
        `renders \`{label}\`. Either it stopped drawing one — in which case ` +
        `remove it from DRAWS_ITS_OWN_LABEL and the hits below are false — or ` +
        `it moved, and the scan needs to follow it.`,
    ).toBe(true);
  });

  it("never wraps a self-labelling control in a Field with the same label", () => {
    const offenders = sourceFiles("").flatMap((file) =>
      doubledLabels(readFileSync(file, "utf8")).map(
        (hit) => `${relative(SRC, file)}: ${hit}`,
      ),
    );

    expect(
      offenders,
      `These controls draw their own label and are wrapped in a Field that ` +
        `draws the same text again, so the operator reads every field name ` +
        `twice. Drop the Field wrapper and keep the control's own label, or ` +
        `give the two different text if both are meant to be visible.\n\n` +
        `Doubled (${offenders.length}):\n${offenders.join("\n")}`,
    ).toEqual([]);
  });
});
