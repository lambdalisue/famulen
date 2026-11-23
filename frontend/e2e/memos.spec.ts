import { expect, test } from "@playwright/test";
import { acceptDialogs, detail, record, row, toast, visit } from "./support";

test("a memo added from the list opens and stays after a reload", async ({ page }) => {
  await visit(page, "/memos", "メモ");
  await page.getByRole("button", { name: "メモを書く" }).click();
  await page.getByRole("dialog").getByLabel("題名").fill("引っ越しの段取り");
  await page.getByRole("dialog").getByRole("button", { name: "足す", exact: true }).click();
  await expect(page).toHaveURL(/\/memos\?id=/);
  await detail(page).getByRole("button", { name: "本文を書く" }).click();
  await detail(page).getByLabel("本文", { exact: true }).fill("# 段取り\n\n- [x] 段ボールを頼む");
  await detail(page).getByRole("button", { name: "保存" }).click();
  await expect(toast(page)).toHaveText("保存した");
  await page.reload();
  await expect(detail(page).getByLabel("題名")).toHaveValue("引っ越しの段取り");
  await expect(detail(page).locator(".markdown-body input[type=checkbox]")).toBeChecked();
  await expect(row(page, "引っ越しの段取り")).toContainText("段ボールを頼む");
});

test("text being written is previewed, and left as it was when given up", async ({ page, request }) => {
  const memo = await record(request, "memos", "読みたい本", { notes: "砂の女" });
  acceptDialogs(page);
  await visit(page, `/memos?id=${memo.id}`, "メモ · 読みたい本");
  await detail(page).getByRole("button", { name: "本文を書く" }).click();
  await detail(page).getByLabel("本文", { exact: true }).fill("## 百年の孤独");
  await detail(page).getByRole("button", { name: "プレビュー" }).click();
  await expect(detail(page).getByRole("heading", { name: "百年の孤独" })).toBeVisible();
  await detail(page).getByRole("button", { name: "やめる" }).click();
  await expect(detail(page).getByText("砂の女")).toBeVisible();
});

test("deleting a memo asks first, then leaves for the memo list without it", async ({ page, request }) => {
  const memo = await record(request, "memos", "捨てるメモ", { notes: "x" });
  acceptDialogs(page);
  await visit(page, `/memos?id=${memo.id}`, "メモ · 捨てるメモ");
  await page.getByRole("button", { name: "捨てるメモを消す" }).click();
  await expect(toast(page)).toHaveText("消した");
  await expect(page).toHaveURL(/\/memos$/);
  await expect(row(page, "捨てるメモ")).toHaveCount(0);
});

test("opening a memo puts it beside the composer", async ({ page, request }) => {
  const memo = await record(request, "memos", "旅の持ち物");
  await visit(page, `/memos?id=${memo.id}`, "メモ · 旅の持ち物");
  await expect(page.locator("#selection")).toContainText("メモ：旅の持ち物");
});
