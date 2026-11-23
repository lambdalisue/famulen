import { expect, test } from "@playwright/test";
import { agree, call, detail, record, toast, visit } from "./support";

const back = (page: Parameters<typeof detail>[0], text: string) => detail(page).locator(".rec-links").filter({ hasText: text });

test("typing [[ in a memo picks a record with the keyboard, and the record shows the memo", async ({ page, request }) => {
  const party = await record(request, "parties", "リンク先の田中さん", { fields: { Kind: "person" } });
  const memo = await record(request, "memos", "手土産の覚え");
  await visit(page, `/memos?id=${memo.id}`, "メモ · 手土産の覚え");
  await detail(page).getByRole("button", { name: "本文を書く" }).click();
  const body = detail(page).getByLabel("本文", { exact: true });
  await body.pressSequentially("和菓子は[[");
  const picker = page.getByRole("dialog", { name: "記録へのリンク" });
  await picker.getByLabel("探す").fill("リンク先の");
  await expect(picker.getByRole("button", { name: /リンク先の田中さん/ })).toHaveAttribute("aria-current", "true");
  await picker.getByLabel("探す").press("Enter");
  await expect(body).toBeFocused();
  await body.pressSequentially("に");
  await expect(body).toHaveValue(`和菓子は[[records/parties/${party.id}|リンク先の田中さん]]に`);
  await body.press("Control+Enter");
  await expect(toast(page)).toHaveText("保存した");
  await detail(page).locator(".md-link", { hasText: "リンク先の田中さん" }).click();
  await expect(page).toHaveURL(new RegExp(`/parties\\?id=${party.id}`));
  await expect(back(page, "触れているメモ")).toContainText("手土産の覚え");
});

test("related records of any kind are added and removed, and show on both", async ({ page, request }) => {
  const memo = await record(request, "memos", "関わるメモ");
  const party = await record(request, "parties", "関わる相手さん", { fields: { Kind: "person" } });
  await visit(page, `/parties?id=${party.id}`, "相手 · 関わる相手さん");
  await detail(page).getByRole("button", { name: "関わるものを足す" }).click();
  await page.getByRole("dialog").getByLabel("探す").fill("関わるメモ");
  await page.getByRole("dialog").getByRole("button", { name: /関わるメモ/ }).click();
  await expect(toast(page)).toHaveText("保存した");
  await detail(page).getByRole("link", { name: "関わるメモ" }).click();
  await expect(page).toHaveURL(new RegExp(`/memos\\?id=${memo.id}`));
  await expect(back(page, "関わる相手")).toContainText("関わる相手さん");
  await page.goto(`/parties?id=${party.id}`);
  await detail(page).getByRole("button", { name: "関わるメモを関わるものから外す" }).click();
  await expect(detail(page).getByRole("link", { name: "関わるメモ" })).toHaveCount(0);
});

test("a link follows a rename, and one whose record is gone says so", async ({ page, request }) => {
  const a = await record(request, "memos", "前の名前");
  const b = await record(request, "tasks", "消える仕事");
  const memo = await record(request, "memos", "つなぐメモ", {
    notes: `[[${a.code}]] と [[${b.code}]]`,
    fields: { Related: b.code },
  });
  await call(request, "RecordService", "SaveRecord", { kind: "memos", id: a.id, title: "今の名前" });
  await call(request, "RecordService", "DeleteRecord", { kind: "tasks", id: b.id });
  await visit(page, `/memos?id=${memo.id}`, "メモ · つなぐメモ");
  await expect(detail(page).locator(".md-link", { hasText: "今の名前" })).toBeVisible();
  await expect(detail(page).locator(".md-link-missing", { hasText: "消える仕事" })).toBeVisible();
  await expect(detail(page).locator(".tag.missing")).toContainText("消える仕事（もうない）");
});

test("a link shown as code in a line of text leads nowhere", async ({ page, request }) => {
  const memo = await record(request, "memos", "見本の先");
  const link = `[[records/memos/${memo.id}|見本の先]]`;
  const party = await record(request, "parties", "見本の相手さん", { fields: { Kind: "person" } });
  await call(request, "RecordService", "SaveRecord", {
    kind: "parties",
    id: party.id,
    log: [`\`${link}\` と ${link}`],
  });
  await visit(page, `/parties?id=${party.id}`, "相手 · 見本の相手さん");
  await expect(detail(page).getByText(`\`${link}\` と`)).toBeVisible();
  await expect(detail(page).getByRole("link", { name: "見本の先" })).toHaveCount(1);
});

test("a memory that links to a record shows on it, and leads back", async ({ page, request }) => {
  const memo = await record(request, "memos", "記憶が触れるメモ");
  await call(request, "MemoryService", "WriteFile", {
    path: "long/semantic/e2e-links.md",
    text: `---\nid: e2e-l1\nsummary: メモの覚え\nsrc: said\n---\n\n[[${memo.code}]] を見る。\n`,
  });
  await visit(page, `/memos?id=${memo.id}`, "メモ · 記憶が触れるメモ");
  await back(page, "触れている記憶").getByRole("link", { name: "メモの覚え" }).click();
  await expect(page).toHaveURL(/\/memory\/file/);
  await page.getByRole("link", { name: "記憶が触れるメモ" }).click();
  await expect(page).toHaveURL(new RegExp(`id=${memo.id}`));
});
