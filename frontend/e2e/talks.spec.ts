import { type APIRequestContext, expect, type Page, test } from "@playwright/test";
import { faint } from "./legible";
import { call, expectView, visit } from "./support";

// Every test begins from the list, as an owner who never chose otherwise does.
test.use({ storageState: { cookies: [], origins: [] } });

type Sent = { id: string; conversation: string };

const list = (page: Page) => page.getByRole("region", { name: "話の一覧" });
const row = (page: Page, title: string) => list(page).getByRole("listitem").filter({ hasText: title });
const said = (page: Page) => page.getByRole("log", { name: "この話の発言" });
const dest = (page: Page) => page.getByRole("group", { name: "送り先" });
const box = (page: Page) => page.getByPlaceholder(/に頼む$/);

const begin = (request: APIRequestContext, text: string) =>
  call<Sent>(request, "ConsoleService", "Send", { text, workgroup: "all", newConversation: true });
/** The secretary says something, in the thread of the owner's message and so in its conversation. */
const answer = (request: APIRequestContext, text: string, to: Sent) => call(request, "ConsoleService", "PostMessage", { text, replyTo: to.id });
const settled = (request: APIRequestContext) =>
  expect.poll(async () => (await call<{ state: string }>(request, "ConsoleService", "GetStatus")).state, { timeout: 20_000 }).not.toBe("running");

async function send(page: Page, text: string) {
  await box(page).fill(text);
  await page.getByRole("button", { name: "送る", exact: true }).click();
  await expect(box(page)).toHaveValue("");
}

async function heard(request: APIRequestContext, conversation: string) {
  const { messages } = await call<{ messages?: { text: string }[] }>(request, "ConsoleService", "ListMessages", { conversation });
  return (messages ?? []).map((m) => m.text);
}

test("what is written from the list begins a conversation, and goes on in it without a reply", async ({ page, request }) => {
  await visit(page, "/", /^ホーム/);
  await expect(list(page)).toBeVisible();
  await expect(dest(page).getByRole("button", { name: "新しい話" })).toHaveAttribute("aria-pressed", "true");
  await send(page, "e2e 話 見積もりを比べて");
  await expect(page.locator(".talk-name")).toContainText("e2e 話 見積もりを比べて");
  await expect(dest(page).getByRole("button", { name: "続き" })).toHaveAttribute("aria-pressed", "true");
  await expect(dest(page)).toContainText("e2e 話 見積もりを比べて");
  await send(page, "e2e 話 二つに絞って");
  await expect(said(page).getByTestId("message")).toHaveCount(2);

  const { conversations } = await call<{ conversations: { id: string; title: string }[] }>(request, "ConsoleService", "ListConversations");
  const mine = conversations.find((c) => c.title === "e2e 話 見積もりを比べて");
  expect(await heard(request, mine?.id ?? "")).toEqual(["e2e 話 見積もりを比べて", "e2e 話 二つに絞って"]);

  // It is still the one open after a reload.
  await page.reload();
  await expectView(page, /^ホーム/);
  await expect(said(page).getByTestId("message")).toHaveCount(2);
});

test("the list tells the title, the last word and what is unread, and opening one has it read everywhere", async ({ page, request }) => {
  const a = await begin(request, "e2e 一覧 甲の話\n詳しく");
  await settled(request);
  await answer(request, "e2e 一覧 甲の答え", a);
  await visit(page, "/", /^ホーム/);
  const mine = row(page, "e2e 一覧 甲の話");
  await expect(mine).toContainText("e2e 一覧 甲の答え");
  await expect(mine.getByLabel("未読 1")).toBeVisible();
  await expect(list(page).getByRole("region", { name: "新しく届いた" })).toContainText("甲の話");

  const other = await page.context().newPage();
  await visit(other, "/", /^ホーム/);
  await mine.getByRole("button").click();
  await expect(said(page)).toContainText("e2e 一覧 甲の答え");
  await expect(said(page).getByRole("separator")).toHaveText("ここから新しい");
  await expect(row(other, "e2e 一覧 甲の話")).toBeVisible();
  await expect(row(other, "e2e 一覧 甲の話").getByLabel(/^未読/)).toHaveCount(0);
  await page.getByRole("button", { name: /^話の一覧へ/ }).click();
  await expect(row(page, "e2e 一覧 甲の話").getByLabel(/^未読/)).toHaveCount(0);
});

test("what comes to another conversation is told over the box, and does not turn the page", async ({ page, request }) => {
  const a = await begin(request, "e2e 届く 甲");
  const b = await begin(request, "e2e 届く 乙");
  await settled(request);
  await visit(page, "/", /^ホーム/);
  await row(page, "e2e 届く 甲").getByRole("button").click();
  await expect(said(page)).toContainText("e2e 届く 甲");
  await answer(request, "e2e 届く 甲の答え", a);
  await expect(said(page)).toContainText("e2e 届く 甲の答え");
  await expect(page.locator(".arrival")).toHaveCount(0);

  await answer(request, "e2e 届く 乙の答え", b);
  await expect(page.locator(".arrival")).toContainText("e2e 届く 乙");
  await expect(said(page)).not.toContainText("乙の答え");
  await expect(page.getByRole("button", { name: /^話の一覧へ（ほかに \d+ 件が待っている）$/ })).toBeVisible();
  await page.locator(".arrival").getByRole("button", { name: "開く" }).click();
  await expect(said(page)).toContainText("e2e 届く 乙の答え");
  await expect(page.locator(".arrival")).toHaveCount(0);
});

test("a request is answered in the conversation that asked, and the list leads to it", async ({ page, request }) => {
  await begin(request, "e2e 黙る");
  await settled(request);
  await begin(request, "e2e 尋ねる e2e-asks");
  await expect.poll(async () => (await call<{ approvals?: unknown[] }>(request, "ConsoleService", "GetInbox")).approvals?.length ?? 0).toBeGreaterThan(0);
  await visit(page, "/", /^ホーム/);
  const asking = list(page).getByRole("region", { name: "あなたの答え待ち" }).getByRole("listitem");
  await expect(asking).toContainText("e2e 尋ねる");
  await expect(asking).toContainText("答え待ち 1");
  const card = page.getByTestId("approval");
  await expect(card).toBeHidden();
  await row(page, "e2e 黙る").getByRole("button").click();
  await expect(card).toBeHidden();
  await page.getByRole("button", { name: /^話の一覧へ/ }).click();
  await asking.getByRole("button").click();
  await expect(card).toBeVisible();
  await card.getByRole("button", { name: "却下" }).click();
  await expect(card).toHaveCount(0);
});

test("what is being written is kept for each conversation, and goes along when the owner turns where it goes", async ({ page, request }) => {
  await begin(request, "e2e 書きかけ 甲");
  await visit(page, "/", /^ホーム/);
  await box(page).fill("新しい話への書きかけ");
  await row(page, "e2e 書きかけ 甲").getByRole("button").click();
  await expect(box(page)).toHaveValue("");
  await box(page).fill("甲への書きかけ");
  await page.getByRole("button", { name: /^話の一覧へ/ }).click();
  await expect(box(page)).toHaveValue("新しい話への書きかけ");
  await row(page, "e2e 書きかけ 甲").getByRole("button").click();
  await expect(box(page)).toHaveValue("甲への書きかけ");

  // What was begun there before is kept along with it.
  await dest(page).getByRole("button", { name: "新しい話" }).click();
  await expect(box(page)).toHaveValue("新しい話への書きかけ\n甲への書きかけ");
  await expect(page.locator(".talk-name")).toContainText("新しい話");
  await dest(page).getByRole("button", { name: "続き" }).click();
  await expect(box(page)).toHaveValue("新しい話への書きかけ\n甲への書きかけ");
  await expect(page.locator(".talk-name")).toContainText("e2e 書きかけ 甲");
});

test("a message is quoted, and what is written goes on in the same conversation", async ({ page, request }) => {
  const a = await begin(request, "e2e 引く 甲");
  await settled(request);
  await answer(request, "e2e 引く 甲の答え", a);
  await visit(page, "/", /^ホーム/);
  await row(page, "e2e 引く 甲").getByRole("button").click();
  await said(page).getByTestId("message").filter({ hasText: "甲の答え" }).getByRole("button", { name: "返信" }).click();
  await expect(page.getByTestId("replying")).toContainText("甲の答え");
  await send(page, "e2e 引く 続き");
  await expect(page.locator(".talk-name")).toContainText("e2e 引く 甲");
  const mine = said(page).getByTestId("message").filter({ hasText: "e2e 引く 続き" });
  await expect(mine.locator(".quote")).toContainText("甲の答え");
  expect(await heard(request, a.conversation)).toContain("e2e 引く 続き");
});

test("what is written stays where it was while the owner goes elsewhere as it is sent", async ({ page, request }) => {
  const a = await begin(request, "e2e 送る間 甲");
  await begin(request, "e2e 送る間 乙");
  await settled(request);
  await visit(page, "/", /^ホーム/);
  await row(page, "e2e 送る間 乙").getByRole("button").click();
  await box(page).fill("乙への書きかけ");
  await page.getByRole("button", { name: /^話の一覧へ/ }).click();
  await row(page, "e2e 送る間 甲").getByRole("button").click();
  let letGo = () => {};
  const held = new Promise<void>((resolve) => {
    letGo = resolve;
  });
  await page.route(/ConsoleService\/Send$/, async (route) => {
    await held;
    await route.continue();
  });
  await box(page).fill("甲への言葉");
  await page.getByRole("button", { name: "送る", exact: true }).click();
  await page.getByRole("button", { name: /^話の一覧へ/ }).click();
  await row(page, "e2e 送る間 乙").getByRole("button").click();
  letGo();
  await expect.poll(() => heard(request, a.conversation)).toContain("甲への言葉");
  await expect(box(page)).toHaveValue("乙への書きかけ");
  await expect(page.locator(".talk-name")).toContainText("e2e 送る間 乙");
  await page.getByRole("button", { name: /^話の一覧へ/ }).click();
  await row(page, "e2e 送る間 甲").getByRole("button").click();
  await expect(box(page)).toHaveValue("");
});

test("a conversation another workgroup does not show is left as that is chosen", async ({ page, request }) => {
  const { workgroups } = await call<{ workgroups: { id: string; name: string }[] }>(request, "WorkgroupService", "ListWorkgroups");
  const work = workgroups.find((w) => w.name === "仕事")?.id;
  await call(request, "ConsoleService", "Send", { text: "e2e 仕事の話", workgroup: work, newConversation: true });
  await visit(page, "/", /^ホーム/);
  await row(page, "e2e 仕事の話").getByRole("button").click();
  await expect(said(page)).toContainText("e2e 仕事の話");
  const choose = (name: string) => page.getByRole("group", { name: "ワークグループ", exact: true }).getByRole("button", { name }).click();
  await choose("個人");
  await expect(list(page)).toBeVisible();
  await expect(row(page, "e2e 仕事の話")).toHaveCount(0);
  await choose("すべて");
});

test("in one line, what is written after another goes on with it", async ({ page, request }) => {
  await visit(page, "/", /^ホーム/);
  await page.getByRole("button", { name: "一続きで見る" }).click();
  await send(page, "e2e 続ける 一つ目");
  await expect(dest(page)).toContainText("e2e 続ける 一つ目");
  await send(page, "e2e 続ける 二つ目");
  const { conversations } = await call<{ conversations: { id: string; title: string }[] }>(request, "ConsoleService", "ListConversations");
  const mine = conversations.find((c) => c.title === "e2e 続ける 一つ目");
  expect(await heard(request, mine?.id ?? "")).toContain("e2e 続ける 二つ目");
});

test("all that was said is seen in one line, and a choice of that stays", async ({ page, request }) => {
  await begin(request, "e2e 一続き 甲");
  await visit(page, "/", /^ホーム/);
  await page.getByRole("button", { name: "一続きで見る" }).click();
  await expect(page.locator("#messages")).toContainText("e2e 一続き 甲");
  await page.reload();
  await expectView(page, /^ホーム/);
  await expect(page.locator("#messages")).toContainText("e2e 一続き 甲");
  await page.getByRole("button", { name: "話の一覧へ" }).click();
  await expect(list(page)).toBeVisible();
});

test("on a phone the conversations open over the page, and where it goes is told", async ({ page, request }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const a = await begin(request, "e2e 携帯の話 甲");
  await settled(request);
  await answer(request, "e2e 携帯の話 答え", a);
  for (const colorScheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme, reducedMotion: "reduce" });
    await visit(page, "/", /^ホーム/);
    await page.getByRole("button", { name: "会話を引き上げる" }).click();
    if (colorScheme === "light") {
      await expect(row(page, "e2e 携帯の話 甲")).toBeVisible();
      expect(await faint(page)).toEqual([]);
      await row(page, "e2e 携帯の話 甲").getByRole("button").click();
    }
    await expect(said(page)).toContainText("e2e 携帯の話 答え");
    await expect(dest(page)).toContainText("e2e 携帯の話 甲");
    for (const name of ["続き", "新しい話"]) {
      expect((await dest(page).getByRole("button", { name }).boundingBox())?.width).toBeGreaterThanOrEqual(44);
    }
    expect(await faint(page)).toEqual([]);
  }
  await page.getByRole("button", { name: "閉じる" }).click();
  await expect(page.locator(".m-peek")).toContainText("e2e 携帯の話 甲");
});
