import { expect, type Page, test } from "@playwright/test";
import { call, dayFromToday, record } from "./support";

const WIDTHS = [390, 1280, 1440, 1920];

const LONG = "住宅改修の見積もりを三社ぶん比べて、どこに頼むかを決めて返事をする（駐輪場の屋根と外構の話も合わせて）";

/** The columns of a page that are wider inside than they are. */
function tooWide(page: Page): Promise<string[]> {
  return page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>("html, #pane, .split-left, .split-scroll, .detail, .chat, .m-head")]
      .filter((el) => el.clientWidth > 0 && el.scrollWidth > el.clientWidth + 1)
      .map((el) => `${el.tagName}.${el.className} ${el.scrollWidth} > ${el.clientWidth}`),
  );
}

/** What is longer inside than it is, though only what is within it is to scroll. */
function tooLong(page: Page): Promise<string[]> {
  return page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>("html, .app, .body, .main, #pane:has(> .framed, > .split-pane), .split-detail, .chat")]
      .filter((el) => el.clientHeight > 0 && el.scrollHeight > el.clientHeight + 1)
      .map((el) => `${el.tagName}.${el.className} ${el.scrollHeight} > ${el.clientHeight}`),
  );
}

async function open(page: Page, path: string) {
  await page.goto(path);
  await expect(page.locator("#composer .view")).not.toBeEmpty();
}

test.describe("no page is larger than it is given, at any of the widths", () => {
  const paths: string[] = [];

  test.beforeAll(async ({ request }) => {
    const party = await record(request, "parties", "はみ出さない相手・キャサリン・エリザベス・モンゴメリー＝スミス", {
      fields: { Kind: "person" },
      sections: [
        {
          name: "Accounts",
          items: [{ text: "email", fields: [{ name: "Handle", value: "a-very-long-address-that-does-not-break@very-long-domain-name.example.co.jp" }] }],
        },
      ],
    });
    const task = await record(request, "tasks", LONG, {
      labels: ["家", "見積もり"],
      notes: `## 表\n\n| 会社 | 金額 |\n| --- | ---: |\n| A | 1,280,000 円 |\n\n\`\`\`\n${"長い行".repeat(80)}\n\`\`\`\n\nhttps://example.com/${"a".repeat(200)}`,
      fields: { Due: dayFromToday(0), Priority: "high" },
    });
    const thing = await record(request, "holdings", "はみ出さない物", {
      fields: { Category: "機器", Serial: "C02".repeat(20) },
      sections: [{ name: "Deadlines", items: [{ text: "保証", fields: [{ name: "On", value: dayFromToday(9) }] }] }],
    });
    const memo = await record(request, "memos", LONG, { notes: "本文 ".repeat(200) });
    const event = await record(request, "events", LONG, { fields: { Start: `${dayFromToday(0)} 10:00`, End: `${dayFromToday(0)} 11:00`, Location: "会議室" } });
    await call(request, "MemoryService", "WriteFile", {
      path: "long/semantic/e2e-wide.md",
      text: `---\nid: e2e-wide\nsummary: ${LONG}\nsrc: said\n---\n\n${LONG}\n`,
    });
    paths.push(
      "/",
      ...["day", "week", "month"].map((u) => `/calendar?unit=${u}&at=${dayFromToday(0)}`),
      `/calendar?unit=week&at=${dayFromToday(0)}&record=${event.id}`,
      "/tasks",
      `/tasks?id=${task.id}`,
      "/tasks?tab=routines",
      "/tasks?tab=delegated",
      "/memos",
      `/memos?id=${memo.id}`,
      "/parties",
      `/parties?id=${party.id}`,
      "/things",
      `/things?id=${thing.id}`,
      "/contracts",
      "/memory",
      ...["pending", "episodic", "working", "core", "long", "forgotten"].map((l) => `/memory/${l}`),
      ...["read", "write", "history"].map((m) => `/memory/file?path=long/semantic/e2e-wide.md&mode=${m}`),
      "/memory/menu",
      "/more",
      "/settings",
      ...["menu", "workgroups", "lookout", "models", "usage", "channels", "services", "services?id=famulen", "services?builtin=1", "vault"].map((s) => `/settings/${s}`),
      ...["dream", "triage", "assess", "worker"].map((r) => `/settings/roles/${r}`),
    );
  });

  for (const width of WIDTHS) {
    test(`at ${width}`, async ({ page }) => {
      test.setTimeout(180_000);
      await page.setViewportSize({ width, height: 900 });
      const wide: { [path: string]: string[] } = {};
      for (const path of paths) {
        await open(page, path);
        // What is written is drawn after the page is, and a table or a line of code is the widest there is.
        await expect(page.locator(".md-plain")).toHaveCount(0);
        const found = await tooWide(page);
        if (found.length > 0) wide[path] = found;
      }
      expect(wide).toEqual({});
    });
  }

  // A window this low leaves no page room enough, so each has something to scroll.
  for (const width of WIDTHS.filter((w) => w > 700)) {
    test(`at ${width} and low, the page itself never scrolls`, async ({ page }) => {
      test.setTimeout(180_000);
      await page.setViewportSize({ width, height: 480 });
      const long: { [path: string]: string[] } = {};
      for (const path of paths) {
        await open(page, path);
        await expect(page.locator(".md-plain")).toHaveCount(0);
        await expect(page.locator("#pane").getByText("読み込んでいる")).toHaveCount(0);
        const found = await tooLong(page);
        if (found.length > 0) long[path] = found;
      }
      expect(long).toEqual({});
    });
  }
});

test.describe("the left and the right each scroll by themselves", () => {
  /** Turn the wheel over an element, further than it can go. */
  async function wheel(page: Page, over: string) {
    await page.locator(over).hover({ position: { x: 20, y: 60 } });
    await page.mouse.wheel(0, 5000);
  }
  const top = (page: Page, of: string) => page.locator(of).evaluate((el) => el.scrollTop);

  async function scrollApart(page: Page, left: string, right: string) {
    for (const [over, other] of [
      [left, right],
      [right, left],
    ]) {
      const before = await top(page, other);
      await wheel(page, over);
      await expect.poll(() => top(page, over)).toBeGreaterThan(0);
      expect(await top(page, other)).toBe(before);
      expect(await page.evaluate(() => window.scrollY)).toBe(0);
      await expect(page.locator(".topbar")).toBeInViewport({ ratio: 1 });
      await expect(page.locator(".tabs")).toBeInViewport({ ratio: 1 });
    }
  }

  for (const width of WIDTHS.filter((w) => w > 700)) {
    test(`at ${width}`, async ({ page, request }) => {
      await page.setViewportSize({ width, height: 480 });
      let id = "";
      for (let i = 0; i < 15; i++) id = (await record(request, "memos", `長い一覧のメモ ${width}-${i}`, { notes: "長い本文\n\n".repeat(60) })).id;
      await open(page, `/memos?id=${id}`);
      await expect(page.locator(".md-plain")).toHaveCount(0);
      await page.locator(".split-left").evaluate((el) => el.scrollTo(0, 0));
      await scrollApart(page, ".split-left", ".split-scroll");

      await open(page, "/settings");
      await expect(page.locator(".detail .radio-cards").first()).toBeVisible();
      await scrollApart(page, ".side", ".detail");

      // With nothing more to show at the left, the wheel over it is for nothing else either.
      await page.setViewportSize({ width, height: 900 });
      await open(page, "/settings");
      await expect(page.locator(".detail .radio-cards").first()).toBeVisible();
      await wheel(page, ".side");
      await wheel(page, ".chat .messages");
      // A wheel is answered only after the fact: the next one that is answered shows this one was too.
      await wheel(page, ".detail");
      await expect.poll(() => top(page, ".detail")).toBeGreaterThan(0);
      expect(await page.evaluate(() => window.scrollY)).toBe(0);
    });
  }
});

test.describe("the conversation is as high as the window, with the bar over the pages alone", () => {
  const box = (page: Page, of: string) =>
    page.locator(of).evaluate((el) => {
      const { left, top, right, bottom } = el.getBoundingClientRect();
      return { left, top, right, bottom };
    });

  // 701 is the narrowest window with the two side by side.
  for (const width of [701, ...WIDTHS.filter((w) => w > 700)]) {
    test(`at ${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await open(page, "/tasks");
      const chat = await box(page, ".chat");
      const messages = await box(page, ".messages");
      const bar = await box(page, ".topbar");
      const tabs = await box(page, ".tabs");
      expect(chat).toMatchObject({ left: 0, top: 0, bottom: 900 });
      expect(messages.top).toBe(0);
      expect(bar).toMatchObject({ left: chat.right, top: 0, right: width, bottom: tabs.top });
      await expect(page.locator(".topbar .logo")).toBeInViewport({ ratio: 1 });
      await expect(page.locator(".topbar .brand")).toBeInViewport({ ratio: 1 });
      await expect(page.getByRole("button", { name: "秘書の状態" })).toBeInViewport({ ratio: 1 });
      await page.locator(".topbar").getByRole("link", { name: "ワークグループを足す" }).click();
      await expect(page).toHaveURL(/settings\/workgroups/);
    });
  }

  test("a long name leaves the bar room for the rest", async ({ page }) => {
    await page.setViewportSize({ width: 701, height: 900 });
    await open(page, "/tasks");
    const name = page.locator(".topbar .brand b");
    await name.evaluate((el) => (el.textContent = "とても長い名前を主人に付けてもらった秘書の名前".repeat(2)));
    await expect(page.getByRole("button", { name: "秘書の状態" })).toBeInViewport({ ratio: 1 });
    expect(await box(page, ".topbar")).toMatchObject({ right: 701 });
    expect(await page.locator(".wg-tabs").evaluate((el) => el.clientWidth)).toBeGreaterThan(100);
  });

  test("a phone keeps its own bar over the whole width", async ({ page }) => {
    await page.setViewportSize({ width: 700, height: 900 });
    await open(page, "/tasks");
    await expect(page.locator(".topbar")).toBeHidden();
    const head = await box(page, "#m-head");
    expect(head).toMatchObject({ left: 0, top: 0, right: 700 });
    expect(await box(page, "#main")).toMatchObject({ left: 0, top: head.bottom, right: 700, bottom: 900 });
  });
});

test("a phone shows a record over its list, and the list comes back", async ({ page, request }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const memo = await record(request, "memos", "幅の狭い画面のメモ", { notes: "本文" });
  await open(page, "/memos");
  const add = await page.getByRole("button", { name: "メモを書く" }).boundingBox();
  expect(add?.y ?? 0).toBeGreaterThan(422);

  await page.locator("[data-record]").filter({ hasText: "幅の狭い画面のメモ" }).getByRole("link").click();
  await expect(page).toHaveURL(new RegExp(`id=${memo.id}`));
  await expect(page.locator("[data-record]").first()).toBeHidden();
  await expect(page.getByRole("button", { name: "メモを書く" })).toHaveCount(0);
  await page.goBack();
  await expect(page.locator("[data-record]").first()).toBeVisible();
});

test("every button and link says what it does", async ({ page, request }) => {
  const t = await record(request, "tasks", "名前のあるボタン", { labels: ["札"], notes: "本文" });
  for (const path of ["/", "/calendar", "/tasks", `/tasks?id=${t.id}`, "/memory", "/settings/services", "/settings/vault", "/settings/models"]) {
    await open(page, path);
    const nameless = await page.evaluate(() =>
      [...document.querySelectorAll<HTMLElement>("button, a[href]")]
        .filter((b) => b.offsetParent && !b.textContent?.trim() && !b.getAttribute("aria-label"))
        .map((b) => b.outerHTML.slice(0, 120)),
    );
    expect(nameless, path).toEqual([]);
  }
});
