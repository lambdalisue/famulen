import bundles from "@arto-app/markdown/bundles/manifest.json";
import runtimeUrl from "@arto-app/markdown/bundles/embed.iife.js?url";
import styleUrl from "@arto-app/markdown/bundles/embed.css?url";
import hljsAllUrl from "@arto-app/markdown/bundles/page-hljs-all.iife.js?url";
import hljsCommonUrl from "@arto-app/markdown/bundles/page-hljs-common.iife.js?url";
import mathUrl from "@arto-app/markdown/bundles/page-math.iife.js?url";
import mermaidUrl from "@arto-app/markdown/bundles/page-mermaid.iife.js?url";
import wasmUrl from "@arto-app/markdown/wasm/arto_markdown_wasm_bg.wasm?url";
import type { Library } from "@arto-app/markdown/wasm";
import { createEffect, createRoot } from "solid-js";
import { isMap, isScalar, isSeq, parseDocument } from "yaml";
import type { RecordLink } from "~/gen/famulen/v1/records_pb";
import { memoryHref } from "./memory";
import { hrefOf } from "./records";
import { dark } from "./theme";

declare global {
  interface Window {
    ArtoRenderer?: { render: (viewer: HTMLElement) => void; setTheme: (theme: "light" | "dark") => Promise<void> };
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

/** Draw the notes, now and after, in the theme of the page. */
function followDark() {
  createRoot(() => createEffect(() => void window.ArtoRenderer?.setTheme(dark() ? "dark" : "light")));
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
    followDark();
    return arto;
  })();
  return engine;
}

/** The name a link spells, where its escapes can be read as such. */
function fileName(path: string): string {
  try {
    return decodeURIComponent(path);
  } catch {
    return path;
  }
}

const RECORD = /^(?:\.?\/)?records\/([^/]+)\/([^/]+?)(?:\.md)?(?:#.*)?$/;

/** Where a link in the notebook leads on the console: to a record, or to a memory file. */
export function notebookHref(path: string): string | undefined {
  const at = fileName(path.replace(/#.*$/, ""));
  const m = RECORD.exec(at);
  return m ? hrefOf(m[1], m[2]) : memoryHref(at);
}

/** Whether the notebook reads `group` as attributes: a YAML mapping of keys to scalars or lists of them. */
function isAttributes(group: string): boolean {
  const doc = parseDocument(group);
  if (doc.errors.length > 0 || doc.warnings.length > 0 || !isMap(doc.contents)) return false;
  const scalar = (node: unknown) => isScalar(node) && node.value !== null;
  return (
    doc.contents.items.length > 0 &&
    doc.contents.items.every(
      ({ key, value }) =>
        isScalar(key) &&
        typeof key.value === "string" &&
        /^[A-Za-z0-9_-]+$/.test(key.value) &&
        (isSeq(value) ? value.items.every(scalar) : scalar(value)),
    )
  );
}

/** An item's line up to its attributes, and what is in their braces. */
function splitAttributes(line: string): [string, string] | undefined {
  const item = line.trimEnd();
  const start = /^[ \t]*- /.exec(item)?.[0].length;
  if (start === undefined || !item.endsWith("}")) return undefined;
  for (let open = item.lastIndexOf("{"); open >= start; open = item.lastIndexOf("{", open - 1)) {
    if (item[open - 1] === " " && isAttributes(item.slice(open))) return [item.slice(0, open), item.slice(open + 1, -1)];
  }
  return undefined;
}

/** What tells a wiki link from the others the renderer draws. */
const WIKI = "\uE002";

const MARK = /\uE000(\d+)\uE001/;

/**
 * Put a mark where each item of `markdown` has its attributes, so they are
 * not drawn as a part of its text.
 */
function setApart(markdown: string): { text: string; attributes: string[] } {
  const attributes: string[] = [];
  let fence: string | undefined;
  const lines = markdown.split("\n").map((line) => {
    const mark = /^\s*(`{3,}|~{3,})/.exec(line)?.[1];
    if (fence !== undefined) {
      if (mark?.startsWith(fence)) fence = undefined;
      return line;
    }
    if (mark) fence = mark;
    const split = splitAttributes(line);
    return split ? `${split[0]}\uE000${attributes.push(split[1]) - 1}\uE001` : line;
  });
  return { text: lines.join("\n"), attributes };
}

/** Put the attributes back where their marks are. */
function putBack(article: HTMLElement, attributes: string[]) {
  const walker = document.createTreeWalker(article, NodeFilter.SHOW_TEXT);
  const texts: Text[] = [];
  while (walker.nextNode()) texts.push(walker.currentNode as Text);
  for (const text of texts) {
    const inCode = text.parentElement?.closest("pre, code");
    let rest = text;
    for (let m = MARK.exec(rest.data); m; m = MARK.exec(rest.data)) {
      const value = attributes[Number(m[1])];
      if (value === undefined) break;
      const mark = rest.splitText(m.index);
      rest = mark.splitText(m[0].length);
      if (inCode) {
        mark.data = `{${value}}`;
        continue;
      }
      const el = document.createElement("span");
      el.className = "attrs mono";
      el.textContent = value;
      mark.replaceWith(el);
    }
  }
}

/**
 * Draw Markdown in `viewer` the way Arto does. Whoever wrote it, nothing in
 * it runs or is fetched from elsewhere: HTML is shown as text, and the
 * renderer is the one for documents that did not come from a file. `links`
 * says what the records it links to are called now, and which are gone.
 */
export async function draw(
  viewer: HTMLElement,
  markdown: string,
  { links = [], ...how }: { inline?: boolean; attributes?: boolean; links?: RecordLink[] } = {},
) {
  const arto = await start();
  const known = (path: string) => {
    const m = RECORD.exec(path);
    return m && links.find((l) => l.kind === m[1] && l.id === m[2]);
  };
  const files = {
    stat: (path: string) => {
      const target = path.replace(WIKI, "");
      const there = /^\/notebook\/(records|memory)\//.test(target) && !known(target.slice("/notebook/".length))?.missing;
      return there ? { path, len: 1 } : null;
    },
    read: () => null,
  };
  const { text, attributes } = how.attributes ? setApart(markdown) : { text: markdown, attributes: [] };
  const result = arto.renderDetached(text.replace(/\[\[[ \t]*/g, `$&${WIKI}`), AT, { rawHtml: "escape", wikiLinks: true, cjkEmphasis: true }, files);
  for (const name of result.libraries) await script(LIBRARIES[name]);
  const article = document.createElement(how.inline ? "span" : "article");
  article.className = "markdown-body";
  article.innerHTML = result.html
    .replaceAll(`data-md-link="${WIKI}`, 'data-wiki data-md-link="')
    .replaceAll(WIKI, "");
  // A lone paragraph is a line of text; any other form keeps its blocks.
  const only = article.firstElementChild;
  if (how.inline && article.childElementCount === 1 && only?.tagName === "P") article.replaceChildren(...only.childNodes);
  putBack(article, attributes);
  // A wiki link names its file as it is; any other link is a URL.
  for (const link of article.querySelectorAll<HTMLElement>("[data-wiki]"))
    link.dataset.mdLink = link.dataset.mdLink?.replaceAll("%", "%25");
  const folded = article.querySelector(".frontmatter-summary");
  if (folded) folded.textContent = "属性";
  for (const a of article.querySelectorAll<HTMLAnchorElement>("a[href^='http']")) {
    a.target = "_blank";
    a.rel = "noreferrer";
  }
  for (const a of article.querySelectorAll<HTMLElement>(".md-link")) {
    const path = a.dataset.mdLink ?? "";
    const now = known(path);
    if (a.classList.contains("md-link-missing")) {
      a.title = "リンクの先がない";
      a.removeAttribute("onmousedown");
      continue;
    }
    if (now?.title && a.textContent === path.replace(/\.md$/, "")) a.textContent = now.title;
    if (now?.title) a.title = now.title;
    a.tabIndex = 0;
    a.setAttribute("role", "link");
  }
  viewer.replaceChildren(article);
  window.ArtoRenderer?.render(viewer);
}
