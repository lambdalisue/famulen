import { type APIRequestContext, expect, type Page, test } from "@playwright/test";
import { call, detail, expectView, letOn, record, row, sayImage, visit, whollyOnScreen } from "./support";

function tabbar(page: Page) {
  return page.locator("#m-tabbar");
}

test("the bottom tab bar holds home, calendar, tasks, memos and ほか", async ({ page }) => {
  await visit(page, "/", /^ホーム/);
  await expect(page.locator(".topbar")).toBeHidden();
  await expect(tabbar(page).getByRole("link")).toHaveText(["ホーム", "カレンダー", "タスク", "メモ", "ほか"]);

  await tabbar(page).getByRole("link", { name: "タスク" }).click();
  await expect(page).toHaveURL(/\/tasks$/);
  await expect(page.getByRole("radiogroup", { name: "タスクの種類" })).toBeVisible();
  await expect(tabbar(page).getByRole("link", { name: "タスク" })).toHaveClass(/\bon\b/);
});

test("the calendar opens on a single day in the narrow layout", async ({ page }) => {
  await visit(page, "/", /^ホーム/);
  await tabbar(page).getByRole("link", { name: "カレンダー" }).click();
  await expect(page).toHaveURL(/\/calendar\?unit=day/);
  await expect(page.getByRole("navigation", { name: "表示の単位" }).getByRole("link", { name: "日" })).toHaveAttribute(
    "aria-current",
    "true",
  );
});

test("an event's own link keeps it open when the narrow layout turns to a single day", async ({ page, request }) => {
  const e = await record(request, "events", "リンクで開く予定", { fields: { Start: "2026-12-09 13:00", End: "2026-12-09 14:00" } });
  await visit(page, `/calendar?record=${e.id}`, /^カレンダー/);
  await expect(page).toHaveURL(/unit=day/);
  await expect(page).toHaveURL(new RegExp(`record=${e.id}`));
  await expect(detail(page).getByLabel("題名")).toHaveValue("リンクで開く予定");
});

test("ほか lists the other tabs and leads into the settings and memory menus", async ({ page }) => {
  await visit(page, "/", /^ホーム/);
  await tabbar(page).getByRole("link", { name: "ほか" }).click();
  await expect(page).toHaveURL(/\/more$/);
  await expectView(page, "ほか");
  for (const name of ["相手", "物", "契約", "記憶", "設定"]) {
    await expect(page.locator("#pane").getByRole("link", { name: new RegExp(`^${name}`) })).toBeVisible();
  }
  await expect(tabbar(page).getByRole("link", { name: "ほか" })).toHaveClass(/\bon\b/);

  await page.locator("#pane").getByRole("link", { name: /^設定/ }).click();
  await expect(page).toHaveURL(/\/settings\/menu$/);
  await expect(page.getByRole("link", { name: "登録したモデル" })).toBeVisible();
  await page.getByRole("link", { name: "登録したモデル" }).click();
  await expect(page).toHaveURL(/\/settings\/models$/);
  await expect(page.locator(".m-title")).toHaveText("登録したモデル");

  await page.goto("/more");
  await page.locator("#pane").getByRole("link", { name: /^記憶/ }).click();
  await expect(page).toHaveURL(/\/memory\/menu$/);
  await page.getByRole("link", { name: /^心得/ }).click();
  await expect(page).toHaveURL(/\/memory\/core$/);
});

test("the chat peeks above the tab bar, opens over the whole screen, and closes", async ({ page }) => {
  await visit(page, "/", /^ホーム/);
  const peek = page.getByRole("button", { name: "会話を引き上げる" });
  const close = page.getByRole("button", { name: "閉じる" });
  await expect(peek).toBeVisible();
  await expect(close).toBeHidden();

  await peek.click();
  await expect(close).toBeVisible();
  await expect(peek).toBeHidden();
  const box = await page.locator(".chat").boundingBox();
  expect(box?.y).toBe(0);

  await close.click();
  await expect(peek).toBeVisible();
  await expect(close).toBeHidden();
});

/** Pull with a finger from the middle of an element. */
async function pull(page: Page, selector: string, by: number) {
  const box = await page.locator(selector).boundingBox();
  if (!box) throw new Error(`no ${selector}`);
  const [x, y] = [box.x + box.width / 2, box.y + box.height / 2];
  const cdp = await page.context().newCDPSession(page);
  const touch = (type: "touchStart" | "touchMove" | "touchEnd", at?: number) =>
    cdp.send("Input.dispatchTouchEvent", { type, touchPoints: at === undefined ? [] : [{ x, y: at }] });
  await touch("touchStart", y);
  for (let i = 1; i <= 10; i++) await touch("touchMove", y + (by * i) / 10);
  await touch("touchEnd");
}

test("the chat is pulled up by its peek and down by its bar", async ({ page }) => {
  await visit(page, "/", /^ホーム/);
  await pull(page, ".m-peek", -400);
  await expect(page.locator(".m-sheetbar")).toBeInViewport();
  await expect.poll(async () => (await page.locator(".chat").boundingBox())?.y).toBe(0);

  await pull(page, ".m-sheetbar", 400);
  await expect(page.getByRole("button", { name: "会話を引き上げる" })).toBeVisible();
  await expect(page.locator("#messages")).toBeHidden();
});

test("starting to write opens the chat, on the newest message", async ({ page, request }) => {
  for (let i = 0; i < 12; i++) await call(request, "ConsoleService", "PostMessage", { text: `スマホの発言 ${i}\n\n${"長い話。".repeat(30)}` });
  await visit(page, "/", /^ホーム/);
  await expect(page.locator(".m-peek")).toContainText("スマホの発言 11");
  await page.getByPlaceholder(/に頼む$/).focus();
  await expect(page.locator(".m-sheetbar")).toBeVisible();
  const atEnd = () => page.locator("#messages").evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight);
  await expect.poll(atEnd).toBeLessThan(2);

  await call(request, "ConsoleService", "PostMessage", { text: `あとから来た発言\n\n${"続き。".repeat(40)}` });
  await expect(page.getByTestId("message").filter({ hasText: "あとから来た発言" })).toBeVisible();
  await expect.poll(atEnd).toBeLessThan(2);
});

test("a message is sent with one tap while typing, and the keyboard stays until dismissed", async ({ page }) => {
  await visit(page, "/", /^ホーム/);
  await page.getByRole("button", { name: "会話を引き上げる" }).click();
  await page.getByPlaceholder(/に頼む$/).fill("明日の天気を見て");
  await expect(page.locator("#m-tabbar")).toBeHidden();

  await page.getByRole("button", { name: "送る", exact: true }).tap();
  await expect(page.getByTestId("message").filter({ hasText: "明日の天気を見て" })).toBeVisible();
  await expect(page.getByPlaceholder(/に頼む$/)).toHaveValue("");
  await expect(page.getByPlaceholder(/に頼む$/)).toBeFocused();

  await page.locator("#messages").tap();
  await expect(page.getByPlaceholder(/に頼む$/)).not.toBeFocused();
  await expect(page.locator("#m-tabbar")).toBeVisible();
});

test("closing the chat while typing puts the keyboard away with it", async ({ page }) => {
  await visit(page, "/", /^ホーム/);
  const say = page.getByPlaceholder(/に頼む$/);
  const peek = page.getByRole("button", { name: "会話を引き上げる" });
  await peek.click();
  await say.fill("書きかけ");
  await expect(page.locator("#composer").getByRole("button")).toHaveText(["続き", "新しい話", "", "送る"]);
  // On iOS a tapped button does not take the focus.
  await page.getByRole("button", { name: "閉じる" }).evaluate((el: HTMLElement) => el.click());
  await expect(say).not.toBeFocused();
  await expect(peek).toBeVisible();
  await expect(page.locator("#m-tabbar")).toBeVisible();
  await expect(say).toHaveValue("書きかけ");

  await say.focus();
  await pull(page, ".m-sheetbar", 300);
  await expect(peek).toBeVisible();
  await expect(say).not.toBeFocused();
});

/** Lay an on-screen keyboard over all but this part of the screen, the way iOS does. */
async function keyboard(page: Page, seen: { top: number; height: number }) {
  await page.evaluate(({ top, height }) => {
    if (!window.visualViewport) throw new Error("no visual viewport");
    Object.defineProperty(window.visualViewport, "height", { value: height, configurable: true });
    Object.defineProperty(window.visualViewport, "offsetTop", { value: top, configurable: true });
    window.visualViewport.dispatchEvent(new Event("resize"));
  }, seen);
}

for (const size of [
  { name: "upright", viewport: { width: 390, height: 844 }, seen: { top: 0, height: 380 } },
  { name: "upright, panned by the browser", viewport: { width: 390, height: 844 }, seen: { top: 180, height: 380 } },
  { name: "under a tall keyboard", viewport: { width: 390, height: 664 }, seen: { top: 0, height: 250 } },
]) {
  test(`many lines being written stay over the keyboard, ${size.name}`, async ({ page, request }) => {
    await page.setViewportSize(size.viewport);
    await visit(page, "/", /^ホーム/);
    await call(request, "ConsoleService", "Send", { text: "e2e-long-bash", workgroup: "all" });
    await expect(page.locator(".m-peek")).toContainText("確認 1");
    const say = page.getByPlaceholder(/に頼む$/);
    await say.focus();
    await keyboard(page, size.seen);
    const lines = Array.from({ length: 5 }, (_, i) => `${i + 1} 行目`);
    await say.fill(lines.join("\n"));
    const bottom = size.seen.top + size.seen.height;
    const within = async (selector: string) => {
      const box = await page.locator(selector).boundingBox();
      return !!box && box.y >= size.seen.top && box.y + box.height <= bottom;
    };
    await expect.poll(() => within("#say")).toBe(true);
    expect(await within("#composer")).toBe(true);
    expect(await within("#composer .send")).toBe(true);
    if (size.seen.height >= 380) expect(await say.evaluate((el) => el.scrollHeight - el.clientHeight)).toBeLessThanOrEqual(0);

    await say.fill([...lines, ...lines, ...lines].join("\n"));
    expect(await within("#composer")).toBe(true);
    expect(await say.evaluate((el) => el.scrollHeight > el.clientHeight && el.clientHeight >= 40)).toBe(true);

    await say.fill("");
    await page.locator("#messages").evaluate(() => (document.activeElement as HTMLElement).blur());
    await keyboard(page, { top: 0, height: size.viewport.height });
    const card = page.getByTestId("approval");
    await card.getByRole("button", { name: "却下" }).click();
    await expect(card).toHaveCount(0);
  });
}

test("changing page closes the chat sheet", async ({ page }) => {
  await visit(page, "/", /^ホーム/);
  await page.getByRole("button", { name: "会話を引き上げる" }).click();
  await expect(page.locator("#messages")).toBeVisible();
  await page.goto("/tasks");
  await expectView(page, "タスク");
  await page.locator("#m-tabbar").getByRole("link", { name: "メモ" }).click();
  await expect(page.locator("#messages")).toBeHidden();
});

test("the workgroup chip opens a sheet, and choosing there narrows the console", async ({ page }) => {
  await visit(page, "/", /^ホーム/);
  await page.getByRole("button", { name: /^ワークグループを選ぶ/ }).click();
  const sheet = page.getByRole("dialog", { name: "ワークグループ" });
  await expect(sheet).toBeVisible();

  await sheet.getByRole("button", { name: "仕事" }).click();
  await expect(sheet).toBeHidden();
  await expect(page.getByRole("button", { name: "ワークグループを選ぶ（いまは仕事）" })).toBeVisible();
  await page.reload();
  await expect(page.getByRole("button", { name: "ワークグループを選ぶ（いまは仕事）" })).toBeVisible();

  await page.getByRole("button", { name: /^ワークグループを選ぶ/ }).click();
  await expect(sheet.getByRole("button", { name: "仕事" })).toHaveAttribute("aria-pressed", "true");
  await sheet.getByRole("button", { name: "すべて" }).click();
  await expect(page.getByRole("button", { name: "ワークグループを選ぶ（いまはすべて）" })).toBeVisible();
});

test("the memo list opens one memo on its own, and the back link returns to the list", async ({ page, request }) => {
  const memo = await record(request, "memos", "スマホで読むメモ", { notes: "本文です" });
  await visit(page, "/memos", "メモ");
  await row(page, "スマホで読むメモ").getByRole("link").click();
  await expect(page).toHaveURL(new RegExp(`/memos\\?id=${memo.id}$`));
  await expect(detail(page).getByLabel("題名")).toHaveValue("スマホで読むメモ");
  await expect(row(page, "スマホで読むメモ")).toBeHidden();

  await page.getByRole("link", { name: "メモへ戻る" }).click();
  await expect(page).toHaveURL(/\/memos$/);
  await expect(row(page, "スマホで読むメモ")).toBeVisible();

  // What was opened stays beside the composer, and the button that adds stands above it.
  const add = page.getByRole("button", { name: "メモを書く" });
  const [button, chat] = [await add.boundingBox(), await page.locator(".chat").boundingBox()];
  expect((button?.y ?? 0) + (button?.height ?? 0)).toBeLessThanOrEqual(chat?.y ?? 0);
  await add.tap();
  await expect(page.getByRole("dialog")).toBeVisible();
});

test("the task page's floating button opens the dialog that adds a task", async ({ page }) => {
  await visit(page, "/tasks", "タスク");
  await page.getByRole("button", { name: "タスクを足す" }).click();
  await page.getByRole("dialog").getByLabel("何をするか").fill("スマホで足す");
  await page.getByRole("dialog").getByRole("button", { name: "足す", exact: true }).click();
  await expect(detail(page).getByLabel("題名")).toHaveValue("スマホで足す");
});

test("the vault opens from the settings menu and takes a secret on the page, not through the chat", async ({ page }) => {
  await visit(page, "/settings/menu", "設定");
  await page.getByRole("link", { name: "金庫" }).click();
  await expect(page).toHaveURL(/\/settings\/vault$/);
  await expect(page.locator(".m-title")).toHaveText("金庫");
  const value = page.getByLabel("中身", { exact: true });
  await value.scrollIntoViewIfNeeded();
  await expect(value).toBeInViewport();
  await expect(value).toHaveAttribute("type", "password");
  await expect(page.getByRole("button", { name: "入れる" })).toBeInViewport();
});

test("the console is an app over all its pages once put on the home screen", async ({ page, request }) => {
  for (const path of ["/", "/tasks", "/settings/services"]) {
    await page.goto(path);
    await expect(page.locator('link[rel="manifest"]')).toHaveAttribute("href", "/icons/male/manifest.webmanifest");
  }
  for (const portrait of ["male", "female"]) {
    const answer = await request.get(`/icons/${portrait}/manifest.webmanifest`);
    expect(answer.ok()).toBe(true);
    expect(answer.headers()["content-type"]).toContain("application/manifest+json");
    const manifest = (await answer.json()) as { icons: { src: string; type: string; purpose?: string }[] };
    // The same app whichever portrait it was put on the home screen with.
    expect(manifest).toMatchObject({ id: "/", scope: "/", start_url: "/", display: "standalone" });
    expect(manifest.icons.some((i) => i.purpose === "maskable")).toBe(true);
    for (const icon of manifest.icons) {
      expect(icon.src).toContain(`/icons/${portrait}/`);
      expect((await request.get(icon.src)).headers()["content-type"]).toBe(icon.type);
    }
  }
});

test("a long call that came while the chat was folded is read to its end once it opens", async ({ page, request }) => {
  await visit(page, "/", /^ホーム/);
  await call(request, "ConsoleService", "Send", { text: "e2e-long-bash", workgroup: "all" });
  await expect(page.locator(".m-peek")).toContainText("確認 1");
  await page.getByRole("button", { name: "会話を引き上げる" }).click();
  const card = page.getByTestId("approval");
  await card.getByRole("button", { name: "すべて読む" }).click();
  await expect(card.locator(".asked-text")).toContainText("rm e2e-last");
  await expect(card.getByRole("button", { name: "畳む" })).toBeVisible();
  await card.getByRole("button", { name: "却下" }).click();
  await expect(card).toHaveCount(0);
});

test("the categories of things are within reach of the thumb, and scroll beside the button that adds", async ({ page, request }) => {
  // Enough of them to have a tab whatever the tests before left.
  for (let i = 1; i <= 12; i++) await record(request, "holdings", `楽器の品 ${i}`, { fields: { Category: "楽器" } });
  for (const c of ["書籍", "家電", "工具", "衣類", "家具"]) await record(request, "holdings", `${c}の品`, { fields: { Category: c } });
  await visit(page, "/things", "物");
  const tabs = page.getByRole("navigation", { name: "分類" });
  expect((await tabs.boundingBox())?.y ?? 0).toBeGreaterThan(400);
  await tabs.getByRole("link", { name: "楽器" }).tap();
  await expect(page.locator("[data-record]")).toHaveText(Array<RegExp>(12).fill(/楽器の品/));
  await row(page, "楽器の品").first().getByRole("link").tap();
  await page.getByRole("link", { name: "物へ戻る" }).click();
  await expect(tabs.getByRole("link", { name: "楽器" })).toBeInViewport();
  const add = page.getByRole("button", { name: "物を足す" });
  await add.tap();
  await expect(page.getByRole("dialog")).toBeVisible();
});

test("a lookout keeps its name readable beside its buttons, and goes along with what is said once told to", async ({ page, request }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const r = await call<{ connection: { id: string } }>(request, "ConnectionService", "AddConnection", {
    target: "e2e-hook",
    how: "definition",
    env: "token=e2e",
  });
  const added = await call<{ id: string }>(request, "LookoutService", "AddLookout", { name: "会社の受信箱を見る", connection: r.connection.id });
  try {
    await visit(page, "/settings/lookout", "設定 · 見張り");
    const source = page.getByRole("listitem").filter({ hasText: "会社の受信箱を見る" });
    expect((await source.locator(".lookout-source").boundingBox())?.width ?? 0).toBeGreaterThan(180);
    await source.getByRole("button", { name: "会話に添える" }).tap();
    await expect(page.locator("#selection")).toContainText("見張り 会社の受信箱を見る");
  } finally {
    await call(request, "LookoutService", "RemoveLookout", { id: added.id });
    await call(request, "ConnectionService", "DeleteConnection", { id: r.connection.id });
  }
});

/** Open the chat on enough messages to read back through, with one of them in the middle in sight. */
async function readBack(page: Page, request: APIRequestContext, name: string) {
  await page.setViewportSize({ width: 390, height: 844 });
  for (let i = 0; i < 12; i++) {
    await call(request, "ConsoleService", "PostMessage", { text: `${name} ${i}\n\n${"長い話。".repeat(30)}` });
  }
  await visit(page, "/", /^ホーム/);
  await page.getByRole("button", { name: "会話を引き上げる" }).click();
  const list = page.locator("#messages");
  const read = page.getByTestId("message").filter({ hasText: `${name} 5` });
  await expect(page.getByTestId("message").filter({ hasText: `${name} 11` }).locator(".markdown-body")).toBeVisible();
  await expect.poll(() => list.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(2);
  await read.evaluate((el) => el.scrollIntoView({ block: "start" }));
  await settled(page);
  return { list, at: async () => Math.round((await read.boundingBox())?.y ?? Number.NaN) };
}

/** Wait until what came has been laid out and drawn. */
function settled(page: Page) {
  return page.evaluate(() => new Promise<void>((done) => setTimeout(() => requestAnimationFrame(() => requestAnimationFrame(() => done())), 300)));
}

/** Change a record, and wait until the conversation has been read again for it. */
async function changeRecord(page: Page, request: APIRequestContext, title: string) {
  await settled(page);
  const read = page.waitForResponse((r) => r.url().endsWith("/ListMessages"));
  await record(request, "memos", title);
  await read;
}

async function reject(page: Page) {
  const card = page.getByTestId("approval");
  await card.getByRole("button", { name: "却下" }).click();
  await expect(card).toHaveCount(0);
}

test("the conversation stays where it is read, whatever comes, and follows again from its end", async ({ page, request }) => {
  const { list, at } = await readBack(page, request, "読み返す");
  const before = await at();
  const still = async (what: string) => {
    await settled(page);
    expect(Math.abs((await at()) - before), what).toBeLessThanOrEqual(1);
  };

  await call(request, "ConsoleService", "PostMessage", { text: "読む間に来た発言" });
  await expect(page.getByTestId("message").filter({ hasText: "読む間に来た発言" }).last()).toBeAttached();
  await still("a message");
  await changeRecord(page, request, "読む間に足したメモ");
  await still("a record");
  await page.getByTestId("message").filter({ hasText: "読み返す 2" }).evaluate((el: HTMLElement) => {
    el.style.minHeight = `${el.offsetHeight + 200}px`;
  });
  await still("a message above that grew, as with an image");
  await call(request, "ConsoleService", "Send", { text: "e2e-bash", workgroup: "all" });
  await expect(page.getByTestId("approval")).toBeVisible();
  await still("a request");
  await reject(page);
  await still("a request that went");

  const long = page.getByTestId("message").filter({ hasText: "読み返す 5" });
  const end = long.locator(".markdown-body > *").last();
  await end.evaluate((el) => el.scrollIntoView({ block: "start" }));
  await list.evaluate((el) => el.scrollBy(0, 20));
  await settled(page);
  const inside = (await end.boundingBox())?.y ?? Number.NaN;
  await long.locator(".meta").evaluate((el: HTMLElement) => {
    el.style.paddingBottom = "150px";
  });
  await settled(page);
  expect(Math.abs(((await end.boundingBox())?.y ?? Number.NaN) - inside), "the message itself grew above").toBeLessThanOrEqual(1);

  await list.evaluate((el) => el.scrollTo(0, el.scrollHeight));
  await settled(page);
  await call(request, "ConsoleService", "PostMessage", { text: `末尾で来た発言\n\n${"続き。".repeat(60)}` });
  await expect(page.getByTestId("message").filter({ hasText: "末尾で来た発言" }).last().locator(".markdown-body")).toBeVisible();
  await expect.poll(() => list.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(2);
});

test("an image opens while writing, and closing it goes back to what is written", async ({ page, request }) => {
  await visit(page, "/", /^ホーム/);
  await sayImage(page, request, "縦長の画像", "縦長.png", { width: 800, height: 4000 });
  await call(request, "ConsoleService", "PostMessage", { text: `縦長の画像のあと\n\n${"続き。".repeat(40)}` });
  const say = page.getByPlaceholder(/に頼む$/);
  await say.focus();
  await say.fill("打ちかけの文字");
  await say.evaluate((el: HTMLTextAreaElement) => el.setSelectionRange(3, 3));
  const opener = page.getByRole("button", { name: "縦長.png を拡大する" });
  await expect.poll(() => opener.locator("img").evaluate((el: HTMLImageElement) => el.naturalWidth)).toBe(800);
  const whole = page.viewportSize()?.height ?? 0;
  const up = () => keyboard(page, { top: 0, height: 500 });
  await up();
  const list = page.locator("#messages");
  await settled(page);
  // Read back less than the keyboard takes, where the list would reach its end without it.
  await opener.evaluate((el) => el.scrollIntoView({ block: "center" }));
  await settled(page);
  const below = await list.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight);
  expect(below).toBeGreaterThan(40);
  expect(below).toBeLessThan(whole - 500);
  const top = await list.evaluate((el) => el.scrollTop);
  const box = page.getByRole("dialog", { name: "縦長.png" });
  const image = box.getByRole("img", { name: "縦長.png" });
  for (const close of [() => page.touchscreen.tap(10, 400), () => box.getByRole("button", { name: "閉じる" }).tap()]) {
    await opener.tap();
    await expect(box).toBeVisible();
    await expect.poll(() => whollyOnScreen(page, image)).toBe(true);
    await keyboard(page, { top: 0, height: whole });
    await settled(page);
    const size = await image.boundingBox();
    expect((size?.height ?? 0) / (size?.width ?? 1)).toBeCloseTo(5, 1);
    await close();
    await expect(box).toHaveCount(0);
    await expect(say).toBeFocused();
    await expect(say).toHaveValue("打ちかけの文字");
    await expect.poll(() => say.evaluate((el: HTMLTextAreaElement) => [el.selectionStart, el.selectionEnd])).toEqual([3, 3]);
    await settled(page);
    await up();
    await settled(page);
    expect(Math.abs((await list.evaluate((el) => el.scrollTop)) - top)).toBeLessThanOrEqual(1);
  }
});

test("going to another page closes the image, and leaves the conversation folded", async ({ page, request }) => {
  await visit(page, "/", /^ホーム/);
  await tabbar(page).getByRole("link", { name: "タスク" }).click();
  await expectView(page, /^タスク/);
  await sayImage(page, request, "移る前の画像", "移る前.png", { width: 400, height: 300 });
  await page.getByPlaceholder(/に頼む$/).focus();
  await page.getByRole("button", { name: "移る前.png を拡大する" }).tap();
  const box = page.getByRole("dialog", { name: "移る前.png" });
  await expect(box).toBeVisible();
  await page.goBack();
  await expectView(page, /^ホーム/);
  await expect(box).toHaveCount(0);
  await expect(page.locator("body")).not.toHaveClass(/chat-open|typing/);
});

test("what is being written keeps the keyboard, its text and its caret, whatever comes", async ({ page, request }) => {
  const { at } = await readBack(page, request, "書きながら");
  const say = page.getByPlaceholder(/に頼む$/);
  await say.focus();
  await keyboard(page, { top: 0, height: 500 });
  await say.fill("打ちかけの文字");
  await say.evaluate((el: HTMLTextAreaElement) => el.setSelectionRange(3, 3));
  await settled(page);
  const before = await at();
  await say.evaluate((el) => {
    el.addEventListener("blur", () => el.setAttribute("data-lost", ""));
  });
  const kept = async (what: string) => {
    await settled(page);
    await expect(say, what).toBeFocused();
    expect(await say.getAttribute("data-lost"), what).toBeNull();
    await expect(say).toHaveValue("打ちかけの文字");
    expect(await say.evaluate((el: HTMLTextAreaElement) => [el.selectionStart, el.selectionEnd])).toEqual([3, 3]);
    await expect(page.locator("#m-tabbar")).toBeHidden();
    expect(Math.abs((await at()) - before), what).toBeLessThanOrEqual(1);
  };

  await call(request, "ConsoleService", "PostMessage", { text: "書く間に来た発言" });
  await expect(page.getByTestId("message").filter({ hasText: "書く間に来た発言" }).last()).toBeAttached();
  await kept("a message");
  await changeRecord(page, request, "書く間に足したメモ");
  await kept("a record");
  await call(request, "ConsoleService", "Send", { text: "e2e-bash", workgroup: "all" });
  await expect(page.getByTestId("approval")).toBeVisible();
  await kept("a request");
  await page.route(/\/(ListMessages|GetInbox|GetPersona)$/, async (r) => {
    await new Promise((done) => setTimeout(done, 1500));
    await r.continue();
  });
  await record(request, "memos", "ゆっくり届くメモ");
  await call(request, "ConsoleService", "PostMessage", { text: "読み直す間に来た発言" });
  await kept("a message while the rest is read again");
  await expect(page.getByTestId("message").filter({ hasText: "読み直す間に来た発言" }).last()).toBeAttached();
  await page.unroute(/\/(ListMessages|GetInbox|GetPersona)$/);
  await page.route(/\/(ListMessages|GetInbox|GetPersona)$/, (r) => r.abort());
  await record(request, "memos", "届かない間に足したメモ");
  await kept("a reload that failed");
  await page.unroute(/\/(ListMessages|GetInbox|GetPersona)$/);
  await changeRecord(page, request, "また届いたメモ");
  await kept("a reload after that");
  await say.evaluate((el) => el.blur());
  await keyboard(page, { top: 0, height: 844 });
  await reject(page);
});

test("what the owner opened in a request stays open while records change", async ({ page, request }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await visit(page, "/", /^ホーム/);
  await call(request, "ConsoleService", "Send", { text: "e2e-long-bash", workgroup: "all" });
  await expect(page.locator(".m-peek")).toContainText("確認 1");
  await page.getByRole("button", { name: "会話を引き上げる" }).click();
  const card = page.getByTestId("approval");
  await card.getByRole("button", { name: "すべて読む" }).tap();
  const fold = card.getByRole("button", { name: "畳む" });
  await fold.focus();
  await changeRecord(page, request, "確認を読む間に足したメモ");
  await settled(page);
  await expect(fold).toBeFocused();
  await reject(page);
});

test("what is read far back stays while the conversation, longer than is read at once, is read again", async ({ page, request }) => {
  const post = (text: string) => call(request, "ConsoleService", "PostMessage", { text });
  for (let i = 0; i < 125; i++) await post(`遠い発言 ${i}`);
  await visit(page, "/", /^ホーム/);
  await page.getByRole("button", { name: "会話を引き上げる" }).click();
  // What is before it is read as it is come near, so it does not stay the first.
  const text = (await page.getByTestId("message").first().locator(".markdown-body").innerText()).trim();
  const oldest = page.getByTestId("message").filter({ hasText: text });
  await page.locator("#messages").evaluate((el) => el.scrollTo(0, 0));
  await settled(page);
  const at = async () => Math.round((await oldest.boundingBox())?.y ?? Number.NaN);
  const before = await at();
  await post("遠くを読む間に来た発言");
  await expect(page.getByTestId("message").filter({ hasText: "遠くを読む間に来た発言" }).last()).toBeAttached();
  await changeRecord(page, request, "遠くを読む間に足したメモ");
  await settled(page);
  await expect(oldest.locator(".markdown-body")).toHaveText(text);
  expect(Math.abs((await at()) - before)).toBeLessThanOrEqual(1);
});

test("what the secretary is doing fits the narrow layout, open and folded", async ({ page, request }) => {
  await visit(page, "/", /^ホーム/);
  letOn("search");
  await page.getByPlaceholder(/に頼む$/).fill("e2e-work を狭い画面で");
  await page.getByRole("button", { name: "送る", exact: true }).click();
  const working = page.getByTestId("working");
  await expect(working).toHaveText("ウェブを調べています…");
  await expect(working).toBeInViewport({ ratio: 1 });
  await page.getByRole("button", { name: "閉じる", exact: true }).click();
  await expect(page.getByRole("button", { name: "会話を引き上げる" })).toContainText("ウェブを調べています…");
  letOn("found", "end");
  await expect.poll(async () => (await call(request, "ConsoleService", "GetStatus")).state, { timeout: 15_000 }).toBe("waiting");
  await expect(page.getByRole("button", { name: "会話を引き上げる" })).not.toContainText("調べています");
});

test("on a phone a message is replied to, and its thread is laid over the conversation with the box still at hand", async ({ page, request }) => {
  const root = await call<{ id: string }>(request, "ConsoleService", "PostMessage", { text: "e2e 携帯 元" });
  await call(request, "ConsoleService", "PostMessage", { text: "e2e 携帯 返信", replyTo: root.id });
  await visit(page, "/", /^ホーム/);
  await page.getByRole("button", { name: "会話を引き上げる" }).click();
  const said = page.locator("#messages").getByTestId("message").filter({ hasText: "e2e 携帯 返信" });
  await said.getByRole("button", { name: "返信", exact: true }).tap();
  await expect(page.getByTestId("replying")).toContainText("e2e 携帯 返信");
  await expect(page.locator("#say")).toBeFocused();
  await page.getByRole("button", { name: "返信をやめる" }).tap();

  await page.getByRole("button", { name: "返信 1 件" }).tap();
  const thread = page.getByRole("region", { name: "スレッド" });
  const over = await page.locator("#messages").boundingBox();
  expect(await thread.boundingBox()).toEqual(over);
  await page.locator("#say").fill("e2e 携帯 スレッドで");
  await page.getByRole("button", { name: "送る", exact: true }).tap();
  await expect(thread.getByTestId("message")).toHaveCount(3);
  await expect.poll(() => whollyOnScreen(page, page.locator("#composer"))).toBe(true);
  // Folded, what is written goes into the conversation again.
  await page.locator(".m-sheetbar").getByRole("button", { name: "閉じる" }).tap();
  await expect(thread).toHaveCount(0);
  await expect(page.getByTestId("replying")).toHaveCount(0);
  await page.getByRole("button", { name: "会話を引き上げる" }).tap();
  await page.getByRole("button", { name: "返信 2 件" }).tap();
  await thread.getByRole("button", { name: "スレッドを閉じる" }).tap();
  await expect(thread).toHaveCount(0);
  await expect.poll(() => said.isVisible()).toBe(true);
});

/** What is pressed, and is given less than 44px by 44px for a finger. Links within a text are as large as the text. */
function tight(page: Page) {
  return page.evaluate(() => {
    const found = new Set<string>();
    const least = 43.5;
    for (const el of document.querySelectorAll<HTMLElement>("button, a, input, select, textarea, summary")) {
      if (!el.checkVisibility({ visibilityProperty: true }) || el.closest("[hidden], .md, p, .sub, .callout, .item-sub:not(:has(> button)), .topbar, .tabs, .sr")) continue;
      // An event is as tall as it is long, and a mark of a chart as it is much.
      if (el.matches(".ev, .allday, .month-date, .week-date, .night, .usage-column, [data-part=hidden-input], [type=hidden], [type=file], .sr")) continue;
      let { width, height } = el.getBoundingClientRect();
      if (width <= 1 || height <= 1) continue;
      const around = el.closest("label")?.getBoundingClientRect();
      if (around) [width, height] = [Math.max(width, around.width), Math.max(height, around.height)];
      const row = el.matches(".item-pick") ? el.closest(".item, .home-ev")?.getBoundingClientRect() : undefined;
      if (row) [width, height] = [row.width, row.height];
      const more = getComputedStyle(el, "::after");
      if (more.content !== "none" && more.position === "absolute") {
        width -= Math.min(0, Number.parseFloat(more.left)) + Math.min(0, Number.parseFloat(more.right));
        height -= Math.min(0, Number.parseFloat(more.top)) + Math.min(0, Number.parseFloat(more.bottom));
      }
      if (width < least || height < least) found.add(`${Math.round(width)}x${Math.round(height)} .${el.className} ${(el.getAttribute("aria-label") ?? el.textContent ?? "").trim().slice(0, 16)}`);
    }
    return [...found];
  });
}

test("what is pressed is large enough for a finger", async ({ page, request }) => {
  test.setTimeout(120_000);
  const task = await record(request, "tasks", "指で押す仕事", { labels: ["指"], fields: { Priority: "high", Due: "2026-12-01" } });
  await record(request, "holdings", "指で押す物", { labels: ["指"], fields: { Category: "機器" } });
  const faults: { [path: string]: string[] } = {};
  for (const path of ["/", "/calendar", "/tasks", `/tasks?id=${task.id}`, "/tasks?tab=delegated", "/memos", "/parties", "/things", "/memory", "/memory/core", "/more", "/settings/menu", "/settings/look", "/settings", "/settings/workgroups", "/settings/services"]) {
    await page.goto(path);
    await expect(page.locator("#composer .view")).not.toBeEmpty();
    const found = await tight(page);
    if (found.length > 0) faults[path] = found;
  }
  await call(request, "ConsoleService", "Send", { text: "指で押す話 e2e-bash", workgroup: "all" });
  await visit(page, "/", /^ホーム/);
  await page.getByRole("button", { name: "会話を引き上げる" }).click();
  const card = page.getByTestId("approval");
  await expect(card).toBeVisible();
  const found = await tight(page);
  if (found.length > 0) faults.conversation = found;
  await card.getByRole("button", { name: "却下" }).click();
  await expect(card).toHaveCount(0);
  expect(faults).toEqual({});
});
