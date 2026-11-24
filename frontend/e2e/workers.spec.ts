import { expect, type Page, test } from "@playwright/test";
import { agree, call, detail, expectView, record, toast, visit } from "./support";

const queued = (page: Page, title: string) => page.locator(".delegated-row").filter({ hasText: title });

async function settle(request: Parameters<typeof call>[0], paused: boolean) {
  await call(request, "WorkerService", "SetWorkerSettings", { slots: 2, paused });
}

test("a delegated task waits while paused, runs once let go, and its end reaches the secretary", async ({ page, request }) => {
  await settle(request, true);
  const t = await call<{ id: string; code: string }>(request, "WorkerService", "DelegateTask", {
    title: "相場を調べる",
    request: "椅子の相場を調べて書く。",
    runner: "agent",
    tools: ["WebSearch"],
    priority: "high",
  });
  await visit(page, "/tasks", "タスク");
  await page.getByRole("link", { name: /委ねた仕事/ }).click();
  await expectView(page, "タスク · 委託");
  await expect(queued(page, "相場を調べる")).toContainText("一時停止中");

  await visit(page, "/settings/roles/worker", "設定 · 手代");
  await page.getByText("一時停止する").click();
  await expect(toast(page)).toHaveText("保存した");
  await page.getByRole("button", { name: "済んだ仕事を見る" }).click();
  await expect(page.getByRole("dialog", { name: "済んだ仕事" }).getByText("相場を調べる")).toBeVisible();

  await visit(page, `/tasks?id=${t.id}`, "タスク · 相場を調べる");
  await expect(detail(page).getByRole("region", { name: "委託の様子" })).toContainText("済んだ");
  await expect(detail(page).locator(".rec-log")).toContainText("が済んだ");
  await page.getByRole("button", { name: "詳細を閉じる" }).click();
  await page.getByRole("radiogroup", { name: "タスクの種類" }).getByText("委託").click();
  await expect(page.getByRole("region", { name: "済んだ仕事" }).getByText("相場を調べる")).toBeVisible();
  const { inputs } = await call<{ inputs: { source: string; detail: string }[] }>(request, "ConsoleService", "ListInputs");
  expect(inputs.filter((i) => i.source === "worker" && i.detail === t.code)).toHaveLength(1);
});

test("the owner delegates a task on its page, sees what is missing, and cancels it", async ({ page, request }) => {
  await settle(request, true);
  const t = await record(request, "tasks", "書庫を直す", { notes: "直す。" });
  await visit(page, `/tasks?id=${t.id}`, "タスク · 書庫を直す");
  await detail(page).getByRole("button", { name: "担当を直す" }).click();
  await detail(page).getByLabel("担当", { exact: true }).selectOption("worker");
  await expect(detail(page).getByRole("region", { name: "委託の様子" })).toContainText("委ね方が足りない");

  await detail(page).getByRole("button", { name: "＋ 委ね方" }).click();
  await detail(page).getByLabel("項目を足す").last().selectOption("Runner");
  await detail(page).getByLabel("委託先", { exact: true }).selectOption("agent");
  await expect(detail(page).getByRole("region", { name: "委託の様子" })).toContainText("一時停止中");

  await detail(page).getByRole("button", { name: "委託をやめる" }).click();
  await agree(page, "委託をやめる");
  await expect(toast(page)).toHaveText("やめた");
  await visit(page, "/tasks?tab=delegated", "タスク · 委託");
  await expect(queued(page, "書庫を直す")).toHaveCount(0);
  await settle(request, false);
});

test("only the delegated tasks of the workgroup on screen are listed", async ({ page, request }) => {
  await settle(request, true);
  const made: string[] = [];
  for (const [title, workgroup] of [["職場の調べもの", "work"], ["家の調べもの", "personal"]]) {
    const t = await call<{ id: string }>(request, "WorkerService", "DelegateTask", { title, request: "調べる。", runner: "agent", workgroups: [workgroup] });
    made.push(t.id);
  }
  const list = "/tasks?tab=delegated";
  await visit(page, list, "タスク · 委託");
  const place = queued(page, "家の調べもの").locator(".delegated-number");
  await expect(place).toHaveText(/^[1-9]/);
  const before = (await place.textContent()) ?? "";
  await page.evaluate(() => {
    document.cookie = "famulen_workgroup=personal; path=/";
  });
  await visit(page, list, "タスク · 委託");
  await expect(queued(page, "職場の調べもの")).toHaveCount(0);
  await expect(place).toHaveText(before);
  await queued(page, "家の調べもの").getByRole("link").click();
  await expect(detail(page).getByRole("region", { name: "委託の様子" })).toContainText(`並びの ${before} 番目`);
  await visit(page, "/tasks", "タスク");
  await expect(page.getByRole("link", { name: /委ねた仕事/ })).toContainText("1 件");
  for (const id of made) await call(request, "WorkerService", "CancelDelegated", { id });
  await settle(request, false);
});

test("how many run at once is saved", async ({ page, request }) => {
  await visit(page, "/settings/roles/worker", "設定 · 手代");
  const slots = page.getByRole("spinbutton", { name: "同時に動く数" });
  await slots.fill("3");
  await slots.press("Tab");
  await expect(toast(page)).toHaveText("保存した");
  await page.reload();
  await expect(slots).toHaveValue("3");

  await page.route("**/WorkerService/GetWorkerSettings", async (route) => {
    await new Promise((r) => setTimeout(r, 300));
    await route.continue();
  });
  await slots.fill("4");
  await slots.press("Tab");
  await page.getByText("一時停止する").click();
  await expect
    .poll(() => call(request, "WorkerService", "GetWorkerSettings"))
    .toMatchObject({ slots: 4, paused: true });
  await settle(request, false);
});

test("how work on code is finished is written, rewritten and removed", async ({ page, request }) => {
  await visit(page, "/settings/roles/worker", "設定 · 手代");
  const section = page.getByRole("region", { name: "仕上げ" });
  await section.getByRole("button", { name: "仕上げを足す" }).click();
  const dialog = page.getByRole("dialog", { name: "仕上げを足す" });
  await dialog.getByLabel("ディレクトリ").fill("src");
  await dialog.getByLabel("手順").fill("1. レビューする");
  await dialog.getByRole("button", { name: "保存" }).click();
  await expect(toast(page)).toContainText("absolute path");
  await dialog.getByLabel("ディレクトリ").fill("/src");
  await dialog.getByRole("button", { name: "保存" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(section).toContainText("1. レビューする");

  // The other settings on the page leave it as it is.
  await page.getByText("一時停止する").click();
  await expect.poll(() => call(request, "WorkerService", "GetWorkerSettings")).toMatchObject({ paused: true, finishes: [{ directory: "/src" }] });

  await section.getByRole("button", { name: "「/src」の仕上げを直す" }).click();
  await page.getByLabel("手順").fill("1. マージする");
  await page.getByRole("button", { name: "保存" }).click();
  await expect(section).toContainText("1. マージする");
  await section.getByRole("button", { name: "「/src」の仕上げを消す" }).click();
  await agree(page, "消す");
  await expect(section).toContainText("まだない");
  await settle(request, false);
});
