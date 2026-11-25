import { expect, type Page, test } from "@playwright/test";
import { agree, call, dayFromToday, detail, expectView, record, toast, visit } from "./support";

const queued = (page: Page, title: string) => page.locator(".delegated-row").filter({ hasText: title });

async function settle(request: Parameters<typeof call>[0], paused: boolean, slots = 2) {
  await call(request, "WorkerService", "SetWorkerSettings", { slots, paused });
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

test("what keeps a delegated task from starting is told, and none has a place while a slot is free", async ({ page, request }) => {
  await settle(request, false);
  const first = await record(request, "tasks", "先に済ませる仕事", {});
  const delegate = (title: string, more: Record<string, unknown>) =>
    call<{ id: string }>(request, "WorkerService", "DelegateTask", { title, request: "調べる。", runner: "agent", ...more });
  const made = [await delegate("後の仕事", { blockedBy: [first.id] }), await delegate("先の日の仕事", { start: dayFromToday(3) })];
  await visit(page, "/tasks?tab=delegated", "タスク · 委託");
  const held = page.getByRole("region", { name: "始まりを待っている" });
  await expect(held).toContainText("後の仕事");
  await expect(page.getByRole("region", { name: "順番を待っている" })).toHaveCount(0);
  await expect(queued(page, "先の日の仕事")).toContainText("始める日時を待つ");
  await expect(queued(page, "先の日の仕事")).toContainText("日 から");
  await expect(queued(page, "後の仕事")).toContainText("先に済ませる仕事の完了を待っている");
  await expect(queued(page, "後の仕事").locator(".delegated-number")).toHaveText("");

  await queued(page, "後の仕事").getByRole("link", { name: "後の仕事", exact: true }).click();
  const how = detail(page).getByRole("region", { name: "委託の様子" });
  await expect(how).toContainText("先に済ませる仕事の完了を待っている");
  await expect(how).not.toContainText("番目");
  await how.getByRole("link", { name: "先に済ませる仕事" }).click();
  await expectView(page, "タスク · 先に済ませる仕事");
  for (const { id } of made) await call(request, "WorkerService", "CancelDelegated", { id });
  await call(request, "RecordService", "SaveRecord", { kind: "tasks", id: first.id, fields: [{ name: "Status", value: "cancelled" }] });
});

test("only the delegated tasks of the workgroup on screen are listed", async ({ page, request }) => {
  await settle(request, false, 1);
  const made: string[] = [];
  // It keeps the one slot until it is cancelled.
  const busy = await call<{ id: string }>(request, "WorkerService", "DelegateTask", { title: "長い仕事", request: "e2e-hold", runner: "agent", workgroups: ["work"] });
  made.push(busy.id);
  try {
    const states = async () => (await call<{ queue?: { state: string }[] }>(request, "WorkerService", "ListDelegated", { task: busy.id })).queue?.map((t) => t.state);
    await expect.poll(states).toEqual(["running"]);
    for (const [title, workgroup] of [["職場の調べもの", "work"], ["家の調べもの", "personal"]]) {
      const t = await call<{ id: string }>(request, "WorkerService", "DelegateTask", { title, request: "調べる。", runner: "agent", workgroups: [workgroup] });
      made.push(t.id);
    }
    const list = "/tasks?tab=delegated";
    await visit(page, list, "タスク · 委託");
    const place = queued(page, "家の調べもの").locator(".delegated-number");
    await expect(page.getByRole("region", { name: "順番を待っている" })).toContainText("家の調べもの");
    await expect(queued(page, "家の調べもの")).toContainText("順番待ち");
    await expect(place).toHaveText("2");
    await page.evaluate(() => {
      document.cookie = "famulen_workgroup=personal; path=/";
    });
    await visit(page, list, "タスク · 委託");
    await expect(queued(page, "職場の調べもの")).toHaveCount(0);
    await expect(place).toHaveText("2");
    await queued(page, "家の調べもの").getByRole("link").click();
    await expect(detail(page).getByRole("region", { name: "委託の様子" })).toContainText("並びの 2 番目");
    await visit(page, "/tasks", "タスク");
    await expect(page.getByRole("link", { name: /委ねた仕事/ })).toContainText("1 件");
  } finally {
    for (const id of made.reverse()) await call(request, "WorkerService", "CancelDelegated", { id });
    await settle(request, false);
  }
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
  await dialog.getByLabel("リポジトリ").fill("src");
  await dialog.getByLabel("手順").fill("1. レビューする");
  await dialog.getByRole("button", { name: "保存" }).click();
  await expect(toast(page)).toContainText("host/owner/name");
  await dialog.getByLabel("リポジトリ").fill("https://github.com/me/src.git");
  await dialog.getByRole("button", { name: "保存" }).click();
  await expect(dialog).toHaveCount(0);
  const read = section.getByRole("region", { name: "仕上げの詳細" });
  await expect(read.getByRole("listitem")).toHaveText("レビューする");
  await expect(read.getByRole("heading", { name: "github.com/me/src", exact: true })).toBeVisible();

  // The other settings on the page leave it as it is.
  await page.getByText("一時停止する").click();
  await expect.poll(() => call(request, "WorkerService", "GetWorkerSettings")).toMatchObject({ paused: true, finishes: [{ repository: "github.com/me/src" }] });

  await section.getByRole("button", { name: "「github.com/me/src」の仕上げを直す" }).click();
  await page.getByLabel("手順").fill("1. マージする");
  await page.getByRole("button", { name: "保存" }).click();
  await expect(read.getByRole("listitem")).toHaveText("マージする");
  await section.getByRole("button", { name: "「github.com/me/src」の仕上げを消す" }).click();
  await agree(page, "消す");
  await expect(read).toHaveCount(0);
  await expect(section).not.toContainText("github.com/me/src");
  await settle(request, false);
});

test("one finish is picked from the list and read beside it as Markdown, or in its place where there is no room", async ({ page, request }) => {
  await call(request, "WorkerService", "SetWorkerSettings", {
    slots: 2,
    finishes: [
      { repository: "github.com/me/api", steps: "# API の手順\n\n- **全部** 試す" },
      { repository: "", steps: "1. マージする" },
    ],
  });
  await visit(page, "/settings/roles/worker", "設定 · 手代");
  const section = page.getByRole("region", { name: "仕上げ", exact: true });
  const list = section.getByRole("list", { name: "仕上げの一覧" });
  const read = section.getByRole("region", { name: "仕上げの詳細" });
  await expect(list.getByRole("link")).toHaveText(["どの仕事にも", "famulen", "api"]);
  await expect(read).toHaveCount(0);
  await expect(section).not.toContainText("マージする");

  await list.getByRole("link", { name: "api" }).click();
  await expect(read.getByRole("heading", { name: "API の手順" })).toBeVisible();
  await expect(read.locator("strong")).toHaveText("全部");
  await list.getByRole("link", { name: "どの仕事にも" }).click();
  await expect(read.getByRole("listitem")).toHaveText("マージする");
  await page.reload();
  await expect(read.getByRole("listitem")).toHaveText("マージする");
  await expect(list).toBeVisible();

  await page.setViewportSize({ width: 390, height: 844 });
  await expect(read).toBeVisible();
  await expect(list).toBeHidden();
  await read.getByRole("link", { name: "仕上げの一覧" }).click();
  await expect(read).toHaveCount(0);
  await expect(list).toBeVisible();
  await settle(request, false);
});

test("the finish Famulen brings along is put in the owner's words, and back", async ({ page, request }) => {
  await settle(request, false);
  await visit(page, "/settings/roles/worker", "設定 · 手代");
  const section = page.getByRole("region", { name: "仕上げ", exact: true });
  const row = section.getByRole("listitem").filter({ hasText: "github.com/lambdalisue/famulen" });
  await expect(row).toContainText("既定");
  await row.getByRole("link").click();
  const read = section.getByRole("region", { name: "仕上げの詳細" });
  await expect(read).toContainText("just ci");

  await read.getByRole("button", { name: "仕上げを直す" }).click();
  await page.getByLabel("手順").fill("1. 自分で見る");
  await page.getByRole("button", { name: "保存" }).click();
  await expect(read.getByRole("listitem")).toHaveText("自分で見る");
  await expect(row).not.toContainText("既定");
  await read.getByRole("button", { name: "既定に戻す" }).click();
  await agree(page, "戻す");
  await expect(read).toContainText("just ci");
  await expect(row).toContainText("既定");
});

test("how far a delegated task got and what its worker writes are seen as it goes on", async ({ page, request }) => {
  await settle(request, false);
  const t = await call<{ id: string }>(request, "WorkerService", "DelegateTask", { title: "長く調べる", request: "e2e-plan", runner: "agent" });
  await visit(page, `/tasks?tab=delegated&id=${t.id}`, /タスク/);
  await expect(detail(page).getByRole("list", { name: "予定" })).toContainText("書く");
  await expect(queued(page, "長く調べる").getByRole("progressbar")).toHaveAttribute("aria-valuenow", "50");
  await detail(page).getByRole("link", { name: "「長く調べる」の出力を見る" }).click();
  await expectView(page, "タスク · 委託 · 長く調べる");
  const output = page.getByRole("region", { name: "出力" });
  await expect(output).toContainText("まず読む");
  const first = await output.getByRole("listitem").first().elementHandle();
  // It goes on without the page being opened again.
  await expect(output).toContainText("書き終えた", { timeout: 15_000 });
  await expect(page.locator(".run-page .tag")).toHaveText("済んだ", { timeout: 15_000 });
  // What was on screen stays as it was, open or folded.
  expect(await first?.evaluate((e) => e.isConnected)).toBe(true);
});

test.describe("a session that waits on the owner", () => {
  let connection = "";

  test.beforeAll(async ({ request }) => {
    await settle(request, false);
    const added = await call<{ connection: { id: string } }>(request, "ConnectionService", "AddConnection", {
      target: "adroit",
      how: "definition",
      env: "key=k",
    });
    connection = added.connection.id;
  });

  test.afterAll(async ({ request }) => {
    await call(request, "ConnectionService", "DeleteConnection", { id: connection });
  });

  type Pending = { id: string; tool: string };
  const pending = async (request: Parameters<typeof call>[0], tool: string) =>
    ((await call<{ approvals?: Pending[] }>(request, "ConsoleService", "GetInbox")).approvals ?? []).filter((a) => a.tool.includes(tool));

  /** A delegated task whose session asks something, open on screen. */
  async function waiting(page: Page, request: Parameters<typeof call>[0], title: string, prompt: string) {
    const t = await call<{ id: string }>(request, "WorkerService", "DelegateTask", { title, request: prompt, runner: "adroit", directory: "/src" });
    await expect.poll(() => pending(request, "start_session")).toHaveLength(1);
    const [start] = await pending(request, "start_session");
    await call(request, "ConsoleService", "AnswerApproval", { id: start?.id, answer: "APPROVAL_ANSWER_ALLOW" });
    await visit(page, `/tasks?tab=delegated&id=${t.id}`, /タスク/);
    const ask = detail(page).getByRole("group", { name: "セッションの問い" });
    await expect(ask).toBeVisible({ timeout: 60_000 });
    return { ask, task: t.id };
  }

  const left = (page: Page, text: string) =>
    expect(detail(page).getByRole("region", { name: "委託の様子" })).toContainText(text, { timeout: 60_000 });

  test("is answered in the owner's words on the task", async ({ page, request }) => {
    test.setTimeout(150_000);
    const { ask, task } = await waiting(page, request, "言葉で答える仕事", "x");
    await expect(ask).toContainText("このセッションを閉じますか？");
    const words = ask.getByRole("textbox");
    await words.fill("まだ");
    // What is written stays as the page is read again.
    await call(request, "RecordService", "SaveRecord", { kind: "tasks", id: task, title: "言葉で答える仕事・改" });
    await expect(detail(page).getByRole("textbox", { name: "題名" })).toHaveValue("言葉で答える仕事・改");
    await expect(words).toHaveValue("まだ");
    await ask.getByRole("button", { name: "答える", exact: true }).click();
    await expect(toast(page)).toHaveText("答えを届けた");
    await expect(ask).toHaveCount(0);
    await left(page, "済んだ");
    await detail(page).getByRole("button", { name: "残したもの" }).click();
    await left(page, "答え: --reply まだ");
  });

  test("is asked again in the conversation, and answered once from either place", async ({ page, request }) => {
    test.setTimeout(150_000);
    const { ask } = await waiting(page, request, "聞き直す仕事", "x");
    expect(await pending(request, "put_question")).toHaveLength(0);
    for (let i = 0; i < 2; i++) {
      await ask.getByRole("button", { name: "再度聞く" }).click();
      await expect(toast(page)).toHaveText("もう一度尋ねた");
      await expect(ask.getByRole("button", { name: "再度聞く" })).toBeEnabled();
    }
    await expect.poll(() => pending(request, "put_question")).toHaveLength(1);
    await expect(ask).toContainText("でも尋ねている");
    await expect(page.getByTestId("approval")).toContainText("このセッションを閉じますか？");
    await ask.getByRole("button", { name: "残す", exact: true }).click();
    await expect(page.getByTestId("approval")).toHaveCount(0);
    expect(await pending(request, "put_question")).toHaveLength(0);
    await left(page, "済んだ");
  });

  test("is allowed what it asks to do on the task", async ({ page, request }) => {
    test.setTimeout(150_000);
    const { ask } = await waiting(page, request, "許しを待つ仕事", "e2e-permission");
    await expect(ask).toContainText("rm -rf build");
    await ask.getByRole("button", { name: "承認", exact: true }).click();
    await left(page, "済んだ");
    await detail(page).getByRole("button", { name: "残したもの" }).click();
    await left(page, "答え: --allow");
  });
});
