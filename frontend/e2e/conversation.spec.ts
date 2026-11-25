import type { APIRequestContext, Page, Request } from "@playwright/test";
import { expect, test } from "@playwright/test";
import { call, onFrame, visit } from "./support";

const SAID = 130;

async function say(request: APIRequestContext, name: string, count = SAID) {
  for (let i = 0; i < count; i++) await call(request, "ConsoleService", "PostMessage", { text: `${name} ${i} です。${"長い話。".repeat(i % 7)}` });
}

const LIST = /ConsoleService\/ListMessages/;
/** Whether what is older than a message is asked for. */
const older = (r: Request) => r.postDataBuffer()?.toString().includes('"before"') ?? false;

const messages = (page: Page) => page.getByTestId("message");
const atEnd = (page: Page) => page.locator("#messages").evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight);

/** Scroll up by the wheel until what is asked for is there. */
async function wheelUp(page: Page, until: () => Promise<boolean>) {
  await page.locator("#messages").hover();
  await expect
    .poll(
      async () => {
        if (await until()) return true;
        await page.mouse.wheel(0, -3000);
        return until();
      },
      { intervals: [100], timeout: 30_000 },
    )
    .toBe(true);
}

/** Keep what is older from coming until it is let go. */
async function heldBack(page: Page) {
  let release = () => {};
  const gate = new Promise<void>((done) => {
    release = done;
  });
  await page.route(LIST, async (route) => {
    if (older(route.request())) await gate;
    await route.continue();
  });
  return release;
}

/** Where a message stands in the list. */
const place = (page: Page, text: string) =>
  messages(page)
    .filter({ hasText: text })
    .evaluate((el) => el.getBoundingClientRect().top);

/** The text of the message at the top of what is seen. */
const firstSeen = (page: Page) =>
  page.locator("#messages").evaluate((el) => {
    const top = el.getBoundingClientRect().top;
    const seen = [...el.querySelectorAll(".msg")].find((m) => m.getBoundingClientRect().top >= top);
    return /\S+ \d+ です/.exec(seen?.textContent ?? "")?.[0] ?? "";
  });

test("the conversation opens on its newest messages, and goes back to its first as it is scrolled", async ({ page, request }) => {
  await say(request, "e2e-遡る");
  await visit(page, "/", /^ホーム/);
  await expect(messages(page)).toHaveCount(50);
  await expect(messages(page).last()).toContainText(`e2e-遡る ${SAID - 1} です`);
  await expect.poll(() => atEnd(page)).toBeLessThan(2);

  await wheelUp(page, () => page.getByText("会話の最初です").isVisible());
  const numbers = (await messages(page).allTextContents()).flatMap((t) => /e2e-遡る (\d+) です/.exec(t)?.[1] ?? []);
  expect(numbers.map(Number)).toEqual(Array.from({ length: SAID }, (_, i) => i));
});

test("what is read stays where it is, as older and newer messages come", async ({ page, request }) => {
  await say(request, "e2e-留まる");
  const release = await heldBack(page);
  await visit(page, "/", /^ホーム/);
  await expect(messages(page)).toHaveCount(50);
  await expect.poll(() => atEnd(page)).toBeLessThan(2);
  await page.locator("#messages").hover();
  await page.mouse.wheel(0, -100);
  await expect.poll(() => onFrame(page, () => atEnd(page))).toBeGreaterThan(50);
  await page.locator("#messages").evaluate((el) => el.scrollTo(0, 120));
  await expect(page.getByText("前の発言を読み込んでいる")).toBeVisible();
  await expect(messages(page).first().locator(".markdown-body")).toBeVisible();
  const reading = await firstSeen(page);
  const was = await place(page, reading);

  release();
  await expect(messages(page)).toHaveCount(100);
  await expect.poll(() => place(page, reading)).toBeCloseTo(was, -1);

  await call(request, "ConsoleService", "PostMessage", { text: "e2e-留まる あとから" });
  await expect(messages(page).filter({ hasText: "e2e-留まる あとから" })).toHaveCount(1);
  expect(Math.abs((await place(page, reading)) - was)).toBeLessThan(3);

  // A changed record has every list read again.
  const again = page.waitForResponse(LIST);
  await call(request, "RecordService", "SaveRecord", { kind: "memos", title: "e2e-読み直す", fields: [] });
  await again;
  await page.waitForTimeout(300);
  expect(Math.abs((await place(page, reading)) - was)).toBeLessThan(3);
  await expect(messages(page).filter({ hasText: "e2e-留まる あとから" })).toHaveCount(1);
  await expect(messages(page)).toHaveCount(101);
});

test("what could not be read says so, and is read again when asked", async ({ page, request }) => {
  await say(request, "e2e-失敗", 60);
  let fails = true;
  await page.route(LIST, (route) =>
    fails && older(route.request()) ? route.fulfill({ status: 500, body: "" }) : route.continue(),
  );
  await visit(page, "/", /^ホーム/);
  await expect(messages(page)).toHaveCount(50);
  const again = page.getByTestId("said-before").getByRole("button", { name: "読み直す" });
  await wheelUp(page, () => again.isVisible());
  fails = false;
  await again.click();
  await expect.poll(() => messages(page).count()).toBeGreaterThanOrEqual(100);
  await expect(again).toHaveCount(0);
});

test("a workgroup whose messages could not be read says so, not that there are none", async ({ page, request }) => {
  await say(request, "e2e-届かない", 60);
  await visit(page, "/", /^ホーム/);
  await expect(messages(page)).toHaveCount(50);
  let fails = true;
  await page.route(LIST, (route) => (fails ? route.fulfill({ status: 500, body: "" }) : route.continue()));
  await page.getByRole("group", { name: "ワークグループ", exact: true }).getByRole("button", { name: "仕事" }).click();
  const before = page.getByTestId("said-before");
  await expect(before).toContainText("読めなかった");
  await expect(messages(page)).toHaveCount(0);
  fails = false;
  await before.getByRole("button", { name: "読み直す" }).click();
  await expect(messages(page)).toHaveCount(50);
  await expect.poll(() => atEnd(page)).toBeLessThan(2);
  await page.getByRole("group", { name: "ワークグループ", exact: true }).getByRole("button", { name: "すべて" }).click();
});

test("another workgroup opens on its own newest messages, whatever was read before", async ({ page, request }) => {
  const { workgroups } = await call<{ workgroups: { id: string; name: string }[] }>(request, "WorkgroupService", "ListWorkgroups");
  const home = workgroups.find((w) => w.name === "個人")?.id;
  await call(request, "ConsoleService", "Send", { text: "e2e-切替 個人だけ", workgroup: home });
  await say(request, "e2e-切替", 60);
  await visit(page, "/", /^ホーム/);
  await expect(messages(page)).toHaveCount(50);
  await wheelUp(page, async () => (await messages(page).count()) > 50);

  const choose = (name: string) => page.getByRole("group", { name: "ワークグループ", exact: true }).getByRole("button", { name }).click();
  const mine = messages(page).filter({ hasText: "e2e-切替 個人だけ" });
  await choose("仕事");
  await expect(messages(page)).toHaveCount(50);
  await expect.poll(() => atEnd(page)).toBeLessThan(2);
  await wheelUp(page, async () => (await messages(page).count()) > 70);
  await expect(mine).toHaveCount(0);
  await choose("すべて");
  await expect(messages(page)).toHaveCount(50);
  await expect.poll(() => atEnd(page)).toBeLessThan(2);
});

test.describe("on a narrow screen", () => {
  test.use({ viewport: { width: 390, height: 780 }, hasTouch: true, isMobile: true });

  test("older messages come in the opened chat too", async ({ page, request }) => {
    await say(request, "e2e-狭い", 60);
    await visit(page, "/", /^ホーム/);
    await page.waitForTimeout(300);
    await expect(messages(page)).toHaveCount(50);
    await page.getByRole("button", { name: "会話を引き上げる" }).click();
    await expect.poll(() => atEnd(page)).toBeLessThan(2);
    await page.locator("#messages").evaluate((el) => el.scrollTo(0, el.scrollTop - 200));
    await page.waitForTimeout(100);
    const seen = async () => {
      await page.locator("#messages").evaluate((el) => el.scrollTo(0, 100));
      return messages(page).count();
    };
    await expect.poll(seen).toBeGreaterThanOrEqual(100);
  });
});
