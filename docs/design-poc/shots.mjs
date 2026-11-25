// Takes every picture in shots/, and checks each page as it is drawn:
// no sideways overflow, no script errors, contrast of every text, size of touch targets.
// Uses the console's own Playwright: cd frontend && pnpm install, then
// node docs/design-poc/shots.mjs [--check]
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const { chromium } = createRequire(process.env.PLAYWRIGHT_FROM ?? join(here, "../../frontend/package.json"))("@playwright/test");
const only = process.argv.includes("--check");
const PAGES = ["home", "calendar", "tasks", "delegated", "routines", "memos", "parties", "things", "contracts", "memory", "approvals", "services", "look", "threads", "parts"];
/** What is picked before a picture: pressed in order. */
const STEPS = [
  ...PAGES.map((p) => [p, p, []]),
  ["delegated-detail", "delegated", ['[data-sel="delegated:j3"]']],
  ["tasks-detail", "tasks", ["#pane [data-open^='tasks:']"]],
  ["parties-detail", "parties", ['[data-sel="parties:p1"]']],
  ["things-detail", "things", ['[data-sel="things:t2"]']],
  ["memory-pending", "memory", ['[data-layer="pending"]']],
  ["memory-core", "memory", ['[data-layer="core"]']],
  ["memory-long", "memory", ['[data-layer="long"]']],
  ["memory-forgotten", "memory", ['[data-layer="forgotten"]']],
  ["memory-detail", "memory", ['[data-layer="episodic"]', '[data-entry="m5"]']],
  ["status", "chat", [".alive:visible"]],
  ["chat", "chat", []],
  ["more", "more", []],
  ["chat-list", "chat", ["#conv .back-list:visible"]],
  ["chat-new", "chat", ['#conv [data-act="tnew"]:visible']],
  ["chat-asked", "chat", ["#conv .back-list:visible", '#conv [data-thread="t2"]']],
  ["chat-result", "chat", ["#conv .back-list:visible", '#conv [data-thread="t5"]']],
  ["chat-side", "chat", [], { conv: "side" }],
  ["chat-stream", "chat", [], { conv: "stream", dest: "chip" }],
  ["chat-arrived", "threads", ['[data-act="arrive"]']],
];
const KEPT = new Set(["home", "tasks", "delegated-detail", "memory-pending", "memory-detail", "parties", "parties-detail", "things", "contracts", "services", "approvals", "calendar", "parts", "chat", "look", "delegated", "memory", "threads", "chat-list", "chat-new", "chat-asked", "chat-result", "chat-side", "chat-stream", "chat-arrived"]);
let bad = 0;
const fail = (where, what) => { bad += 1; console.log(`${where}: ${what}`); };

/** Every text whose contrast on what is behind it is under AA. Runs in the page. */
const audit = (touch) => {
  const ctx = Object.assign(document.createElement("canvas"), { width: 1, height: 1 }).getContext("2d", { willReadFrequently: true });
  const rgba = (c) => { ctx.clearRect(0, 0, 1, 1); ctx.fillStyle = c; ctx.fillRect(0, 0, 1, 1); const d = ctx.getImageData(0, 0, 1, 1).data; return [d[0], d[1], d[2], d[3] / 255]; };
  const over = (a, b) => [0, 1, 2].map((k) => a[k] * a[3] + b[k] * (1 - a[3]));
  const lum = (c) => { const [r, g, b] = c.map((v) => ((v /= 255) <= .03928 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4)); return .2126 * r + .7152 * g + .0722 * b; };
  const out = new Set();
  const walk = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let n; (n = walk.nextNode()); ) {
    const el = n.parentElement;
    if (!n.data.trim() || el.closest("script,style,[hidden],:disabled,option,.sr") || !el.checkVisibility()) continue;
    const layers = [];
    for (let e = el; e; e = e.parentElement) { const c = rgba(getComputedStyle(e).backgroundColor); if (c[3]) layers.push(c); if (c[3] === 1) break; }
    const back = layers.reduceRight((b, c) => over(c, b), [255, 255, 255]);
    const front = over(rgba(getComputedStyle(el).color), back);
    const [x, y] = [lum(front), lum(back)].sort((a, b) => b - a);
    const size = parseFloat(getComputedStyle(el).fontSize);
    const ratio = (x + .05) / (y + .05);
    if (ratio < (size >= 24 ? 3 : 4.5)) out.add(`contrast ${ratio.toFixed(2)} "${n.data.trim().slice(0, 16)}" .${el.className}`);
  }
  // A field is told from the page by its edge alone.
  for (const el of document.querySelectorAll(".input, .select > select, .circle")) {
    if (!el.checkVisibility() || el.matches(":disabled, [aria-pressed='true']")) continue;
    const line = rgba(getComputedStyle(el).borderTopColor);
    let back = el.parentElement;
    while (rgba(getComputedStyle(back).backgroundColor)[3] < 1) back = back.parentElement;
    const [x, y] = [lum(line), lum(rgba(getComputedStyle(back).backgroundColor))].sort((a, b) => b - a);
    if ((x + .05) / (y + .05) < 3) out.add(`edge ${((x + .05) / (y + .05)).toFixed(2)} .${el.className} ${el.getAttribute("aria-label") ?? ""}`);
  }
  if (touch) for (const el of document.querySelectorAll("button, a, input, select, textarea")) {
    if (!el.checkVisibility() || el.closest(".prose, [inert], dialog:not([open])") || el.matches(".cev, .bars button, .kv a, .callout *")) continue;
    const box = (e) => e.getBoundingClientRect();
    let { width: w, height: h } = box(el);
    const label = el.closest("label"); if (label) { h = Math.max(h, box(label).height); w = Math.max(w, box(label).width); }
    if (el.matches(".pick")) { const p = box(el.closest(".item, .mini, .thing") ?? el); h = p.height; w = p.width; }
    if (el.matches(".circle")) { w += 22; h += 22; } if (el.matches(".chip .x")) { w += 26; h += 26; }
    if (el.matches(".switch")) { w += 6; h += 22; }
    if (h && (h < 43.5 || w < 43.5)) out.add(`touch ${Math.round(w)}x${Math.round(h)} ${el.className} ${el.textContent.trim().slice(0, 12)}`);
  }
  for (const e of document.querySelectorAll("body, .page, .dscroll, .talk")) if (e.scrollWidth > e.clientWidth + 1) out.add(`overflow ${e.className || e.tagName} ${e.scrollWidth}>${e.clientWidth}`);
  for (const u of document.querySelectorAll("use")) if (!document.querySelector(u.getAttribute("href"))) out.add(`icon ${u.getAttribute("href")}`);
  return [...out];
};

const browser = await chromium.launch();
const url = pathToFileURL(join(here, "index.html")).href;
for (const [size, width, height] of [["desktop", 1440, 900], ["phone", 390, 844]]) {
  for (const theme of ["light", "dark"]) {
    for (const accent of ["green", "indigo", "plum"]) {
      const context = await browser.newContext({ viewport: { width, height }, colorScheme: theme, reducedMotion: "reduce", hasTouch: size === "phone" });
      await context.addInitScript((a) => localStorage.setItem("famulen-accent", a), accent);
      for (const [name, hash, steps, look = {}] of STEPS) {
        if (size === "desktop" && name === "more") continue;
        const page = await context.newPage();
        await page.addInitScript((l) => { for (const k of ["conv", "dest"]) { if (l[k]) localStorage.setItem(`famulen-${k}`, l[k]); else localStorage.removeItem(`famulen-${k}`); } }, look);
        const where = `${name} ${size} ${theme} ${accent}`;
        page.on("pageerror", (e) => fail(where, e.message));
        page.on("request", (r) => !r.url().startsWith("file:") && !r.url().startsWith("data:") && fail(where, `asks for ${r.url()}`));
        await page.goto(`${url}#${hash}`);
        try { for (const s of steps) await page.locator(s).first().click({ timeout: 3000 }); } catch (e) { fail(where, e.message.split("\n")[0]); }
        for (const w of await page.evaluate(audit, size === "phone")) fail(where, w);
        if (!only && accent === "green" && KEPT.has(name)) await page.screenshot({ path: join(here, `shots/${name}-${size}-${theme}.png`) });
        if (!only && accent !== "green" && theme === "light" && size === "desktop" && name === "delegated") await page.screenshot({ path: join(here, `shots/accent-${accent}.png`) });
        await page.close();
      }
      await context.close();
    }
  }
}

// The theme: the system's is followed live, a choice outlives a reload and is there before the first frame.
const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });
const page = await context.newPage();
const ground = () => page.evaluate(() => getComputedStyle(document.body).backgroundColor);
await page.goto(url);
const dark = await ground();
await page.emulateMedia({ colorScheme: "light" });
const light = await ground();
if (dark === light) fail("theme", "the system's change is not followed");
await page.locator('#theme [data-theme-pick="dark"]').click();
if ((await ground()) !== dark) fail("theme", "dark was picked but is not drawn");
// Seen as the body is put in, before anything of it can be painted.
await page.addInitScript(() => new MutationObserver((_, o) => { if (!document.body) return; window.first = document.documentElement.dataset.theme; o.disconnect(); }).observe(document, { childList: true, subtree: true }));
await page.reload();
if ((await page.evaluate(() => window.first)) !== "dark" || (await ground()) !== dark) fail("theme", "the choice flashes or is lost on reload");
await page.locator('#theme [data-theme-pick="system"]').click();
if ((await ground()) !== light) fail("theme", "going back to the system's is not drawn");
await browser.close();
console.log(bad ? `${bad} problems` : "no problems");
process.exit(bad ? 1 : 0);
