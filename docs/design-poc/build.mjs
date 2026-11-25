// Puts the sources together into one page that opens by itself.
// node docs/design-poc/build.mjs [--artifact <path>]
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const read = (p) => readFileSync(join(here, p), "utf8").trim();
const css = ["tokens", "ui", "shell", "threads"].map((n) => read(`src/${n}.css`)).join("\n");
const js = ["data", "threads", "app"].map((n) => read(`src/${n}.js`)).join("\n");
const art = read("../../brand/male/app-icon.svg");
// Ids inside the art would clash with the page's own.
const face = art.replace(/^<svg[^>]*>/, '<symbol id="aide-face" viewBox="0 0 1024 1024">').replace(/<\/svg>$/, "</symbol>").replaceAll("round", "aide-round");
const icons = read("src/icons.svg").replace(/<\/svg>$/, `${face}</svg>`);
const used = new Set([...js.matchAll(/["']([a-z0-9]+(?:-[a-z0-9]+)*)["']/g)].map((m) => m[1]));
const sprite = icons.replace(/<symbol id="i-([^"]+)"[\s\S]*?<\/symbol>\n?/g, (all, name) => (used.has(name) ? all : ""));
const head = `<title>Famulen コンソールの PoC</title>\n<script>${read("src/head.js")}</script>\n<style>\n${css}\n</style>`;
const body = `<!-- Icons: Tabler Icons, ${read("src/icons.LICENSE").match(/Copyright.*/)[0]}, MIT License (src/icons.LICENSE) -->\n${sprite}\n<div class="app" id="app"></div>\n<script>\n${js}\n</script>`;
writeFileSync(join(here, "index.html"), `<!doctype html>\n<html lang="ja">\n<head>\n<meta charset="utf-8">\n<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover, interactive-widget=resizes-content">\n${head}\n</head>\n<body>\n${body}\n</body>\n</html>\n`);
const out = process.argv.indexOf("--artifact");
if (out > 0) writeFileSync(process.argv[out + 1], `${head}\n${body}\n`);
