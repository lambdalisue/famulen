import { randomUUID } from "node:crypto";
import { type APIRequestContext, expect, type Locator, type Page, test } from "@playwright/test";
import { call, dayFromToday, detail, record, row } from "./support";

const ids: { [name: string]: string } = {};
const made: { kind: string; id: string }[] = [];
let group = "";

async function add(request: APIRequestContext, kind: string, title: string, given: Parameters<typeof record>[3] = {}) {
  const { id } = await record(request, kind, title, given);
  made.push({ kind, id });
  return id;
}

test.beforeAll(async ({ request }) => {
  test.setTimeout(120_000);
  ({ id: group } = await call<{ id: string }>(request, "WorkgroupService", "AddWorkgroup", { name: `巻物の組 ${randomUUID()}`, color: "#0B5650" }));
  const notes = "長い備考\n\n".repeat(60);
  for (let i = 100; i < 230; i++) ids.thing = await add(request, "holdings", `巻物の物 ${i}`, { fields: { Category: "巻物" } });
  // Found by words, as digits may stand in the code of any record.
  await add(request, "holdings", "巻物の目印", { fields: { Category: "巻物" } });
  await add(request, "routines", "巻物の目印", { fields: { Schedule: "0 9 * * *" } });
  for (let i = 100; i < 130; i++) {
    await add(request, "holdings", `巻物の箱 ${i}`, { fields: { Category: "巻物の箱" } });
    await add(request, "contracts", `巻物の契約 ${i}`);
    await add(request, "parties", `巻物の相手 ${i}`, { fields: { Kind: "person" } });
    ids.task = await add(request, "tasks", `巻物のタスク ${i}`, { notes, fields: { Due: dayFromToday(0) } });
    ids.memo = await add(request, "memos", `巻物のメモ ${i}`, { notes });
    await add(request, "routines", `巻物の定期 ${i}`, { fields: { Schedule: "0 9 * * *" } });
    await add(request, "events", `巻物の予定 ${i}`, { notes, fields: { Start: `${dayFromToday(0)} 10:00`, End: `${dayFromToday(0)} 11:00` } });
    await call(request, "MemoryService", "WriteFile", {
      path: `long/semantic/e2e-scroll-${i}.md`,
      text: `---\nid: e2e-scroll-${i}\nsummary: 巻物の記憶 ${i}\nsrc: said\n---\n\n${notes}\n`,
    });
  }
});

test.afterAll(async ({ request }) => {
  test.setTimeout(120_000);
  for (const r of made) await call(request, "RecordService", "DeleteRecord", r);
  await call(request, "WorkgroupService", "DeleteWorkgroup", { id: group });
});

async function open(page: Page, path: string) {
  await page.goto(path);
  await expect(page.locator("#composer .view")).not.toBeEmpty();
  await expect(page.locator("#pane").getByText("読み込んでいる")).toHaveCount(0);
  await expect(page.locator(".md-plain"), path).toHaveCount(0);
}

/** A change that every page reads again for, and the wait until it is on the screen. */
async function change(request: APIRequestContext, page: Page) {
  const name = `巻物の組 ${randomUUID()}`;
  // Each answer brings every list anew, whatever it was that changed.
  await call(request, "WorkgroupService", "UpdateWorkgroup", { id: group, name });
  await expect(page.locator(".app > dialog.sheet")).toContainText(name);
  await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
}

/** Where the page stands: what scrolls in it, marked so that one made anew is told from it. */
async function scrollAll(page: Page) {
  return page.evaluate(() => {
    const all = [...document.querySelectorAll<HTMLElement>("#pane, #pane *")].filter(
      (el) => el.clientHeight > 0 && el.scrollHeight > el.clientHeight && /auto|scroll/.test(getComputedStyle(el).overflowY) && !el.matches("textarea"),
    );
    all.forEach((el, n) => {
      el.dataset.scrolled = String(n);
      el.scrollTop = Math.min(300, Math.floor((el.scrollHeight - el.clientHeight) / 2));
    });
    return all.map((el, n) => `${n} ${el.className} ${el.scrollTop}`);
  });
}

const places = (page: Page) =>
  page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>("[data-scrolled]")].map((el) => `${el.dataset.scrolled} ${el.className} ${el.scrollTop}`),
  );

async function scrollTo(list: Locator, top: number) {
  await list.evaluate((el, to) => el.scrollTo(0, to), top);
  expect(await list.evaluate((el) => el.scrollTop)).toBe(top);
}

const pages = (): string[] => [
  "/",
  ...["day", "week", "month"].map((u) => `/calendar?unit=${u}&at=${dayFromToday(0)}`),
  "/tasks",
  `/tasks?id=${ids.task}`,
  "/tasks?tab=routines",
  "/tasks?tab=delegated",
  "/memos",
  `/memos?id=${ids.memo}`,
  "/parties",
  "/things",
  "/things?category=巻物",
  `/things?category=巻物&id=${ids.thing}`,
  "/contracts",
  "/memory",
  ...["pending", "episodic", "working", "core", "long", "forgotten"].map((l) => `/memory/${l}`),
  "/memory/file?path=long/semantic/e2e-scroll-101.md&mode=read",
  "/more",
  "/settings",
  ...["workgroups", "lookout", "models", "usage", "channels", "services", "services?builtin=1", "vault", "notices"].map((s) => `/settings/${s}`),
  ...["dream", "triage", "assess", "worker"].map((r) => `/settings/roles/${r}`),
];

for (const [screen, viewport, scroller] of [
  ["a wide screen", { width: 1440, height: 480 }, ".split-left"],
  ["a phone", { width: 390, height: 600 }, "#pane"],
] as const) {
  test.describe(`on ${screen}`, () => {
    test.use({ viewport });

    test("every page stays where it was scrolled to as records change", async ({ page, request }) => {
      test.setTimeout(240_000);
      const moved: { [path: string]: string[][] } = {};
      const scrolled: string[] = [];
      for (const path of pages()) {
        await open(page, path);
        const before = await scrollAll(page);
        if (before.length > 0) scrolled.push(path);
        await change(request, page);
        await change(request, page);
        const after = await places(page);
        if (JSON.stringify(after) !== JSON.stringify(before)) moved[path] = [before, after];
      }
      expect(moved).toEqual({});
      expect(scrolled).toEqual(expect.arrayContaining(["/tasks", "/memos", "/parties", "/things", "/contracts", "/memory/long", "/settings"]));
    });

    test("an event open beside the calendar stays where it was read as records change", async ({ page, request }) => {
      await open(page, `/calendar?unit=week&at=${dayFromToday(0)}`);
      await page.getByRole("button", { name: /巻物の予定/ }).first().dispatchEvent("click");
      await expect(detail(page)).toContainText("長い備考");
      await expect(detail(page).locator(".md-plain")).toHaveCount(0);
      const before = await scrollAll(page);
      expect(before.length).toBeGreaterThan(0);
      await change(request, page);
      await change(request, page);
      expect(await places(page)).toEqual(before);
    });

    test("what narrows a list and what is open beside it stay as records come", async ({ page, request }) => {
      const wide = screen === "a wide screen";
      await open(page, `/things?category=巻物${wide ? `&id=${ids.thing}` : ""}`);
      const search = page.getByLabel("物を探す");
      await search.fill("巻物の物");
      await search.press("End");
      await scrollTo(page.locator(scroller), 300);
      const { id } = await record(request, "holdings", `巻物の物 999 ${screen}`, { fields: { Category: "巻物" } });
      await change(request, page);
      await expect(search).toHaveValue("巻物の物");
      await expect(search).toBeFocused();
      if (wide) await expect(detail(page).getByLabel("題名")).toHaveValue("巻物の物 229");
      expect(await page.locator(scroller).evaluate((el) => el.scrollTop)).toBe(300);
      await call(request, "RecordService", "DeleteRecord", { kind: "holdings", id });
    });

    test("more rows are drawn below without moving those read", async ({ page }) => {
      await open(page, "/things?category=巻物");
      await scrollTo(page.locator(scroller), 300);
      await page.getByRole("button", { name: /^あと/ }).dispatchEvent("click");
      await expect.poll(() => row(page, "巻物の物").count()).toBeGreaterThan(100);
      expect(await page.locator(scroller).evaluate((el) => el.scrollTop)).toBe(300);
    });

    for (const [path, label, words] of [
      ["/things?category=巻物", "物を探す", "巻物の目印"],
      ["/tasks?tab=routines", "定期タスクを探す", "巻物の目印"],
    ] as const) {
      test(`what is typed to narrow ${label} keeps the focus as rows come back`, async ({ page }) => {
        await open(page, path);
        const search = page.getByLabel(label);
        await search.fill(words);
        await expect(row(page, "巻物の物")).toHaveCount(0);
        await expect(row(page, "巻物の定期")).toHaveCount(0);
        await expect(row(page, "巻物の目印").first()).toBeVisible();
        const few = await page.locator("[data-record]").count();
        await search.press("End");
        await search.press("Backspace");
        await search.press("Backspace");
        await expect.poll(() => page.locator("[data-record]").count()).toBeGreaterThan(few);
        await expect(search).toBeFocused();
      });
    }
  });
}

test("a sheet and a dialog opened over a page do not move it", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 480 });
  await open(page, `/things?category=巻物&id=${ids.thing}`);
  await scrollTo(page.locator(".split-left"), 300);
  const sheet = page.getByRole("dialog", { name: "秘書の状態" });
  await page.getByRole("button", { name: "秘書の状態" }).click();
  await expect(sheet.getByText("待っている入力")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(sheet).toBeHidden();
  const add = page.getByRole("button", { name: "物を足す" });
  await add.dispatchEvent("click");
  await expect(page.getByRole("dialog").getByLabel("名前")).toBeVisible();
  await page.keyboard.press("Escape");
  expect(await page.locator(".split-left").evaluate((el) => el.scrollTop)).toBe(300);
  await expect(detail(page)).toBeVisible();
});
