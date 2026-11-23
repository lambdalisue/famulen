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

async function open(page: Page, path: string) {
  await page.goto(path);
  await expect(page.locator("#composer .view")).not.toBeEmpty();
}

test.describe("no page is wider than it is given, at any of the widths", () => {
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
