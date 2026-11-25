import { expect, test } from "@playwright/test";
import { call, detail, field, record, row, toast, visit } from "./support";

const me = (as: string) => [{ name: "Relations", items: [{ text: "me", fields: [{ name: "As", value: as }] }] }];

test("a person opens beside the list, with where they belong, and the place names its people", async ({ page, request }) => {
  const org = await record(request, "parties", "山田商事", { fields: { Kind: "org" } });
  const person = await record(request, "parties", "山田花子さん", {
    fields: { Kind: "person", Aliases: "はなちゃん, 花子" },
    sections: [{ name: "Member of", items: [{ text: `[[records/parties/${org.id}|山田商事]]` }] }],
  });
  await visit(page, `/parties?id=${person.id}`, "相手 · 山田花子さん");
  await expect(field(page, "ほかの名前")).toContainText("はなちゃん");
  await detail(page).getByRole("link", { name: "山田商事" }).first().click();
  await expect(page).toHaveURL(new RegExp(`id=${org.id}$`));
  await expect(detail(page).locator(".rec-links").filter({ hasText: "属する人" })).toContainText("山田花子さん");
});

test("a person has several relations to the owner, shown in the list", async ({ page, request }) => {
  const p = await record(request, "parties", "佐藤一郎さん", { fields: { Kind: "person" }, sections: me("部下, 友人") });
  await visit(page, `/parties?id=${p.id}`, "相手 · 佐藤一郎さん");
  await expect(row(page, "佐藤一郎さん")).toContainText("部下");
  await expect(row(page, "佐藤一郎さん")).toContainText("友人");
});

test("a relation between two people is added and shows on both", async ({ page, request }) => {
  const a = await record(request, "parties", "伊藤夫さん", { fields: { Kind: "person" } });
  const b = await record(request, "parties", "伊藤妻さん", { fields: { Kind: "person" } });
  await visit(page, `/parties?id=${a.id}`, "相手 · 伊藤夫さん");
  await detail(page).getByRole("button", { name: "＋ 関係" }).click();
  await detail(page).getByRole("button", { name: "相手を選ぶ" }).click();
  await page.getByRole("dialog").getByLabel("探す").fill("伊藤妻");
  await page.getByRole("dialog").getByRole("button", { name: /伊藤妻さん/ }).click();
  await expect(toast(page)).toHaveText("保存した");
  await expect(detail(page).locator(".rec-item")).toContainText("伊藤妻さん");
  await page.goto(`/parties?id=${b.id}`);
  await expect(detail(page).locator(".rec-links").filter({ hasText: "関係" })).toContainText("伊藤夫さん");
});

test("a loose group is added by hand, and any name finds a person", async ({ page, request }) => {
  await record(request, "parties", "鈴木次郎さん", { fields: { Kind: "person", Aliases: "じろちゃん" } });
  await visit(page, "/parties", "相手");
  await page.getByRole("button", { name: "グループを足す" }).click();
  await page.getByRole("dialog").getByLabel("名前").fill("キーボード仲間");
  await page.getByRole("dialog").getByRole("button", { name: "足す", exact: true }).click();
  await expect(field(page, "種別")).toContainText("グループ");
  await page.getByRole("button", { name: "詳細を閉じる" }).click();
  await page.getByLabel("相手を探す").fill("じろちゃん");
  await expect(row(page, "鈴木次郎さん")).toBeVisible();
  await expect(row(page, "キーボード仲間")).toHaveCount(0);
});

test("an inferred account waits for the owner, and 同じ人 confirms it", async ({ page, request }) => {
  const p = await record(request, "parties", "高橋三郎さん", { fields: { Kind: "person" } });
  await call(request, "RecordService", "LinkAccount", {
    party: p.id,
    service: "github",
    handle: "saburo",
    confirmed: false,
    how: "メールの署名から",
  });
  await visit(page, `/parties?id=${p.id}`, "相手 · 高橋三郎さん");
  await detail(page).getByRole("button", { name: "同じ人" }).click();
  await expect(toast(page)).toHaveText("同じ人にした");
  await expect(detail(page).getByRole("button", { name: "同じ人" })).toHaveCount(0);
  await expect(detail(page).getByText("saburo")).toBeVisible();
});

test("what the secretary remembers about a party is listed on its page", async ({ page, request }) => {
  const p = await record(request, "parties", "田中四郎さん", { fields: { Kind: "person" } });
  await call(request, "MemoryService", "WriteFile", {
    path: "long/semantic/e2e-tanaka-sweets.md",
    text: `---\nid: e2e-p1\nsummary: 甘いものが好き\nsrc: said\n---\n\n[[records/parties/${p.id}|田中四郎さん]] は甘いものが好き。\n`,
  });
  await call(request, "MemoryService", "WriteFile", {
    path: "long/prospective/e2e-tanaka-papers.md",
    text: `---\nid: e2e-p2\nsummary: 資料を金曜までに送る\nsrc: inferred\nparty: ${p.id}\n---\n\n田中さんに資料を金曜までに送る。\n`,
  });
  await call(request, "MemoryService", "WriteFile", {
    path: "long/prospective/e2e-someone-else.md",
    text: "---\nid: e2e-p3\nsummary: 別の人への約束\nsrc: said\n---\n\n別の人への約束。\n",
  });
  await visit(page, `/parties?id=${p.id}`, "相手 · 田中四郎さん");

  const section = page.locator("section").filter({ has: page.getByRole("heading", { name: /が覚えていること$/ }) });
  await expect(section.locator(".item")).toHaveText([/あなたが言った.*甘いものが好き/, /推し量り.*資料を金曜までに送る/]);
});

test("a party nothing is remembered about says so", async ({ page, request }) => {
  const p = await record(request, "parties", "南の商会", { fields: { Kind: "org" } });
  await visit(page, `/parties?id=${p.id}`, "相手 · 南の商会");
  await expect(page.getByText("まだ覚えていることはない")).toBeVisible();
});

test("whoever rests is put away under 休み, and is found by words", async ({ page, request }) => {
  await record(request, "parties", "休田五郎さん", { fields: { Kind: "person", Status: "resting" } });
  await record(request, "parties", "今田六郎さん", { fields: { Kind: "person" } });
  await visit(page, "/parties", "相手");
  await expect(row(page, "今田六郎さん")).toBeVisible();
  await expect(row(page, "休田五郎さん")).toBeHidden();
  await page.getByLabel("相手を探す").fill("休田");
  await expect(row(page, "休田五郎さん")).toBeVisible();
});
