import { memo, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useUIStore } from "../../lib/store";

/**
 * Renders a ```mermaid fenced block as a diagram.
 *
 * Three things shape this component:
 *
 * - **Mermaid is loaded on demand.** It is the largest single dependency in the
 *   dashboard, and most sessions never see a diagram, so it is behind a dynamic
 *   `import()` that Vite splits into its own chunk. The promise is cached at
 *   module scope so a conversation with twenty diagrams parses the library once.
 * - **A diagram that does not parse falls back to its source.** Agents emit
 *   Mermaid that is wrong, truncated, or a dialect this version does not know;
 *   none of that is a reason to show the operator an error where a code block
 *   would do. The streaming view never reaches here at all — `Typewriter_v2`
 *   renders its own `<pre>` — so a half-written diagram is shown as text while
 *   it arrives and becomes a diagram once the turn settles.
 * - **`securityLevel: "strict"`** keeps Mermaid from emitting raw HTML in node
 *   labels or wiring `click` directives to scripts. Chat content is model
 *   output, which is untrusted input by definition.
 */

type MermaidApi = typeof import("mermaid")["default"];

let mermaidPromise: Promise<MermaidApi> | null = null;

function loadMermaid(): Promise<MermaidApi> {
  mermaidPromise ??= import("mermaid").then((mod) => mod.default);
  return mermaidPromise;
}

// `mermaid.render` needs an id that is unique per document, not per component:
// it parks a measuring element in the DOM under that id while it lays the
// diagram out, and two concurrent renders sharing an id clobber each other.
// A module-global counter is what makes it document-wide; a per-instance one
// would collide across components by construction.
let seq = 0;

/**
 * Apply the same link policy to a diagram's anchors as to markdown links.
 *
 * Mermaid emits a `click X "url" _blank` directive as
 * `<a xlink:href="…" target="_blank">` and never sets `rel`, so a diagram node
 * would be the one link on the page opting out of the `noopener noreferrer`
 * that `MarkdownContent` gives every other link — and it looks like a diagram
 * node, not like a link.
 *
 * Anchors mermaid did not emit cannot be here: `DOMPurify` has already run over
 * the whole SVG inside `mermaid.render`.
 */
function withLinkPolicy(svg: string): string {
  return svg.replace(/<a\b(?![^>]*\brel=)/gi, '<a rel="noopener noreferrer"');
}

interface MermaidDiagramProps {
  /** The fenced block's contents, verbatim. */
  source: string;
}

export const MermaidDiagram = memo(function MermaidDiagram({ source }: MermaidDiagramProps) {
  const { t } = useTranslation();
  const theme = useUIStore((s) => s.theme);
  const [svg, setSvg] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  // Guards against a late render landing after the component unmounted or the
  // source changed — chat re-renders on every streamed token upstream of here.
  const renderToken = useRef(0);

  useEffect(() => {
    const token = ++renderToken.current;
    let cancelled = false;
    setFailed(false);

    loadMermaid()
      .then(async (mermaid) => {
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: "strict",
          theme: theme === "light" ? "default" : "dark",
          fontFamily: "inherit",
          // Pinned, and pinned *through* `secure`, because `layout` is not one
          // of the keys mermaid protects from an `%%{init: …}%%` directive. A
          // single line of diagram source — which may have come from a web page
          // the agent read — would otherwise pull the 1.4 MB ELK layout engine.
          layout: "dagre",
          secure: [
            "secure",
            "securityLevel",
            "startOnLoad",
            "maxTextSize",
            "suppressErrorRendering",
            "maxEdges",
            "layout",
          ],
        });
        const { svg: rendered } = await mermaid.render(`mermaid-${seq++}`, source);
        if (cancelled || token !== renderToken.current) return;
        // An empty or absent SVG is a failure, not a diagram: mounting it would
        // give an empty box announced as a diagram, with the source gone.
        if (rendered) setSvg(withLinkPolicy(rendered));
        else setFailed(true);
      })
      .catch(() => {
        if (!cancelled && token === renderToken.current) {
          setSvg(null);
          setFailed(true);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [source, theme]);

  if (failed || !svg) {
    return (
      <pre
        className="p-2 rounded-lg bg-main font-mono text-[11px] overflow-x-auto mb-1.5"
        data-testid="mermaid-source"
        aria-label={failed ? t("chat.mermaid_unrenderable", { defaultValue: "Diagram source (could not be rendered)" }) : undefined}
      >
        <code>{source}</code>
      </pre>
    );
  }

  return (
    <figure className="mb-1.5">
      <div
        className="overflow-x-auto rounded-lg bg-main p-2 [&>svg]:max-w-full [&>svg]:h-auto"
        data-testid="mermaid-diagram"
        // Mermaid returns an SVG string; there is no React tree to hand back.
        // It is generated by Mermaid itself under `securityLevel: "strict"`,
        // which strips raw HTML from labels, drops `javascript:` hrefs and
        // refuses `click` callbacks — verified against mermaid 12 with hostile
        // input, including an `%%{init:{"securityLevel":"loose"}}%%` directive,
        // which mermaid's own `secure` list rejects.
        dangerouslySetInnerHTML={{ __html: svg }}
      />
      {/*
        The source stays reachable. `role="img"` on the container would collapse
        the whole diagram to one node for a screen reader — and every diagram on
        the page would announce the same word — so the SVG keeps mermaid's own
        <title>/<desc>, and this is the way to the text it was drawn from.
      */}
      <details className="mt-1">
        <summary className="cursor-pointer text-[10px] text-text-dim hover:text-text-main">
          {t("chat.mermaid_show_source", { defaultValue: "Diagram source" })}
        </summary>
        <pre className="mt-1 p-2 rounded-lg bg-main font-mono text-[11px] overflow-x-auto">
          <code>{source}</code>
        </pre>
      </details>
    </figure>
  );
});
