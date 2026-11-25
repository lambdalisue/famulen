import type { APIRequestContext, Page } from "@playwright/test";
import { expect, test } from "@playwright/test";
import { call, visit } from "./support";

type Said = { id: string; text: string; thread?: string };

const post = (request: APIRequestContext, text: string, replyTo = "0") => call<Said>(request, "ConsoleService", "PostMessage", { text, replyTo });
const inTalk = (page: Page, text: string) => page.locator("#messages").getByTestId("message").filter({ hasText: text });
const thread = (page: Page) => page.getByRole("region", { name: "スレッド" });
const replying = (page: Page) => page.getByTestId("replying");

async function send(page: Page, text: string) {
  await page.getByPlaceholder(/に頼む$/).fill(text);
  await page.getByRole("button", { name: "送る", exact: true }).click();
  await expect(page.getByPlaceholder(/に頼む$/)).toHaveValue("");
}

test("a message is replied to, its reply quotes it, and the thread holds both and takes more", async ({ page, request }) => {
  const root = await post(request, "e2e 返信 元の **報告**");
  await visit(page, "/", /^ホーム/);
  await inTalk(page, "元の 報告").getByRole("button", { name: "返信" }).click();
  await expect(replying(page)).toContainText("e2e 返信 元の 報告");
  await expect(page.getByPlaceholder(/に頼む$/)).toBeFocused();
  await send(page, "e2e 返信 一つ目");
  await expect(replying(page)).toHaveCount(0);

  const reply = inTalk(page, "e2e 返信 一つ目");
  await expect(reply.locator(".quote")).toContainText("元の 報告");
  const opener = page.getByRole("button", { name: "返信 1 件" });
  await opener.click();
  const said = thread(page).getByTestId("message");
  await expect(said).toHaveCount(2);
  await expect(said.locator(".quote"), "what the thread begins with is there to see").toHaveCount(0);

  await send(page, "e2e 返信 二つ目");
  await expect(said).toHaveCount(3);
  await expect(page.getByRole("button", { name: "返信 2 件" })).toBeVisible();
  // The secretary's word is in the thread and in the conversation.
  await post(request, "e2e 返信 秘書から", root.id);
  await expect(said).toHaveCount(4);
  await expect(inTalk(page, "e2e 返信 秘書から")).toHaveCount(1);

  await thread(page).getByRole("button", { name: "スレッドを閉じる" }).click();
  await expect(thread(page)).toHaveCount(0);
  await send(page, "e2e 返信 外へ");
  await expect(inTalk(page, "e2e 返信 外へ").locator(".quote")).toHaveCount(0);
  await page.reload();
  await inTalk(page, "e2e 返信 二つ目").getByRole("button", { name: "スレッドを開く" }).click();
  await expect(said).toHaveCount(4);
});

test("a reply is given up, and what was typed stays", async ({ page, request }) => {
  await post(request, "e2e やめる 元");
  await visit(page, "/", /^ホーム/);
  await page.getByPlaceholder(/に頼む$/).fill("e2e やめる 書きかけ");
  await inTalk(page, "e2e やめる 元").getByRole("button", { name: "返信" }).click();
  await page.getByRole("button", { name: "返信をやめる" }).click();
  await expect(replying(page)).toHaveCount(0);
  await expect(page.getByPlaceholder(/に頼む$/)).toHaveValue("e2e やめる 書きかけ");
  await expect(page.getByPlaceholder(/に頼む$/)).toBeFocused();
});

test("a quote leads to its message, read back to when it is older than what was read", async ({ page, request }) => {
  const root = await post(request, "e2e 遠い 元の発言");
  for (let i = 0; i < 300; i++) await post(request, `e2e 遠い ${i} です。${"長い話。".repeat(i % 9)}`);
  await post(request, "e2e 遠い 返信", root.id);
  await visit(page, "/", /^ホーム/);
  const quote = inTalk(page, "e2e 遠い 返信").locator(".quote");
  await expect(quote).toContainText("e2e 遠い 元の発言");
  await expect(inTalk(page, "元の発言").filter({ hasNot: page.locator(".quote") })).toHaveCount(0);
  await quote.click();
  const at = page.locator(`#messages [data-id="${root.id}"]`);
  const place = () => at.evaluate((el) => el.getBoundingClientRect().top - (el.closest("#messages")?.getBoundingClientRect().top ?? 0));
  await expect(at).toBeInViewport();
  await expect.poll(place).toBeCloseTo(8, -1);
  // It stays there as what is around it is drawn and something new comes.
  await post(request, "e2e 遠い あとから");
  await expect(inTalk(page, "e2e 遠い あとから")).toHaveCount(1);
  await page.waitForTimeout(300);
  expect(Math.abs((await place()) - 8)).toBeLessThan(5);
});

test("what is read stays where it is as a thread opens, is answered in and closes", async ({ page, request }) => {
  const root = await post(request, "e2e 並ぶ 元");
  for (let i = 0; i < 30; i++) await post(request, `e2e 並ぶ ${i} です。${"長い話。".repeat(i % 9)}`);
  await post(request, "e2e 並ぶ 返信", root.id);
  await visit(page, "/", /^ホーム/);
  // Threads of the other tests come into sight as older messages are read.
  const at = page.locator(`#messages [data-id="${root.id}"]`);
  const opens = at.getByRole("button", { name: "返信 1 件" });
  await page.locator("#messages").hover();
  await expect(async () => {
    await page.mouse.wheel(0, -3000);
    await expect(opens).toBeAttached({ timeout: 200 });
  }).toPass();
  await at.evaluate((el) => el.scrollIntoView());
  await page.waitForTimeout(200);
  const top = () => at.evaluate((el) => el.getBoundingClientRect().top);
  const was = await top();
  await opens.click();
  await send(page, "e2e 並ぶ スレッドで");
  await expect(thread(page).getByTestId("message")).toHaveCount(3);
  expect(Math.abs((await top()) - was)).toBeLessThan(5);
  await thread(page).getByRole("button", { name: "スレッドを閉じる" }).click();
  await page.waitForTimeout(200);
  expect(Math.abs((await top()) - was)).toBeLessThan(5);
});
