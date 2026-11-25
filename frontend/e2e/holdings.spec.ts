import { expect, type Page, test } from "@playwright/test";
import { agree, call, dayFromToday, detail, field, record, row, toast, visit } from "./support";

const deadline = (what: string, days: number) => [{ name: "Deadlines", items: [{ text: what, fields: [{ name: "On", value: dayFromToday(days) }] }] }];

const categories = (page: Page) => page.getByRole("navigation", { name: "分類" });

for (const [kind, path, view, add] of [
  ["holdings", "/things", "物", "物を足す"],
  ["contracts", "/contracts", "契約", "契約を足す"],
] as const) {
  test.describe(view, () => {
    test("one added in the dialog opens beside the list", async ({ page }) => {
      await visit(page, path, view);
      await page.getByRole("button", { name: add }).click();
      const dialog = page.getByRole("dialog");
      await dialog.getByLabel("名前").fill(`${view}の新入り`);
      await dialog.getByLabel("分類", { exact: true }).fill("機器");
      await dialog.getByRole("button", { name: "足す", exact: true }).click();
      await expect(toast(page)).toHaveText("足した");
      await expect(detail(page).getByLabel("題名")).toHaveValue(`${view}の新入り`);
      await expect(field(page, "分類")).toContainText("機器");
    });

    test("those with the soonest deadline come first, with the days left", async ({ page, request }) => {
      await record(request, kind, `${view}の後`, { fields: { Category: `${view}期限` }, sections: deadline("更新", 60) });
      await record(request, kind, `${view}の先`, { fields: { Category: `${view}期限` }, sections: deadline("保証", 20) });
      await visit(page, path, view);
      await page.getByLabel(`${view}を探す`).fill(`${view}期限`);
      await expect(page.locator("[data-record]")).toHaveText([/の先.*保証 20 日後/, /の後.*更新 60 日後/]);
    });

    test("the list and the cards hold the same, and a choice narrows them", async ({ page, request }) => {
      await record(request, kind, `${view}の甲`, { fields: { Category: "甲類" }, notes: "棚の上" });
      await record(request, kind, `${view}の乙`, { fields: { Category: "乙類" } });
      await visit(page, path, view);
      if (kind === "holdings") await categories(page).getByRole("link", { name: "甲類" }).click();
      else await page.getByLabel("分類で絞る").selectOption("甲類");
      await expect(row(page, `${view}の甲`)).toBeVisible();
      await expect(row(page, `${view}の乙`)).toHaveCount(0);
      await page.getByRole("radiogroup", { name: "並べ方" }).getByText("カード").click();
      await expect(page.locator(".rec-card").filter({ hasText: `${view}の甲` })).toContainText("棚の上");
      await page.reload();
      await expect(page.locator(".rec-card").first()).toBeVisible();
    });

    test("one is removed after asking", async ({ page, request }) => {
      const r = await record(request, kind, `${view}の不要品`);
      await visit(page, `${path}?id=${r.id}`, `${view} · ${view}の不要品`);
      await page.getByRole("button", { name: `${view}の不要品を消す` }).click();
      await agree(page, "消す");
      await expect(toast(page)).toHaveText("消した");
      await expect(row(page, `${view}の不要品`)).toHaveCount(0);
    });
  });
}

test("what is being typed as another item stays when the record is read again", async ({ page, request }) => {
  const r = await record(request, "holdings", "打ちかけの物", { fields: { Status: "owned" } });
  await visit(page, `/things?id=${r.id}`, "物 · 打ちかけの物");
  await detail(page).getByRole("button", { name: "＋ 期限" }).click();
  await detail(page).getByLabel("期限を足す").fill("点検");
  await call(request, "RecordService", "SaveRecord", { kind: "holdings", id: r.id, title: "読み直した物" });
  await expect(detail(page)).toContainText("読み直した物");
  await expect(detail(page).getByLabel("期限を足す")).toHaveValue("点検");
});

test("a thing says how it was got and several deadlines, and one let go is left out until asked for", async ({ page, request }) => {
  const r = await record(request, "holdings", "譲るカメラ", { fields: { Status: "owned" } });
  await visit(page, `/things?id=${r.id}`, "物 · 譲るカメラ");
  await detail(page).getByRole("button", { name: "＋ 手に入れた" }).click();
  const got = detail(page).locator("section").filter({ has: page.getByRole("heading", { name: "手に入れた", exact: true }) });
  await got.getByLabel("項目を足す").selectOption("Price");
  await got.getByLabel("金額の額").fill("120000");
  await got.getByLabel("金額の額").press("Tab");
  await expect(toast(page)).toHaveText("保存した");
  await got.getByLabel("項目を足す").selectOption("How");
  // The toast of the save before may still show, so it cannot tell this one is done.
  const saved = page.waitForResponse((r) => r.request().method() === "POST" && (r.request().postData() ?? "").includes("gift"));
  await got.getByLabel("どうやって", { exact: true }).selectOption("gift");
  await saved;
  await page.reload();
  await expect(got).toContainText("120,000");
  await expect(got).toContainText("もらった");

  await detail(page).getByRole("button", { name: "＋ 期限" }).click();
  await detail(page).getByLabel("期限を足す").fill("保証");
  await detail(page).getByLabel("期限を足す").press("Enter");
  await detail(page).getByLabel("期限を足す").fill("点検");
  // What is typed while the one before is saved stays.
  await expect(detail(page).locator(".rec-item")).toHaveText([/保証/]);
  await expect(detail(page).getByLabel("期限を足す")).toHaveValue("点検");
  await detail(page).getByLabel("期限を足す").press("Enter");
  await expect(detail(page).locator(".rec-item")).toHaveText([/保証/, /点検/]);

  await field(page, "状態").getByRole("button", { name: "状態を直す" }).click();
  await field(page, "状態").getByLabel("状態", { exact: true }).selectOption("released");
  await page.goto("/things");
  await expect(row(page, "譲るカメラ")).toHaveCount(0);
  await page.getByLabel("手放した物も").check();
  await expect(row(page, "譲るカメラ")).toBeVisible();
});

test("a category's tab stays in the address and over what is opened from it", async ({ page, request }) => {
  await record(request, "holdings", "棚の辞書", { fields: { Category: "辞書類" } });
  await record(request, "holdings", "棚の地図", { fields: { Category: "地図類" } });
  await visit(page, "/things", "物");
  await categories(page).getByRole("link", { name: "辞書類 1" }).click();
  await expect(row(page, "棚の地図")).toHaveCount(0);
  await row(page, "棚の辞書").getByRole("link").click();
  await page.reload();
  await expect(detail(page).getByLabel("題名")).toHaveValue("棚の辞書");
  await expect(row(page, "棚の地図")).toHaveCount(0);
  await page.getByRole("button", { name: "詳細を閉じる" }).click();
  await expect(categories(page).getByRole("link", { name: "辞書類 1" })).toHaveAttribute("aria-current", "true");

  await page.getByRole("button", { name: "物を足す" }).click();
  await page.getByRole("dialog").getByLabel("名前").fill("棚の図鑑");
  await page.getByRole("dialog").getByRole("button", { name: "足す", exact: true }).click();
  await expect(field(page, "分類")).toContainText("辞書類");
  await expect(row(page, "棚の図鑑")).toBeVisible();

  await page.reload();
  await page.getByRole("button", { name: "棚の図鑑を消す" }).click();
  await agree(page, "消す");
  await expect(categories(page).getByRole("link", { name: "辞書類 1" })).toHaveAttribute("aria-current", "true");

  await categories(page).getByRole("link", { name: "地図類" }).focus();
  await page.keyboard.press("Enter");
  await expect(categories(page).getByRole("link", { name: "地図類" })).toHaveAttribute("aria-current", "true");
  await expect(categories(page).getByRole("link", { name: "地図類" })).toBeFocused();
});

test("a large category shows a few among all, and the rest in its own tab", async ({ page, request }) => {
  for (let i = 100; i < 220; i++) await record(request, "holdings", `蔵書 ${i}`, { fields: { Category: "蔵書類" } });
  // Found by words, as digits may stand in the code of any record.
  await record(request, "holdings", "蔵書の目録", { fields: { Category: "蔵書類" } });
  await visit(page, "/things", "物");
  await expect(row(page, "蔵書")).toHaveCount(5);
  await expect(categories(page).getByRole("link").nth(1)).toHaveText("蔵書類 121");
  await page.getByRole("link", { name: "蔵書類 121 件を見る" }).click();
  await expect(row(page, "蔵書")).toHaveCount(100);
  await expect(row(page, "蔵書 100")).toBeInViewport();
  const list = page.locator(".rec-rows");
  expect(await list.evaluate((el) => el.scrollHeight - el.clientHeight)).toBe(0);
  await page.getByRole("button", { name: "あと 21 件" }).click();
  await expect(row(page, "蔵書")).toHaveCount(121);

  await page.getByLabel("物を探す").fill("蔵書の目録");
  await expect(row(page, "蔵書")).toHaveCount(1);
  await categories(page).getByRole("link", { name: "甲類" }).click();
  await expect(page.getByText("当てはまるものがない")).toBeVisible();
});

test("categories past those that fit are picked from a list", async ({ page, request }) => {
  for (let i = 1; i <= 9; i++) await record(request, "holdings", `余りの品 ${i}`, { fields: { Category: `余り ${i}` } });
  await visit(page, "/things", "物");
  await page.getByRole("combobox", { name: "ほかの分類" }).selectOption("余り 9");
  await expect(page.locator("[data-record]")).toHaveText([/余りの品 9/]);
  await expect(page.getByRole("combobox", { name: "ほかの分類" })).toHaveValue("余り 9");
  await page.getByLabel("物を探す").fill("余り");
  await expect(categories(page).getByRole("link", { name: "すべて 9" })).toBeVisible();
  await expect(page.getByRole("combobox", { name: "ほかの分類" })).toHaveValue("余り 9");
});

const PICTURE = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==", "base64");

/** Things read from here on have a picture, which a mail alone could give. */
async function pictured(page: Page, id: string, picture: string) {
  await page.route(/RecordService\/(ListRecords|GetRecord)/, async (route) => {
    const answer = await route.fetch();
    const json = await answer.json();
    for (const r of json.records ?? [json]) if (r.id === id) r.picture = picture;
    await route.fulfill({ response: answer, json });
  });
  await visit(page, "/memos", /^メモ/);
  await page.evaluate(() => {
    const a = document.createElement("a");
    a.href = "/things";
    document.querySelector("main")?.append(a);
    a.click();
  });
}

for (const [name, viewport] of [
  ["a wide screen", { width: 1440, height: 900 }],
  ["a phone", { width: 360, height: 740 }],
] as const) {
  test.describe(`on ${name}`, () => {
    test.use({ viewport });

    test("a thing's picture is small in the list, larger on its page, and opens over everything", async ({ page, request }) => {
      const r = await record(request, "holdings", `絵のある物 ${name}`);
      await record(request, "holdings", `絵のない物 ${name}`);
      await page.route("**/holding-pictures/e2e", (route) => route.fulfill({ contentType: "image/png", body: PICTURE }));
      await pictured(page, r.id, "e2e");
      await page.getByLabel("物を探す").fill(name);
      const small = row(page, `絵のある物 ${name}`).locator("img");
      await expect(small).toHaveAttribute("src", "/holding-pictures/e2e");
      const box = await small.boundingBox();
      expect([box?.width, box?.height]).toEqual([48, 48]);
      await expect(row(page, `絵のない物 ${name}`)).toBeVisible();
      await expect(row(page, `絵のない物 ${name}`).locator("img")).toHaveCount(0);

      await row(page, `絵のある物 ${name}`).getByRole("link").click();
      const open = detail(page).getByRole("button", { name: `絵のある物 ${name}の画像を拡大する` });
      await expect(open).toBeVisible();
      const wide = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
      expect(wide).toBeLessThanOrEqual(0);
      await open.click();
      const dialog = page.getByRole("dialog", { name: `絵のある物 ${name}` });
      await expect(dialog.getByRole("img")).toHaveAttribute("src", "/holding-pictures/e2e");
      await page.keyboard.press("Escape");
      await expect(dialog).toBeHidden();
      await expect(open).toBeFocused();
    });
  });
}

test("a thing whose picture cannot be read shows as one without", async ({ page, request }) => {
  const r = await record(request, "holdings", "絵の消えた物");
  await pictured(page, r.id, "gone");
  await page.getByLabel("物を探す").fill("絵の消えた物");
  await expect(row(page, "絵の消えた物")).toBeVisible();
  await expect(row(page, "絵の消えた物").locator("img")).toHaveCount(0);
  await row(page, "絵の消えた物").getByRole("link").click();
  await expect(detail(page).getByLabel("題名")).toHaveValue("絵の消えた物");
  await expect(detail(page).locator(".thing-picture")).toHaveCount(0);
});
