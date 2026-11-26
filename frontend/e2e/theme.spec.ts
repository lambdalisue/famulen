import { expect, type Page, test } from "@playwright/test";
import { faint } from "./legible";
import { call, dayFromToday, record, visit } from "./support";

const ground = (page: Page) => page.evaluate(() => getComputedStyle(document.body).backgroundColor);
const LIGHT = "rgb(243, 244, 247)";
const DARK = "rgb(13, 15, 22)";

/** The theme is in the bar, and on its own settings page. */
const choice = (page: Page) => page.getByRole("radiogroup", { name: "テーマ", exact: true });

test("unless chosen the theme is the system's, and turns with it", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await visit(page, "/settings/look", "設定 · 表示");
  await expect(choice(page).getByRole("radio", { name: "システム" })).toBeChecked();
  await expect.poll(() => ground(page)).toBe(DARK);
  await page.emulateMedia({ colorScheme: "light" });
  await expect.poll(() => ground(page)).toBe(LIGHT);
});

test("the chosen theme stands after a reload, before any script of the page runs, and in the other tabs", async ({ page, context }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await visit(page, "/settings/look", "設定 · 表示");
  const other = await context.newPage();
  await other.emulateMedia({ colorScheme: "light" });
  await visit(other, "/", /^ホーム/);

  await choice(page).getByText("ダーク").click();
  await expect.poll(() => ground(page)).toBe(DARK);
  await expect.poll(() => ground(other)).toBe(DARK);

  // Without the page's own scripts only what the head runs is left.
  await page.route(/\.js(\?|$)/, (route) => route.abort());
  await page.reload();
  await expect.poll(() => ground(page)).toBe(DARK);
  await page.unroute(/\.js(\?|$)/);

  await visit(page, "/", /^ホーム/);
  await page.getByRole("radiogroup", { name: "テーマの切り替え" }).getByTitle("システム").click();
  await expect.poll(() => ground(page)).toBe(LIGHT);
  await expect.poll(() => ground(other)).toBe(LIGHT);
});

test("the bar of a phone takes the color the page has at its top", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  for (const colorScheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme });
    await visit(page, "/", /^ホーム/);
    const bar = () => page.evaluate(() => {
      const drawn = (css: string) => {
        const el = document.body.appendChild(document.createElement("i"));
        el.style.color = css;
        const c = getComputedStyle(el).color;
        el.remove();
        return c;
      };
      const meta = [...document.querySelectorAll<HTMLMetaElement>('meta[name="theme-color"]')].find((m) => window.matchMedia(m.media).matches);
      const top = document.body.classList.contains("chat-open") ? ".chat" : "#m-head";
      return [drawn(meta?.content ?? ""), getComputedStyle(document.querySelector(top) ?? document.body).backgroundColor];
    });
    const [named, top] = await bar();
    expect(named).toBe(top);
    await page.getByRole("button", { name: "会話を引き上げる" }).click();
    await expect(page.locator("body")).toHaveClass(/chat-open/);
    const [still, over] = await bar();
    expect(still).toBe(over);
    // Left open, the page read again would show it open.
    await page.getByRole("button", { name: "閉じる" }).click();
  }
});

test("a note is drawn in the theme of the page", async ({ page, request }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  const memo = await record(request, "memos", "暗い頁のメモ", { notes: "本文の **太字** と `code`\n\n- 一つ目" });
  await visit(page, `/memos?id=${memo.id}`, /^メモ/);
  const viewer = page.locator("article.rec .markdown-viewer").filter({ hasText: "本文の" });
  await expect(viewer).toHaveAttribute("data-theme", "dark");
});

test.describe("every text and field can be told from what is behind it", () => {
  const paths: string[] = [];

  test.beforeAll(async ({ request }) => {
    const party = await record(request, "parties", "読める相手", { fields: { Kind: "person" } });
    const task = await record(request, "tasks", "読める仕事", {
      labels: ["家"],
      notes: "本文の [リンク](https://example.com) と `code`\n\n> 引用",
      fields: { Due: dayFromToday(-1), Priority: "high" },
    });
    const thing = await record(request, "holdings", "読める物", {
      fields: { Category: "機器" },
      sections: [{ name: "Deadlines", items: [{ text: "保証", fields: [{ name: "On", value: dayFromToday(3) }] }] }],
    });
    await record(request, "events", "読める予定", { fields: { Start: `${dayFromToday(0)} 10:00`, End: `${dayFromToday(0)} 11:00`, Location: "会議室" } });
    await call(request, "ConsoleService", "Send", { text: "読める発言の **太字** と `code` と [リンク](https://example.invalid)", workgroup: "all" });
    paths.push(
      "/",
      ...["day", "week", "month"].map((u) => `/calendar?unit=${u}&at=${dayFromToday(0)}`),
      "/tasks",
      `/tasks?id=${task.id}`,
      "/tasks?tab=routines",
      "/tasks?tab=delegated",
      "/memos",
      `/parties?id=${party.id}`,
      `/things?id=${thing.id}`,
      "/contracts",
      "/memory",
      ...["pending", "episodic", "working", "core", "long", "forgotten"].map((l) => `/memory/${l}`),
      "/more",
      "/settings",
      ...["menu", "workgroups", "look", "lookout", "models", "usage", "notices", "channels", "services", "services?id=famulen", "vault"].map((s) => `/settings/${s}`),
      ...["dream", "triage", "assess", "worker"].map((r) => `/settings/roles/${r}`),
    );
  });

  for (const colorScheme of ["light", "dark"] as const) {
    for (const width of [1440, 390]) {
      test(`in ${colorScheme} at ${width}`, async ({ page }) => {
        test.setTimeout(180_000);
        await page.setViewportSize({ width, height: 900 });
        // A color on its way from one theme to the other is neither.
        await page.emulateMedia({ colorScheme, reducedMotion: "reduce" });
        const faults: { [path: string]: string[] } = {};
        for (const path of paths) {
          await page.goto(path);
          await expect(page.locator("#composer .view")).not.toBeEmpty();
          const found = await faint(page);
          if (found.length > 0) faults[path] = found;
        }
        expect(faults).toEqual({});
      });
    }
  }
});
