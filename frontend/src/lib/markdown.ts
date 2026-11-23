import bundles from "@arto-app/markdown/bundles/manifest.json";
import runtimeUrl from "@arto-app/markdown/bundles/embed.iife.js?url";
import styleUrl from "@arto-app/markdown/bundles/embed.css?url";
import hljsAllUrl from "@arto-app/markdown/bundles/page-hljs-all.iife.js?url";
import hljsCommonUrl from "@arto-app/markdown/bundles/page-hljs-common.iife.js?url";
import mathUrl from "@arto-app/markdown/bundles/page-math.iife.js?url";
import mermaidUrl from "@arto-app/markdown/bundles/page-mermaid.iife.js?url";
import wasmUrl from "@arto-app/markdown/wasm/arto_markdown_wasm_bg.wasm?url";
import type { Library } from "@arto-app/markdown/wasm";
import { hrefOf } from "./records";

declare global {
  interface Window {
    ArtoRenderer?: { render: (viewer: HTMLElement) => void };
    handleMarkdownLinkClick?: (path: string, button: number) => void;
  }
}

const LIBRARIES: Record<Library, string> = {
  mermaid: mermaidUrl,
  math: mathUrl,
  "hljs-common": hljsCommonUrl,
  "hljs-all": hljsAllUrl,
};

/** Where the notes are rendered at, so a link from the notebook's root stays as written. */
const AT = "/notebook/note.md";

const loaded = new Map<string, Promise<void>>();

function script(src: string): Promise<void> {
  const known = loaded.get(src);
  if (known) return known;
  const made = new Promise<void>((resolve, reject) => {
    const el = document.createElement("script");
    el.src = src;
    el.onload = () => resolve();
    el.onerror = () => reject(new Error(`could not load ${src}`));
    document.head.append(el);
  });
  loaded.set(src, made);
  return made;
}

let engine: Promise<typeof import("@arto-app/markdown/wasm")> | undefined;

function start() {
  engine ??= (async () => {
    if (bundles.version !== 1) throw new Error("an unknown form of Arto's bundles");
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = styleUrl;
    document.head.prepend(link);
    const [arto] = await Promise.all([import("@arto-app/markdown/wasm"), script(runtimeUrl)]);
    await arto.default({ module_or_path: wasmUrl });
    return arto;
  })();
  return engine;
}

/** Where a link in the notes leads on the console, when it leads to a record. */
export function recordHref(path: string): string | undefined {
  const m = /^(?:\.?\/)?records\/([^/]+)\/([^/]+?)(?:\.md)?(?:#.*)?$/.exec(path);
  return m ? hrefOf(m[1], decodeURIComponent(m[2])) : undefined;
}

/**
 * Draw Markdown in `viewer` the way Arto does. Whoever wrote it, nothing in
 * it runs or is fetched from elsewhere: HTML is shown as text, and the
 * renderer is the one for documents that did not come from a file.
 */
export async function draw(viewer: HTMLElement, markdown: string) {
  const arto = await start();
  const files = {
    stat: (path: string) => (/\/(records|memory)\//.test(path) ? { path, len: 1 } : null),
    read: () => null,
  };
  const result = arto.renderDetached(markdown, AT, { rawHtml: "escape", wikiLinks: true, cjkEmphasis: true }, files);
  for (const name of result.libraries) await script(LIBRARIES[name]);
  const article = document.createElement("article");
  article.className = "markdown-body";
  article.innerHTML = result.html;
  for (const a of article.querySelectorAll<HTMLAnchorElement>("a[href^='http']")) {
    a.target = "_blank";
    a.rel = "noreferrer";
  }
  viewer.replaceChildren(article);
  window.ArtoRenderer?.render(viewer);
}
