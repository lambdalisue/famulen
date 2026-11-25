import { type APIRequestContext, expect, type Page, test } from "@playwright/test";
import { call, dayFromToday, record, visit } from "./support";

function tile(page: Page, heading: string | RegExp) {
  return page.locator("section.tile").filter({ has: page.getByRole("heading", { name: heading, exact: true }) });
}

const headings = (page: Page) => page.locator("section.tile h2");

test.describe.configure({ mode: "serial" });

type Tile = { kind: string; x?: number; y?: number; width?: number; height?: number };
const savedTiles = (request: APIRequestContext) => call<{ tiles: Tile[] }>(request, "SettingsService", "GetHome").then((h) => h.tiles);

test("home opens with the tiles in the order they are read", async ({ page, request }) => {
  await call(request, "SettingsService", "SetTiles", {
    tiles: [
      { kind: "today", x: 0, y: 0, width: 6, height: 7 },
      { kind: "approvals", x: 6, y: 0, width: 6, height: 3 },
      { kind: "aide", x: 6, y: 3, width: 6, height: 4 },
      { kind: "task", x: 0, y: 7, width: 6, height: 7 },
      { kind: "memos", x: 6, y: 7, width: 6, height: 7 },
    ],
  });
  await visit(page, "/", /^ホーム · /);
  await expect(headings(page)).toHaveText(["今日", "確認待ち", /が進めている$/, "今日と今週のタスク", "最近のメモ"]);
});

test("the arrange controls show only while arranging, and the choice survives a reload", async ({ page }) => {
  await visit(page, "/", /^ホーム · /);
  await expect(page.getByRole("button", { name: "タイルを足す" })).toBeHidden();

  await page.getByRole("button", { name: "配置を変える" }).click();
  await expect(page.getByRole("button", { name: "タイルを足す" })).toBeVisible();
  await page.reload();
  await expect(page.getByRole("button", { name: "タイルを足す" })).toBeVisible();

  await page.getByRole("button", { name: "配置を変える" }).click();
  await expect(page.getByRole("button", { name: "タイルを足す" })).toBeHidden();
});

async function drag(page: Page, from: { x: number; y: number }, by: { x: number; y: number }) {
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(from.x + by.x / 2, from.y + by.y / 2);
  await page.mouse.move(from.x + by.x, from.y + by.y);
  await page.mouse.up();
}

test("a tile dragged by its head stays where it is let go, and what it lies over goes below it", async ({ page, request }) => {
  await call(request, "SettingsService", "SetTiles", {
    tiles: [
      { kind: "today", x: 0, y: 0, width: 4, height: 4 },
      { kind: "approvals", x: 8, y: 0, width: 4, height: 4 },
    ],
  });
  await visit(page, "/", /^ホーム · /);
  await page.getByRole("button", { name: "配置を変える" }).click();
  const grid = await page.locator(".tiles").boundingBox();
  const head = await tile(page, "今日").getByRole("heading").boundingBox();
  if (!grid || !head) throw new Error("no grid");
  const column = (grid.width + 12) / 12;
  await drag(page, { x: head.x + 10, y: head.y + 10 }, { x: 8 * column, y: 0 });
  await expect.poll(() => savedTiles(request)).toEqual([
    { kind: "today", x: 8, width: 4, height: 4 },
    { kind: "approvals", x: 8, y: 4, width: 4, height: 4 },
  ]);

  // Nothing is pulled up: the room it leaves stays.
  await tile(page, "今日").evaluate((el) => el.scrollIntoView({ block: "start" }));
  const moved = await tile(page, "今日").getByRole("heading").boundingBox();
  if (!moved) throw new Error("no tile");
  await drag(page, { x: moved.x + 10, y: moved.y + 10 }, { x: -8 * column, y: 6 * 56 });
  await expect.poll(() => savedTiles(request)).toEqual([
    { kind: "today", y: 6, width: 4, height: 4 },
    { kind: "approvals", x: 8, y: 4, width: 4, height: 4 },
  ]);

  // The one that was moved, though it is read after the other now, is the same element.
  const grip = tile(page, "今日").getByRole("button", { name: "今日を動かす" });
  await grip.press("ArrowRight");
  await expect(grip).toBeFocused();
  await grip.press("ArrowLeft");
  await expect.poll(() => savedTiles(request)).toContainEqual({ kind: "today", y: 6, width: 4, height: 4 });

  // Dragged to where there is room for it on screen.
  await page.locator("#pane").evaluate((el) => el.scrollTo(0, el.scrollHeight));
  const corner = await tile(page, "今日").getByRole("button", { name: "今日の大きさを変える" }).boundingBox();
  if (!corner) throw new Error("no corner");
  await drag(page, { x: corner.x + 12, y: corner.y + 12 }, { x: 2 * column, y: 2 * 56 });
  await expect.poll(() => savedTiles(request)).toContainEqual({ kind: "today", y: 6, width: 6, height: 6 });
  await page.getByRole("button", { name: "配置を変える" }).click();
});

test("the arrow keys move and size a tile, and tiles are added and taken off", async ({ page, request }) => {
  await call(request, "SettingsService", "SetTiles", { tiles: [{ kind: "approvals", x: 0, y: 0, width: 4, height: 4 }] });
  await visit(page, "/", /^ホーム · /);
  await page.getByRole("button", { name: "配置を変える" }).click();

  const grip = tile(page, "確認待ち").getByRole("button", { name: "確認待ちを動かす" });
  await grip.press("ArrowRight");
  await expect.poll(() => savedTiles(request)).toEqual([{ kind: "approvals", x: 1, width: 4, height: 4 }]);
  await tile(page, "確認待ち").getByRole("button", { name: "確認待ちの大きさを変える" }).press("ArrowDown");
  await expect.poll(() => savedTiles(request)).toEqual([{ kind: "approvals", x: 1, width: 4, height: 5 }]);

  await page.getByLabel("足すタイル").selectOption({ label: "期限の近い物と契約" });
  await page.getByRole("button", { name: "タイルを足す" }).click();
  await expect(headings(page).last()).toHaveText("期限の近い物と契約");
  await page.reload();
  await expect(headings(page).last()).toHaveText("期限の近い物と契約");
  await tile(page, "期限の近い物と契約").getByRole("button", { name: "外す" }).click();
  await expect(tile(page, "期限の近い物と契約")).toHaveCount(0);
  await page.getByRole("button", { name: "配置を変える" }).click();
});

test("a press goes on from the last one, however slowly that is saved and read again", async ({ page, request }) => {
  await call(request, "SettingsService", "SetTiles", { tiles: [{ kind: "approvals", x: 0, y: 0, width: 4, height: 4 }] });
  await page.route(/SettingsService\/(SetTiles|GetHome)/, async (route) => {
    await new Promise((done) => setTimeout(done, 500));
    await route.continue();
  });
  await visit(page, "/", /^ホーム · /);
  await page.getByRole("button", { name: "配置を変える" }).click();
  const grip = tile(page, "確認待ち").getByRole("button", { name: "確認待ちを動かす" });
  for (let i = 0; i < 3; i++) await grip.press("ArrowRight");
  await expect.poll(() => savedTiles(request)).toEqual([{ kind: "approvals", x: 3, width: 4, height: 4 }]);
  await page.getByRole("button", { name: "配置を変える" }).click();
});

test("the secretary's tasks show as going on only while they are, and its working memory does not show", async ({ page, request }) => {
  await call(request, "MemoryService", "SetWorking", { name: "e2e-waits", done: "頼んだ", next: "届いたら見る", waiting: "返事を待つ" });
  await record(request, "tasks", "動いている仕事", { fields: { Assignee: "secretary", Status: "in_progress" } });
  await record(request, "tasks", "待っている仕事", { fields: { Assignee: "secretary", Status: "waiting", "Waiting for": "先方の返事" } });
  await call(request, "SettingsService", "SetTiles", { tiles: [{ kind: "aide", x: 0, y: 0, width: 6, height: 10 }] });
  try {
    await visit(page, "/", /^ホーム · /);
    const part = (name: RegExp) => tile(page, /が進めている$/).locator(".home-group").filter({ has: page.getByRole("heading", { name }) });
    await expect(part(/^動いている/)).toContainText("動いている仕事");
    await expect(part(/^待っている/)).toContainText("待っている仕事");
    await expect(part(/^動いている/)).not.toContainText("待っている仕事");
    await expect(page.locator(".tiles")).not.toContainText("e2e-waits");
  } finally {
    await call(request, "MemoryService", "CloseWorking", { id: "e2e-waits" });
  }
});

test("the tiles show today's holds, tasks, memos and close due dates", async ({ page, request }) => {
  const today = dayFromToday(0);
  await record(request, "events", "ホームの仮押さえ", {
    fields: { Status: "hold", Start: `${today} 23:00`, End: `${dayFromToday(1)} 00:30`, Meeting: "https://meet.example.com/home" },
  });
  await record(request, "tasks", "ホームに出るタスク", { fields: { Due: today } });
  await record(request, "memos", "ホームに出るメモ");
  await record(request, "holdings", "ホームに出る保証", {
    sections: [{ name: "Deadlines", items: [{ text: "保証", fields: [{ name: "On", value: dayFromToday(5) }] }] }],
  });
  await call(request, "SettingsService", "SetTiles", {
    tiles: [
      { kind: "today", x: 0, y: 0, width: 6, height: 7 },
      { kind: "approvals", x: 6, y: 0, width: 6, height: 3 },
      { kind: "aide", x: 6, y: 3, width: 6, height: 4 },
      { kind: "task", x: 0, y: 7, width: 6, height: 7 },
      { kind: "memos", x: 6, y: 7, width: 6, height: 4 },
      { kind: "due", x: 6, y: 11, width: 6, height: 4 },
    ],
  });
  await visit(page, "/", /^ホーム · /);

  await expect(tile(page, "今日").getByRole("link", { name: "仮 · ホームの仮押さえ" })).toBeVisible();
  await expect(tile(page, "今日").getByRole("link", { name: "入る" })).toHaveAttribute("href", "https://meet.example.com/home");
  await expect(tile(page, "確認待ち").getByText("いまはない")).toBeVisible();
  await expect(tile(page, "今日と今週のタスク").getByText("ホームに出るタスク")).toBeVisible();
  await expect(tile(page, "最近のメモ").getByRole("link", { name: /ホームに出るメモ/ })).toBeVisible();
  await expect(tile(page, "期限の近い物と契約").getByText("ホームに出る保証")).toBeVisible();

  await tile(page, "最近のメモ").getByRole("link", { name: /ホームに出るメモ/ }).click();
  await expect(page).toHaveURL(/\/memos\?id=/);
});

test("putting the setup card away keeps it away after a reload", async ({ page }) => {
  await visit(page, "/", /^ホーム · /);
  const setup = page.locator("section").filter({ has: page.getByRole("heading", { name: "はじめに" }) });
  await expect(setup.getByRole("link", { name: "Google アカウントをつなぐ（カレンダーとメール）" })).toHaveAttribute(
    "href",
    "/settings/services",
  );
  await expect(setup.getByRole("link", { name: "Slack のワークスペースを足す" })).toHaveAttribute(
    "href",
    "/settings/services",
  );

  await setup.getByRole("button", { name: "あとで" }).click();
  await expect(page.getByRole("heading", { name: "はじめに" })).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole("heading", { name: "はじめに" })).toHaveCount(0);
});
