import { expect, type Page, test } from "@playwright/test";
import { detail, expectView, record, row, visit } from "./support";

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
    "aria-pressed",
    "true",
  );
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

test("the chat peeks above the tab bar, opens as a half sheet, widens, and closes", async ({ page }) => {
  await visit(page, "/", /^ホーム/);
  const peek = page.getByRole("button", { name: "会話を引き上げる" });
  const close = page.getByRole("button", { name: "閉じる" });
  await expect(peek).toBeVisible();
  await expect(close).toBeHidden();

  await peek.click();
  await expect(page.locator("body")).toHaveClass(/sheet-half/);
  await expect(close).toBeVisible();
  await expect(peek).toBeHidden();

  await page.getByRole("button", { name: "広げる" }).click();
  await expect(page.locator("body")).toHaveClass(/sheet-full/);

  await close.click();
  await expect(page.locator("body")).not.toHaveClass(/sheet-(half|full)/);
  await expect(peek).toBeVisible();
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
  await expect(page.locator("body")).toHaveClass(/sheet-half/);
  await page.goto("/tasks");
  await expectView(page, "タスク");
  await page.locator("#m-tabbar").getByRole("link", { name: "メモ" }).click();
  await expect(page.locator("body")).not.toHaveClass(/sheet-half/);
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
  await row(page, "スマホで読むメモ").click();
  await expect(page).toHaveURL(new RegExp(`/memos\\?id=${memo.id}$`));
  await expect(detail(page).getByLabel("題名")).toHaveValue("スマホで読むメモ");
  await expect(row(page, "スマホで読むメモ")).toBeHidden();

  await page.getByRole("link", { name: "メモへ戻る" }).click();
  await expect(page).toHaveURL(/\/memos$/);
  await expect(row(page, "スマホで読むメモ")).toBeVisible();
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
    await expect(page.locator('link[rel="manifest"]')).toHaveAttribute("href", "/manifest.webmanifest");
  }
  const answer = await request.get("/manifest.webmanifest");
  expect(answer.ok()).toBe(true);
  expect(await answer.json()).toMatchObject({ scope: "/", start_url: "/", display: "standalone" });
  for (const icon of ["/apple-touch-icon.png", "/icon-512.png"]) {
    expect((await request.get(icon)).headers()["content-type"]).toBe("image/png");
  }
});
