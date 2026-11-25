import { expect, type Page, test } from "@playwright/test";
import { agree, call, dayFromToday, detail, expectView, field, pane, record, row, toast, visit } from "./support";

function group(page: Page, heading: string) {
  return page.locator("details.rec-group").filter({ has: page.getByRole("heading", { name: heading, exact: true }) });
}

const dialog = (page: Page) => page.getByRole("dialog");

test("a task added in the dialog opens beside the list, and shows under 今日 after a reload", async ({ page }) => {
  await visit(page, "/tasks", "タスク");
  await page.getByRole("button", { name: "タスクを足す" }).click();
  await dialog(page).getByLabel("何をするか").fill("請求書を送る");
  await dialog(page).getByLabel("期日の日付").fill(dayFromToday(0));
  await dialog(page).getByRole("button", { name: "足す", exact: true }).click();

  await expect(toast(page)).toHaveText("足した");
  await expect(page).toHaveURL(/\/tasks\?id=/);
  await expect(detail(page).getByLabel("題名")).toHaveValue("請求書を送る");
  await page.goto("/tasks");
  await expect(group(page, "今日").getByText("請求書を送る")).toBeVisible();
});

test("a task due in a few days goes under 今週, one without a date under いつか", async ({ page, request }) => {
  await record(request, "tasks", "週内の見積もり", { fields: { Due: dayFromToday(3) } });
  await record(request, "tasks", "いつか片付ける本棚");
  await visit(page, "/tasks", "タスク");

  await expect(group(page, "今週").getByText("週内の見積もり")).toBeVisible();
  await expect(group(page, "いつか").getByText("いつか片付ける本棚")).toBeVisible();
});

test("finishing a task moves it under 済, and undoing brings it back", async ({ page, request }) => {
  await record(request, "tasks", "電池を買う");
  await visit(page, "/tasks", "タスク");

  await page.getByRole("button", { name: "電池を買うを済にする" }).click();
  await expect(toast(page)).toHaveText("済にした");
  await page.getByRole("heading", { name: "済", exact: true }).click();
  await expect(group(page, "済").getByText("電池を買う")).toBeVisible();

  await page.getByRole("button", { name: "電池を買うを戻す" }).click();
  await expect(toast(page)).toHaveText("戻した");
  await expect(group(page, "いつか").getByText("電池を買う")).toBeVisible();
});

test("handing a task to the secretary labels it with the secretary, and taking it back labels it あなた", async ({ page, request }) => {
  await record(request, "tasks", "会議室を取る");
  await visit(page, "/tasks", "タスク");
  const task = row(page, "会議室を取る");
  await expect(task.getByRole("button", { name: "任せる" })).toHaveText("あなた");

  await task.getByRole("button", { name: "任せる" }).click();
  await expect(toast(page)).toHaveText(/に任せた$/);
  await page.reload();
  await task.getByRole("button", { name: "自分でやる" }).click();
  await expect(toast(page)).toHaveText("自分でやる");
  await expect(task.getByRole("button", { name: "任せる" })).toHaveText("あなた");
});

test("the list is narrowed by assignee, label and words", async ({ page, request }) => {
  await record(request, "tasks", "自分の歯医者", { labels: ["健康"], notes: "保険証を持つ" });
  await record(request, "tasks", "秘書の下調べ", { fields: { Assignee: "secretary" } });
  await visit(page, "/tasks", "タスク");

  await page.getByLabel("担当で絞る").selectOption("secretary");
  await expect(row(page, "秘書の下調べ")).toBeVisible();
  await expect(row(page, "自分の歯医者")).toHaveCount(0);
  await page.getByLabel("担当で絞る").selectOption("");

  await page.getByRole("button", { name: "健康", exact: true }).click();
  await expect(row(page, "自分の歯医者")).toBeVisible();
  await expect(row(page, "秘書の下調べ")).toHaveCount(0);
  await page.getByRole("button", { name: "健康", exact: true }).click();

  await page.getByLabel("タスクを探す").fill("保険証");
  await expect(row(page, "自分の歯医者")).toBeVisible();
  await expect(row(page, "秘書の下調べ")).toHaveCount(0);
});

test("a task's title, fields, labels, own fields and log are changed where they stand", async ({ page, request }) => {
  const t = await record(request, "tasks", "花を頼む");
  await visit(page, `/tasks?id=${t.id}`, "タスク · 花を頼む");

  await field(page, "状態").getByRole("button", { name: "状態を直す" }).click();
  await field(page, "状態").getByLabel("状態", { exact: true }).selectOption("waiting");
  await expect(toast(page)).toHaveText("保存した");
  await detail(page).getByLabel("項目を足す").selectOption("Waiting for");
  await field(page, "待っているもの").getByLabel("待っているもの", { exact: true }).fill("花屋の返事");
  await field(page, "待っているもの").getByLabel("待っているもの", { exact: true }).press("Tab");
  await detail(page).getByLabel("ラベルを足す").fill("贈り物");
  await detail(page).getByLabel("ラベルを足す").press("Enter");
  await expect(detail(page).getByText("贈り物")).toBeVisible();
  await detail(page).getByLabel("足す項目の名前").fill("色");
  await detail(page).getByLabel("足す項目の値").fill("白");
  await detail(page).getByLabel("足す項目の値").press("Enter");
  await expect(detail(page).getByLabel("色の値")).toHaveValue("白");
  await detail(page).getByLabel("経過を足す").fill("電話した");
  await detail(page).getByLabel("経過を足す").press("Enter");
  await expect(detail(page).getByText("電話した")).toBeVisible();
  await detail(page).getByLabel("題名").fill("花束を頼む");
  await detail(page).getByLabel("題名").press("Tab");
  await expect(toast(page)).toHaveText("保存した");
  await expectView(page, "タスク · 花束を頼む");

  await page.reload();
  await expect(field(page, "状態")).toContainText("待ち");
  await expect(field(page, "待っているもの")).toContainText("花屋の返事");
  await expect(detail(page).getByText("pending → waiting")).toBeVisible();

  await detail(page).getByRole("radiogroup", { name: "見方" }).getByText("原文").click();
  await expect(detail(page).locator(".rec-raw")).toContainText("- Waiting for: 花屋の返事");
  await expect(detail(page).locator(".rec-raw")).toContainText("labels: [贈り物]");
});

test("notes are drawn as Markdown, open the records they link to, and run nothing", async ({ page, request }) => {
  const to = await record(request, "tasks", "つながる先");
  const t = await record(request, "tasks", "備考のあるタスク");
  await visit(page, `/tasks?id=${t.id}`, "タスク · 備考のあるタスク");
  await detail(page).getByRole("button", { name: "備考を書く" }).click();
  await detail(page)
    .getByLabel("備考", { exact: true })
    .fill(`## 手順\n\n- **一つ目**\n\n[[records/tasks/${to.id}|つながる先]]\n\n<script>document.title = "x"</script>`);
  await detail(page).getByRole("button", { name: "保存" }).click();

  await expect(detail(page).getByRole("heading", { name: "手順" })).toBeVisible();
  await expect(detail(page).locator(".markdown-body strong")).toHaveText("一つ目");
  await expect(detail(page).locator(".markdown-body script")).toHaveCount(0);
  await detail(page).locator(".md-link", { hasText: "つながる先" }).click();
  await expect(page).toHaveURL(new RegExp(`id=${to.id}`));
  await expect(detail(page).getByRole("link", { name: "備考のあるタスク" })).toBeVisible();
});

test("a task says what it waits for, and the other says what waits for it", async ({ page, request }) => {
  const first = await record(request, "tasks", "先の仕事");
  const then = await record(request, "tasks", "後の仕事", { fields: { "Blocked by": `[[records/tasks/${first.id}|先の仕事]]` } });
  await visit(page, `/tasks?id=${then.id}`, "タスク · 後の仕事");
  await expect(detail(page).getByText("「先の仕事」が済むまで待っている")).toBeVisible();

  await detail(page).getByRole("link", { name: "先の仕事" }).click();
  await expect(detail(page).locator(".rec-links").filter({ hasText: "これを待つタスク" })).toContainText("後の仕事");
  await detail(page).getByRole("button", { name: "済にする" }).click();
  await expect(toast(page)).toHaveText("済にした");
  await page.getByRole("button", { name: "詳細を閉じる" }).click();
  await expect(row(page, "後の仕事")).toBeVisible();
  await expect(row(page, "後の仕事").getByText("待ち")).toHaveCount(0);
});

test("a task opens beside the list, which stays as it was narrowed, and closing gives the list its room back", async ({ page, request }) => {
  for (const n of ["一", "二", "三"]) await record(request, "tasks", `行き来するタスク${n}`);
  await visit(page, "/tasks", "タスク");
  await page.getByLabel("タスクを探す").fill("行き来する");
  await expect(page.locator("[data-record]")).toHaveCount(3);
  const wide = (await page.locator("[data-record]").first().boundingBox())?.width ?? 0;

  await page.locator("[data-record]").nth(1).getByRole("link").click();
  await expect(page).toHaveURL(/\/tasks\?id=/);
  await expect(page.locator("[data-record]:visible")).toHaveCount(3);
  await expect(page.locator("[data-record]").nth(1).getByRole("link")).toHaveAttribute("aria-current", "true");
  expect((await page.locator("[data-record]").first().boundingBox())?.width ?? 0).toBeLessThan(wide);
  await expect(page.getByText("2 / 3")).toBeVisible();
  const here = await detail(page).getByLabel("題名").inputValue();
  await page.getByRole("button", { name: "次の項目" }).click();
  await expect(page.getByText("3 / 3")).toBeVisible();
  await expect(detail(page).getByLabel("題名")).not.toHaveValue(here);
  await expect(page.getByRole("button", { name: "次の項目" })).toBeDisabled();

  await page.reload();
  await expect(detail(page).getByLabel("題名")).not.toHaveValue(here);
  await page.getByLabel("タスクを探す").fill("行き来する");
  await page.getByRole("button", { name: "詳細を閉じる" }).click();
  await expect(page).toHaveURL(/\/tasks$/);
  await expect(page.getByLabel("タスクを探す")).toHaveValue("行き来する");
  await expect(detail(page)).toHaveCount(0);
});

test("up and down move between the tasks and bring the open one along, and Escape closes it", async ({ page, request }) => {
  for (const n of ["一", "二", "三"]) await record(request, "tasks", `上下のタスク${n}`);
  await visit(page, "/tasks", "タスク");
  await page.getByLabel("タスクを探す").fill("上下のタスク");
  await expect(page.locator("[data-record]")).toHaveCount(3);
  const link = (n: number) => page.locator("[data-record]").nth(n).getByRole("link");
  await link(0).focus();
  await page.keyboard.press("ArrowDown");
  await expect(link(1)).toBeFocused();
  await expect(detail(page)).toHaveCount(0);

  await page.keyboard.press("Enter");
  const title = (await link(1).textContent()) ?? "";
  await expect(detail(page).getByLabel("題名")).toHaveValue(title);
  await page.keyboard.press("ArrowDown");
  await expect(link(2)).toBeFocused();
  await expect(detail(page).getByLabel("題名")).toHaveValue((await link(2).textContent()) ?? "");
  await page.keyboard.press("Home");
  await expect(detail(page).getByLabel("題名")).toHaveValue((await link(0).textContent()) ?? "");

  // However far it moved along the list, the way back is one step.
  await page.goBack();
  await expect(page).not.toHaveURL(/id=/);
  await expect(detail(page)).toHaveCount(0);

  await link(0).click();
  await expect(detail(page)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(detail(page)).toHaveCount(0);
  await expect(link(0)).toBeFocused();
});

test("a record that cannot be read says so and is read again; one that is gone says that", async ({ page, request }) => {
  const t = await record(request, "tasks", "読めないタスク");
  let broken = true;
  await page.route("**/famulen.v1.RecordService/GetRecord**", (route) =>
    broken ? route.fulfill({ status: 500, contentType: "application/json", body: JSON.stringify({ code: "internal", message: "disk is away" }) }) : route.continue(),
  );
  await visit(page, "/tasks", "タスク");
  await row(page, "読めないタスク").getByRole("link").click();
  const there = pane(page, "タスクの詳細");
  await expect(there.getByRole("alert")).toContainText("disk is away");
  await expect(there.getByText("この記録はもうない")).toHaveCount(0);
  broken = false;
  await there.getByRole("button", { name: "読み直す" }).click();
  await expect(detail(page).getByLabel("題名")).toHaveValue("読めないタスク");

  await page.goto("/tasks?id=01ARZ3NDEKTSV4RRFFQ69G5FAV");
  await expect(pane(page, "タスクの詳細").getByText("この記録はもうない")).toBeVisible();
  expect(t.id).not.toBe("");
});

test("leaving notes half written asks before they are lost", async ({ page, request }) => {
  const t = await record(request, "tasks", "書きかけのタスク");
  await visit(page, `/tasks?id=${t.id}`, "タスク · 書きかけのタスク");
  await detail(page).getByRole("button", { name: "備考を書く" }).click();
  await detail(page).getByLabel("備考", { exact: true }).fill("まだ途中");
  await page.getByRole("button", { name: "詳細を閉じる" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "やめる" }).click();
  await expect(detail(page).getByLabel("備考", { exact: true })).toHaveValue("まだ途中");
  await page.getByRole("button", { name: "詳細を閉じる" }).click();
  await agree(page, "捨てる");
  await expect(detail(page)).toHaveCount(0);
});

test("deleting a task asks first and then takes it off the list", async ({ page, request }) => {
  const t = await record(request, "tasks", "古い下書きを捨てる");
  await visit(page, `/tasks?id=${t.id}`, "タスク · 古い下書きを捨てる");
  await page.getByRole("button", { name: "古い下書きを捨てるを消す" }).click();
  await agree(page, "消す");
  await expect(toast(page)).toHaveText("消した");
  await expect(page).toHaveURL(/\/tasks$/);
  await expect(row(page, "古い下書きを捨てる")).toHaveCount(0);
});

test("a task goes beside the composer only once told to, until removed", async ({ page, request }) => {
  const t = await record(request, "tasks", "会話に渡すタスク");
  await visit(page, `/tasks?id=${t.id}`, "タスク · 会話に渡すタスク");
  await expect(detail(page).getByLabel("題名")).toHaveValue("会話に渡すタスク");
  await expect(page.locator("#selection")).toHaveCount(0);
  await detail(page).getByRole("button", { name: "会話に添える" }).click();
  await expect(page.locator("#selection")).toContainText(`タスク：会話に渡すタスク（${t.code}）`);
  await page.locator("#selection").getByRole("button", { name: "外す" }).click();
  await expect(page.locator("#selection")).toHaveCount(0);
});

test("a wake-up the secretary set waits on the task page and can be cancelled", async ({ page, request }) => {
  await call(request, "RoutineService", "WakeAt", { inMinutes: 90, note: "タスクに出る起床予約", why: "主人が返事を待っている" });
  await visit(page, "/tasks", "タスク");
  await expect(page.getByText("タスクに出る起床予約")).toBeVisible();
  await expect(page.getByText("主人が返事を待っている")).toBeVisible();
  await page.getByRole("button", { name: "起床予約「タスクに出る起床予約」を取り消す" }).click();
  await agree(page, "取り消す");
  await expect(toast(page)).toHaveText("起床予約を取り消した");
  await expect(page.getByText("タスクに出る起床予約")).toHaveCount(0);
});

test.describe("定期", () => {
  test("the task page switches between 一度きり and 定期, and 定期 holds no one-off tasks", async ({ page, request }) => {
    await record(request, "tasks", "一度きりの用事");
    await record(request, "routines", "定期に出る仕事", { fields: { Schedule: "0 7 * * *" } });
    await visit(page, "/tasks?tab=routines", "タスク · 定期");
    await expect(row(page, "定期に出る仕事")).toContainText("毎日 7:00");
    await expect(row(page, "一度きりの用事")).toHaveCount(0);

    await page.getByRole("radiogroup", { name: "タスクの種類" }).getByText("一度きり", { exact: true }).click();
    await expectView(page, "タスク");
    await expect(row(page, "一度きりの用事")).toBeVisible();
    await expect(row(page, "定期に出る仕事")).toHaveCount(0);
  });

  test("a weekday routine is added in a dialog and opens with its runs to come", async ({ page }) => {
    await visit(page, "/tasks?tab=routines", "タスク · 定期");
    await page.getByRole("button", { name: "定期タスクを足す" }).click();
    await dialog(page).getByLabel("何をするか").fill("朝の支度");
    await dialog(page).getByRole("radiogroup", { name: "繰り返し" }).getByText("平日").click();
    await dialog(page).getByLabel("時刻").fill("08:30");
    await dialog(page).getByLabel("詳しく").fill("今日の予定をまとめて");
    await dialog(page).getByRole("button", { name: "足す", exact: true }).click();

    await expect(toast(page)).toHaveText("足した");
    await expect(row(page, "朝の支度")).toContainText("平日 8:30");
    await expect(detail(page).getByText("今日の予定をまとめて")).toBeVisible();
    await expect(detail(page).getByRole("button", { name: "この回を飛ばす" })).toHaveCount(5);
  });

  test("closing the dialog with やめる saves nothing", async ({ page }) => {
    await visit(page, "/tasks?tab=routines", "タスク · 定期");
    await page.getByRole("button", { name: "定期タスクを足す" }).click();
    await dialog(page).getByLabel("何をするか").fill("やめた仕事");
    await dialog(page).getByRole("button", { name: "やめる" }).click();
    await expect(dialog(page)).toHaveCount(0);
    await expect(row(page, "やめた仕事")).toHaveCount(0);
  });

  for (const [name, choice, fill, when] of [
    ["夜の振り返り", "毎日", {}, "毎日 21:00"],
    ["週の計画", "毎週", { 曜日: "5" }, "毎週金曜 21:00"],
    ["月末の締め", "毎月", { 日: "25" }, "毎月 25 日 21:00"],
  ] as const) {
    test(`a routine is added ${choice}`, async ({ page }) => {
      await visit(page, "/tasks?tab=routines", "タスク · 定期");
      await page.getByRole("button", { name: "定期タスクを足す" }).click();
      await dialog(page).getByLabel("何をするか").fill(name);
      await dialog(page).getByRole("radiogroup", { name: "繰り返し" }).getByText(choice).click();
      for (const [label, value] of Object.entries(fill)) {
        const control = dialog(page).getByLabel(label, { exact: true });
        if (label === "曜日") await control.selectOption(value);
        else await control.fill(value);
      }
      await dialog(page).getByLabel("時刻").fill("21:00");
      await dialog(page).getByRole("button", { name: "足す", exact: true }).click();
      await expect(row(page, name)).toContainText(when);
    });
  }

  test("a routine is added by its cron line, and a line that cannot be read is refused", async ({ page }) => {
    await visit(page, "/tasks?tab=routines", "タスク · 定期");
    await page.getByRole("button", { name: "定期タスクを足す" }).click();
    await dialog(page).getByLabel("何をするか").fill("見回り");
    await dialog(page).getByRole("radiogroup", { name: "繰り返し" }).getByText("細かく").click();
    await dialog(page).getByLabel("cron").fill("every morning");
    await dialog(page).getByRole("button", { name: "足す", exact: true }).click();
    await expect(toast(page)).toContainText("five fields");
    await dialog(page).getByLabel("cron").fill("*/30 9-18 * * 1-5");
    await dialog(page).getByRole("button", { name: "足す", exact: true }).click();
    await expect(row(page, "見回り")).toContainText("*/30 9-18 * * 1-5");
  });

  test("the next run alone is skipped and put back", async ({ page, request }) => {
    await record(request, "routines", "飛ばせる仕事", { fields: { Schedule: "0 5 * * *" } });
    await visit(page, "/tasks?tab=routines", "タスク · 定期");
    await page.getByRole("button", { name: "飛ばせる仕事の次の回を飛ばす" }).click();
    await expect(toast(page)).toHaveText("この回を飛ばす");
    await page.reload();
    await page.getByRole("button", { name: "飛ばせる仕事の次の回を飛ばさない" }).click();
    await expect(page.getByRole("button", { name: "飛ばせる仕事の次の回を飛ばす" })).toBeVisible();
  });

  test("a condition to skip on is added, and a routine is stopped with its switch", async ({ page, request }) => {
    const r = await record(request, "routines", "止められる仕事", { fields: { Schedule: "0 5 * * *" } });
    await visit(page, `/tasks?tab=routines&id=${r.id}`, "タスク · 止められる仕事");
    await detail(page).getByRole("button", { name: "＋ 飛ばす条件" }).click();
    await detail(page).getByRole("button", { name: "前の回が済むまで" }).click();
    await expect(detail(page).getByText("前の回がまだ済んでいない")).toBeVisible();
    await page.getByRole("button", { name: "詳細を閉じる" }).click();
    await row(page, "止められる仕事").locator(".switch").click();
    await expect(toast(page)).toHaveText("止めた");
    await row(page, "止められる仕事").getByRole("link").click();
    await expect(field(page, "動かす")).toContainText("いいえ");
  });

  test("the routines show on the status page among what is coming", async ({ page, request }) => {
    await record(request, "routines", "状態に出る仕事", { fields: { Schedule: "* * * * *", Start: dayFromToday(1) } });
    await visit(page, "/", /^ホーム · /);
    await page.getByRole("button", { name: "秘書の状態" }).click();
    const coming = page.locator("section").filter({ has: page.getByRole("heading", { name: "これからの予定" }) });
    await expect(coming.getByText(/仕事|支度|振り返り|計画|締め|見回り/).first()).toBeVisible();
  });

  test("settings no longer list the routines", async ({ page }) => {
    await visit(page, "/settings", "設定 · 秘書");
    const nav = page.getByRole("navigation", { name: "設定" });
    await expect(nav.getByRole("link", { name: "定期の仕事" })).toHaveCount(0);
  });
});
