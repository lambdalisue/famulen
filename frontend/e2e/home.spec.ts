import { expect, type Page, test } from "@playwright/test";
import { call, dayFromToday, record, visit } from "./support";

function tile(page: Page, heading: string | RegExp) {
  return page.locator("section.tile").filter({ has: page.getByRole("heading", { name: heading, exact: true }) });
}

const headings = (page: Page) => page.locator("section.tile h2");

test.describe.configure({ mode: "serial" });

test("home opens with the default tiles in order", async ({ page, request }) => {
  await call(request, "SettingsService", "SetTiles", {
    tiles: [
      { kind: "today", wide: true, tall: true },
      { kind: "approvals" },
      { kind: "aide" },
      { kind: "task", wide: true, tall: true },
      { kind: "memos", wide: true },
    ],
  });
  await visit(page, "/", /^ホーム · /);
  await expect(headings(page)).toHaveText(["今日", "確認待ち", /が進めている$/, "今日と今週のタスク", "最近のメモ"]);
});

test("the arrange controls show only while arranging, and the choice survives a reload", async ({ page }) => {
  await visit(page, "/", /^ホーム · /);
  await expect(page.getByRole("button", { name: "タイルを足す" })).toBeHidden();

  await page.getByRole("button", { name: "並べ替える" }).click();
  await expect(page.getByRole("button", { name: "タイルを足す" })).toBeVisible();
  await page.reload();
  await expect(page.getByRole("button", { name: "タイルを足す" })).toBeVisible();

  await page.getByRole("button", { name: "並べ替える" }).click();
  await expect(page.getByRole("button", { name: "タイルを足す" })).toBeHidden();
});

test("moving, resizing, adding and removing tiles is kept across a reload", async ({ page }) => {
  await visit(page, "/", /^ホーム · /);
  await page.getByRole("button", { name: "並べ替える" }).click();

  await tile(page, "今日").getByRole("button", { name: "後ろへ" }).click();
  await expect(headings(page).nth(0)).toHaveText("確認待ち");
  await expect(headings(page).nth(1)).toHaveText("今日");

  await tile(page, "今日").getByRole("button", { name: "前へ" }).click();
  await expect(headings(page).nth(0)).toHaveText("今日");

  await expect(tile(page, "確認待ち")).not.toHaveClass(/w2/);
  await tile(page, "確認待ち").getByRole("button", { name: "大きさを変える" }).click();
  await expect(tile(page, "確認待ち")).toHaveClass(/w2/);

  await page.getByLabel("足すタイル").selectOption({ label: "期限の近い物と契約" });
  await page.getByRole("button", { name: "タイルを足す" }).click();
  await expect(headings(page).last()).toHaveText("期限の近い物と契約");

  await page.reload();
  await expect(tile(page, "確認待ち")).toHaveClass(/w2/);
  await expect(headings(page).last()).toHaveText("期限の近い物と契約");

  await tile(page, "期限の近い物と契約").getByRole("button", { name: "外す" }).click();
  await expect(tile(page, "期限の近い物と契約")).toHaveCount(0);
  await tile(page, "確認待ち").getByRole("button", { name: "大きさを変える" }).click();
  await tile(page, "確認待ち").getByRole("button", { name: "大きさを変える" }).click();
  await expect(tile(page, "確認待ち")).not.toHaveClass(/w2/);
  await page.getByRole("button", { name: "並べ替える" }).click();
});

test("a press goes on from the last one, however slowly that is saved and read again", async ({ page, request }) => {
  await call(request, "SettingsService", "SetTiles", { tiles: [{ kind: "today" }, { kind: "approvals" }] });
  await page.route(/SettingsService\/(SetTiles|GetHome)/, async (route) => {
    await new Promise((done) => setTimeout(done, 500));
    await route.continue();
  });
  await visit(page, "/", /^ホーム · /);
  await page.getByRole("button", { name: "並べ替える" }).click();

  const size = tile(page, "確認待ち").getByRole("button", { name: "大きさを変える" });
  await size.click();
  await size.click();
  await tile(page, "確認待ち").getByRole("button", { name: "前へ" }).click();

  await expect(headings(page)).toHaveText(["確認待ち", "今日"]);
  await expect(tile(page, "確認待ち")).toHaveClass(/w2 h2/);
  const home = () => call<{ tiles: unknown[] }>(request, "SettingsService", "GetHome").then((h) => h.tiles);
  await expect.poll(home).toEqual([{ kind: "approvals", wide: true, tall: true }, { kind: "today" }]);
  await page.getByRole("button", { name: "並べ替える" }).click();
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
      { kind: "today", wide: true, tall: true },
      { kind: "approvals" },
      { kind: "aide" },
      { kind: "task", wide: true, tall: true },
      { kind: "memos", wide: true },
      { kind: "due", wide: true },
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
