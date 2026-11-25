import { expect, type Page, test } from "@playwright/test";
import { call, detail, expectView, record, row, visit } from "./support";

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
  await expect(page.locator("#composer").getByRole("button")).toHaveText(["", "送る"]);
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

test("what the secretary is doing fits the narrow layout, open and folded", async ({ page, request }) => {
  await visit(page, "/", /^ホーム/);
  await page.getByPlaceholder(/に頼む$/).fill("e2e-work を狭い画面で");
  await page.getByRole("button", { name: "送る", exact: true }).click();
  const working = page.getByTestId("working");
  await expect(working).toHaveText("ウェブを調べています…");
  await expect(working).toBeInViewport({ ratio: 1 });
  await page.getByRole("button", { name: "閉じる", exact: true }).click();
  await expect(page.getByRole("button", { name: "会話を引き上げる" })).toContainText("ウェブを調べています…");
  await expect.poll(async () => (await call(request, "ConsoleService", "GetStatus")).state, { timeout: 15_000 }).toBe("waiting");
  await expect(page.getByRole("button", { name: "会話を引き上げる" })).not.toContainText("調べています");
});
