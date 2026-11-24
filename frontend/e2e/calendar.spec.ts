import { expect, test } from "@playwright/test";
import { agree, pane, record, toast, visit } from "./support";

test("the unit buttons switch between a day, a week and a month around the same date", async ({ page }) => {
  await visit(page, "/calendar?unit=week&at=2026-12-07", "カレンダー · 週 · 12月7日 – 12月13日");
  await expect(page.getByRole("heading", { name: "12月7日 – 12月13日" })).toBeVisible();

  await page.getByRole("link", { name: "日", exact: true }).click();
  await expect(page).toHaveURL(/unit=day&at=2026-12-07/);
  await expect(page.getByRole("heading", { name: "12月7日（月）" })).toBeVisible();

  await page.getByRole("link", { name: "月", exact: true }).click();
  await expect(page).toHaveURL(/unit=month&at=2026-12-07/);
  await expect(page.getByRole("heading", { name: "2026年12月" })).toBeVisible();

  await page.getByRole("link", { name: "週", exact: true }).click();
  await expect(page.getByRole("heading", { name: "12月7日 – 12月13日" })).toBeVisible();
});

test("the arrows step back and forward by the unit shown", async ({ page }) => {
  await visit(page, "/calendar?unit=week&at=2026-12-07", /^カレンダー · 週/);
  await page.getByRole("link", { name: "次へ" }).click();
  await expect(page.getByRole("heading", { name: "12月14日 – 12月20日" })).toBeVisible();
  await page.getByRole("link", { name: "前へ" }).click();
  await page.getByRole("link", { name: "前へ" }).click();
  await expect(page.getByRole("heading", { name: "11月30日 – 12月6日" })).toBeVisible();

  await visit(page, "/calendar?unit=month&at=2026-12-07", /^カレンダー · 月/);
  await page.getByRole("link", { name: "次へ" }).click();
  await expect(page.getByRole("heading", { name: "2027年1月" })).toBeVisible();

  await visit(page, "/calendar?unit=day&at=2026-12-31", /^カレンダー · 日/);
  await page.getByRole("link", { name: "次へ" }).click();
  await expect(page.getByRole("heading", { name: "1月1日（金）" })).toBeVisible();
});

test("without a calendar the page says where to add one", async ({ page }) => {
  await visit(page, "/calendar?unit=week&at=2026-12-07", /^カレンダー/);
  await expect(page.getByRole("link", { name: "カレンダーの節で足す" })).toHaveAttribute("href", "/settings/services");
});

test("a hold added in the dialog shows in the grid, opens with what it says, and is removed", async ({ page }) => {
  await visit(page, "/calendar?unit=week&at=2026-12-07", /^カレンダー · 週/);
  await page.getByRole("button", { name: "予定を足す" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("radiogroup", { name: "予定の種類" }).getByText("仮押さえ").click();
  await dialog.getByLabel("何の予定か").fill("設計の打ち合わせ");
  await dialog.getByLabel("始まりの日").fill("2026-12-08");
  await dialog.getByLabel("終わりの日").fill("2026-12-08");
  await dialog.getByLabel("場所").fill("会議室 B");
  await dialog.getByLabel("会議の URL").fill("https://meet.example.com/abc");
  await dialog.getByLabel("備考").fill("図面を **忘れない**");
  await dialog.getByRole("button", { name: "足す", exact: true }).click();
  await expect(toast(page)).toHaveText("仮押さえした");

  await page.getByRole("button", { name: /仮 · 設計の打ち合わせ/ }).click();
  const sheet = pane(page, "設計の打ち合わせ");
  await expect(sheet.getByText("12月8日（火） 10:00 – 11:00")).toBeVisible();
  await expect(sheet.getByRole("link", { name: "会議に入る" })).toHaveAttribute("href", "https://meet.example.com/abc");
  await expect(sheet.getByRole("link", { name: "地図" })).toHaveAttribute("href", /google\.com\/maps/);
  await expect(sheet.locator(".markdown-body strong")).toHaveText("忘れない");
  await expect(page.locator("#selection")).toContainText("設計の打ち合わせ");
  await sheet.getByRole("button", { name: "設計の打ち合わせを消す" }).click();
  await agree(page, "消す");
  await expect(toast(page)).toHaveText("消した");
  await expect(page.getByRole("button", { name: /設計の打ち合わせ/ })).toHaveCount(0);
});

test("an event's attendees show with their answers", async ({ page, request }) => {
  const p = await record(request, "parties", "参加する田中", { fields: { Kind: "person" } });
  await record(request, "events", "参加者のいる会", {
    fields: { Start: "2026-12-09 13:00", End: "2026-12-09 14:00" },
    sections: [
      {
        name: "Attendees",
        items: [
          { text: `[[records/parties/${p.id}|参加する田中]]`, fields: [{ name: "Response", value: "accepted" }, { name: "Organizer", value: "yes" }] },
          { text: "佐藤", fields: [{ name: "Response", value: "declined" }] },
        ],
      },
    ],
  });
  await visit(page, "/calendar?unit=day&at=2026-12-09", /^カレンダー/);
  await page.getByRole("button", { name: /参加者のいる会/ }).click();
  const people = page.locator(".ev-people");
  await expect(people.locator("li")).toHaveText([/出席.*参加する田中.*主催/, /欠席.*佐藤/]);
  await expect(people.getByRole("link", { name: "参加する田中" })).toHaveAttribute("href", `/parties?id=${p.id}`);
});

test("a repeating event shows on each of its days but the one called off", async ({ page, request }) => {
  await record(request, "events", "毎週の会", {
    fields: { Start: "2026-12-07 09:00", End: "2026-12-07 10:00", Repeat: "FREQ=DAILY;COUNT=3" },
    sections: [{ name: "Exceptions", items: [{ text: "2026-12-08 09:00", fields: [{ name: "Status", value: "cancelled" }] }] }],
  });
  await visit(page, "/calendar?unit=week&at=2026-12-07", /^カレンダー/);
  await expect(page.getByRole("button", { name: /毎週の会/ })).toHaveCount(2);
});

test("a task due on a day opens from its day", async ({ page, request }) => {
  const t = await record(request, "tasks", "年賀状を出す", { fields: { Due: "2026-12-10" } });
  await visit(page, "/calendar?unit=week&at=2026-12-07", /^カレンダー · 週/);
  await page.getByRole("link", { name: "年賀状を出す" }).click();
  await expect(page).toHaveURL(new RegExp(`/tasks\\?id=${t.id}`));
});

test("an event over several days shows on each of them, from midnight to midnight", async ({ page, request }) => {
  await record(request, "events", "夜行で帰る", { fields: { Start: "2026-12-21 22:00", End: "2026-12-22 07:30" } });
  await record(request, "events", "真夜中まで", { fields: { Start: "2026-12-23 20:00", End: "2026-12-24 00:00" } });
  await visit(page, "/calendar?unit=week&at=2026-12-21", /^カレンダー · 週/);
  await expect(page.getByRole("button", { name: /夜行で帰る/ })).toHaveCount(2);
  await expect(page.getByRole("button", { name: /夜行で帰る/ }).nth(0)).toContainText("22:00–");
  await expect(page.getByRole("button", { name: /夜行で帰る/ }).nth(1)).toContainText("–07:30");
  await expect(page.getByRole("button", { name: /真夜中まで/ })).toHaveCount(1);
  await visit(page, "/calendar?unit=month&at=2026-12-21", /^カレンダー · 月/);
  await expect(page.getByRole("button", { name: /夜行で帰る/ })).toHaveCount(2);
});

test("an event opens beside the calendar, which stays", async ({ page, request }) => {
  await record(request, "events", "横に開く会", { fields: { Start: "2026-12-28 10:00", End: "2026-12-28 11:00" } });
  await visit(page, "/calendar?unit=week&at=2026-12-28", /^カレンダー · 週/);
  const event = page.locator(".week").getByRole("button", { name: /横に開く会/ });
  await event.click();
  await expect(pane(page, "横に開く会")).toBeVisible();
  await expect(event).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "予定を閉じる" }).click();
  await expect(pane(page, "横に開く会")).toHaveCount(0);
});
