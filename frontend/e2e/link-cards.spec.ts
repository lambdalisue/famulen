import { expect, type Page, test } from "@playwright/test";
import { call, visit } from "./support";

const PREVIEWS = /ConsoleService\/ListLinkPreviews/;
const PIXEL = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==", "base64");

const CARD = {
  url: "https://site.example/page",
  site: "サイトの名前",
  title: "<b>頁の題</b>",
  description: `頁の説明 ${"長い説明".repeat(60)}`,
  image: "picture",
};

const messages = (page: Page) => page.getByTestId("message");
const atEnd = (page: Page) => page.locator("#messages").evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight);

const say = (request: Parameters<typeof call>[0], text: string) => call(request, "ConsoleService", "PostMessage", { text });

/** Pages that tell of themselves, and the picture they name. */
async function pagesTell(page: Page, previews: object[] = [CARD]) {
  await page.route(PREVIEWS, (route) => route.fulfill({ json: { previews } }));
  await page.route(/\/link-images\//, (route) => route.fulfill({ contentType: "image/png", body: PIXEL }));
}

test("a link in either one's message shows what its page tells, as text, with its picture from the console", async ({ page, request }) => {
  await pagesTell(page);
  const outside: string[] = [];
  page.on("request", (r) => {
    if (new URL(r.url()).hostname.endsWith(".example")) outside.push(r.url());
  });
  await say(request, "e2e-カード https://site.example/page を見て");
  await call(request, "ConsoleService", "Send", { text: "e2e-カード 主人 https://site.example/page", workgroup: "all" });
  await say(request, "e2e-カード リンクのない発言");
  await visit(page, "/", /^ホーム/);

  for (const text of ["e2e-カード https", "e2e-カード 主人"]) {
    const said = messages(page).filter({ hasText: text });
    const card = said.locator(".link-card");
    await expect(card).toHaveCount(1);
    await expect(card).toHaveAttribute("href", CARD.url);
    await expect(card.locator(".link-card-title")).toHaveText("<b>頁の題</b>");
    await expect(card).toContainText("サイトの名前");
    await expect(card).toContainText("頁の説明");
    await expect(card.locator("img")).toHaveAttribute("src", "/link-images/picture");
    await expect(said.locator(".markdown-body a")).toHaveText(CARD.url);
  }
  await expect(messages(page).filter({ hasText: "リンクのない" }).locator(".link-card")).toHaveCount(0);
  await expect.poll(() => atEnd(page)).toBeLessThan(2);
  expect(outside).toEqual([]);
});

test("a page that tells nothing, or could not be read, leaves the link as it is", async ({ page, request }) => {
  // The backend itself refuses this machine.
  await say(request, "e2e-名乗らない http://127.0.0.1:17790/health");
  await visit(page, "/", /^ホーム/);
  const said = messages(page).filter({ hasText: "e2e-名乗らない" });
  await expect(said.locator(".markdown-body a")).toBeVisible();
  await expect(said.locator(".link-card")).toHaveCount(0);

  await page.route(PREVIEWS, (route) => route.fulfill({ status: 500, body: "" }));
  await say(request, "e2e-読めない https://site.example/page");
  const failed = messages(page).filter({ hasText: "e2e-読めない" });
  await expect(failed.locator(".markdown-body a")).toBeVisible();
  await expect(failed.locator(".link-card")).toHaveCount(0);
});

test("a card that comes above what is read leaves it where it is", async ({ page, request }) => {
  const release = { go: () => {} };
  const gate = new Promise<void>((done) => {
    release.go = done;
  });
  let asked = 0;
  await page.route(PREVIEWS, async (route) => {
    asked += 1;
    await gate;
    await route.fulfill({ json: { previews: [{ ...CARD, image: "" }] } });
  });
  for (let i = 0; i < 30; i++) await say(request, `e2e-動かない ${i} です。${"長い話。".repeat(20)}`);
  await say(request, "e2e-動かない 上 https://site.example/page");
  for (let i = 30; i < 50; i++) await say(request, `e2e-動かない ${i} です。${"長い話。".repeat(5)}`);
  await visit(page, "/", /^ホーム/);
  await expect(messages(page).last().locator(".markdown-body")).toBeVisible();
  await expect.poll(() => atEnd(page)).toBeLessThan(2);
  // Not by the wheel, whose motion goes on for a while.
  const link = messages(page).filter({ hasText: "e2e-動かない 上" });
  // The message with the link is just above what is seen.
  await link.evaluate((el) => {
    const list = document.querySelector("#messages");
    if (list) list.scrollTop += el.getBoundingClientRect().bottom - list.getBoundingClientRect().top + 20;
  });
  await expect.poll(() => asked).toBeGreaterThan(0);
  await page.waitForTimeout(200);
  const reading = messages(page).filter({ hasText: "e2e-動かない 31 " });
  const place = () => reading.evaluate((el) => el.getBoundingClientRect().top);
  const was = await place();
  release.go();
  await expect(link.locator(".link-card")).toHaveCount(1);
  await expect.poll(async () => Math.abs((await place()) - was)).toBeLessThan(3);
});

test.describe("on a narrow screen", () => {
  test.use({ viewport: { width: 360, height: 740 }, hasTouch: true, isMobile: true });

  test("cards stay within the chat, and what is being written keeps its focus", async ({ page, request }) => {
    await pagesTell(page, [CARD, { ...CARD, title: "x".repeat(200), image: "" }]);
    await say(request, "e2e-狭いカード 前");
    await visit(page, "/", /^ホーム/);
    await page.getByRole("button", { name: "会話を引き上げる" }).click();
    const box = page.locator("#say");
    await box.fill("書きかけ");
    await say(request, "e2e-狭いカード https://site.example/page");
    const said = messages(page).filter({ hasText: "e2e-狭いカード https" });
    await expect(said.locator(".link-card")).toHaveCount(2);
    await expect(box).toBeFocused();
    await expect(box).toHaveValue("書きかけ");
    const wide = await page.locator("#messages").evaluate((el) => el.scrollWidth - el.clientWidth);
    expect(wide).toBeLessThanOrEqual(0);
    await expect.poll(() => atEnd(page)).toBeLessThan(2);
  });
});
