import { expect, test } from "@playwright/test";
import { agree, call, dayFromToday, detail, field, record, row, toast, visit } from "./support";

const deadline = (what: string, days: number) => [{ name: "Deadlines", items: [{ text: what, fields: [{ name: "On", value: dayFromToday(days) }] }] }];

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
      await page.getByLabel("分類で絞る").selectOption("甲類");
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
