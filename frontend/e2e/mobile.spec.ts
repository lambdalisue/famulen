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

  await page.getByRole("button", { name: "キーボードを閉じる" }).tap();
  await expect(page.locator("#m-tabbar")).toBeVisible();
  await page.getByRole("button", { name: "閉じる" }).tap();
  await expect(page.getByRole("button", { name: "会話を引き上げる" })).toBeVisible();
});

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
