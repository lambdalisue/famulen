import { expect, type Page, test } from "@playwright/test";
import { dayFromToday, expectView, record, toast, visit } from "./support";

const tab = (page: Page, name: string) => page.getByRole("navigation", { name: "セクション" }).first().getByRole("link", { name, exact: true });

/** Hold every read of records until the returned function lets them go. */
async function holdReads(page: Page) {
  let letGo = () => {};
  const held = new Promise<void>((done) => {
    letGo = done;
  });
  await page.route("**/famulen.v1.RecordService/ListRecords", async (route) => {
    await held;
    await route.continue().catch(() => {});
  });
  return letGo;
}

test("coming back to a page shows what it read before at once, then what is there now", async ({ page, request }) => {
  await record(request, "tasks", "e2e-戻る 前から");
  await visit(page, "/tasks", "タスク");
  await expect(page.getByText("e2e-戻る 前から")).toBeVisible();
  await tab(page, "メモ").click();
  await expectView(page, /^メモ/);

  // The page on screen reads again as the record is made; the one left behind is read when it is come back to.
  const reread = page.waitForResponse("**/famulen.v1.RecordService/ListRecords");
  await record(request, "tasks", "e2e-戻る あとから");
  await reread;
  const letGo = await holdReads(page);
  await tab(page, "タスク").click();

  await expectView(page, "タスク");
  await expect(page.getByText("e2e-戻る 前から")).toBeVisible();
  await expect(page.getByText("e2e-戻る あとから")).toHaveCount(0);
  letGo();
  await expect(page.getByText("e2e-戻る あとから")).toBeVisible();
});

test("a page come back to after a change of the owner's shows what is there now, not what it read before", async ({ page, request }) => {
  await record(request, "tasks", "e2e-戻る 今日済ませる", { fields: { Due: dayFromToday(0) } });
  // The task's due day on the calendar, not its row on the page of tasks.
  const due = page.locator('a.due[title="期日：e2e-戻る 今日済ませる"]');
  await visit(page, "/calendar", /^カレンダー/);
  await expect(due).toBeVisible();
  await tab(page, "タスク").click();
  await expectView(page, "タスク");
  await page.getByRole("button", { name: "e2e-戻る 今日済ませるを済にする" }).click();
  await expect(toast(page)).toHaveText("済にした");

  const letGo = await holdReads(page);
  const reading = page.waitForRequest("**/famulen.v1.RecordService/ListRecords");
  await tab(page, "カレンダー").click();
  await reading;
  await expect(due).toHaveCount(0);
  letGo();
  await expectView(page, /^カレンダー/);
  await expect(due).toHaveCount(0);
});

test("after a change, the page waits for what is there now rather than show what it read before", async ({ page, request }) => {
  await record(request, "tasks", "e2e-戻る 済ませる");
  await visit(page, "/tasks", "タスク");

  const letGo = await holdReads(page);
  await page.getByRole("button", { name: "e2e-戻る 済ませるを済にする" }).click();
  await page.waitForRequest("**/famulen.v1.RecordService/ListRecords");
  await expect(toast(page)).toHaveCount(0);
  letGo();
  await expect(toast(page)).toHaveText("済にした");
});
