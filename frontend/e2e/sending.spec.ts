import { expect, type Page, test } from "@playwright/test";
import { call, holdBack, toast, visit } from "./support";

type Inbox = { approvals?: { id: string; status: string }[] };

async function askApproval(page: Page, request: Parameters<typeof call>[0]) {
  await call(request, "ConsoleService", "Send", { text: "二度押しの話 e2e-bash", workgroup: "all" });
  // The secretary may still be at the end of what was asked before.
  const pending = async () =>
    (await call<Inbox>(request, "ConsoleService", "GetInbox")).approvals?.some((a) => a.status === "pending") ?? false;
  await expect.poll(pending, { timeout: 30_000 }).toBe(true);
  await visit(page, "/", /^ホーム/);
  const card = page.getByTestId("approval");
  await expect(card).toBeVisible();
  return card;
}

test.afterEach(async ({ request }) => {
  for (const a of (await call<Inbox>(request, "ConsoleService", "GetInbox")).approvals ?? []) {
    if (a.status === "pending") await call(request, "ConsoleService", "AnswerApproval", { id: a.id, answer: "APPROVAL_ANSWER_DENY" });
  }
});

test("an approval pressed again and again while it is sent goes once, and its other answers wait with it", async ({ page, request }) => {
  const card = await askApproval(page, request);
  const sent = await holdBack(page, "AnswerApproval");
  const deny = card.getByRole("button", { name: "却下" });
  await deny.dblclick();
  await expect(deny).toBeDisabled();
  await expect(deny).toHaveAttribute("aria-busy", "true");
  await expect(card.getByRole("button", { name: "承認", exact: true })).toBeDisabled();
  await expect(card.getByRole("button", { name: "今後は常に許可" })).toBeDisabled();
  await deny.focus();
  await page.keyboard.press("Enter");
  await page.keyboard.press("Space");
  await sent.arrived(1);
  sent.letGo();
  await expect(card).toHaveCount(0);
  expect(sent.calls).toBe(1);
});

test("an answer that does not reach the backend is said, and can be given again", async ({ page, request }) => {
  const card = await askApproval(page, request);
  const sent = await holdBack(page, "AnswerApproval");
  sent.failing(true);
  const deny = card.getByRole("button", { name: "却下" });
  await deny.click();
  await expect(deny).toBeDisabled();
  await sent.arrived(1);
  sent.letGo();
  await expect(toast(page)).toContainText("e2e-届かなかった");
  await expect(deny).toBeEnabled();
  await expect(card.getByRole("button", { name: "承認", exact: true })).toBeEnabled();
  sent.failing(false);
  await deny.click();
  await expect(card).toHaveCount(0);
  expect(sent.calls).toBe(2);
});

test("what is said is sent once, however often it is sent while it goes", async ({ page }) => {
  await visit(page, "/", /^ホーム/);
  const sent = await holdBack(page, "Send");
  const say = page.getByPlaceholder(/に頼む$/);
  await say.fill("e2e-一度だけ届く");
  const send = page.getByRole("button", { name: "送る", exact: true });
  await send.dblclick();
  await expect(send).toBeDisabled();
  await say.press("ControlOrMeta+Enter");
  await say.press("ControlOrMeta+Enter");
  await sent.arrived(1);
  sent.letGo();
  await expect(page.getByTestId("message").filter({ hasText: "e2e-一度だけ届く" })).toHaveCount(1);
  await expect(say).toHaveValue("");
  expect(sent.calls).toBe(1);
});

test("a dialog's form sent with Enter again and again goes once", async ({ page }) => {
  await visit(page, "/settings/workgroups", /ワークグループ/);
  const sent = await holdBack(page, "AddWorkgroup");
  await page.getByRole("button", { name: "ワークグループを足す" }).click();
  const dialog = page.getByRole("dialog");
  const name = dialog.getByRole("textbox").first();
  await name.fill("e2e-二度押し");
  await name.press("Enter");
  await name.press("Enter");
  await name.press("Enter");
  const add = dialog.getByRole("button", { name: "足す", exact: true });
  await expect(add).toBeDisabled();
  await expect(add).toHaveAttribute("aria-busy", "true");
  await sent.arrived(1);
  sent.letGo();
  await expect(dialog).toHaveCount(0);
  expect(sent.calls).toBe(1);
});
