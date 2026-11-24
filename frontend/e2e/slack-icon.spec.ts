import { expect, type Page, test } from "@playwright/test";
import { visit } from "./support";

const WRONG = "それは Access Token。下に並ぶ Refresh Token（xoxe- で始まる）を写す";

const OLD = "トークンが古いか、もう使われている。作り直して貼る";

/** A Famulen with one Slack app in two workspaces, whose icon follows once a refresh token is pasted. */
async function withSlack(page: Page) {
  let follows = false;
  await page.route("**/famulen.v1.SettingsService/ListSlackApps**", (route) =>
    route.fulfill({ json: { apps: [{ id: "A0E2E", workspaces: ["Acme", "Beta"], follows }] } }),
  );
  await page.route("**/famulen.v1.SettingsService/LetSlackAppFollow", (route) => {
    const { app, refreshToken } = route.request().postDataJSON() as { app: string; refreshToken: string };
    expect(app).toBe("A0E2E");
    if (refreshToken === "xoxe-1-old") return route.fulfill({ json: { troubles: [OLD] } });
    if (!refreshToken.startsWith("xoxe-")) return route.fulfill({ status: 400, json: { code: "invalid_argument", message: WRONG } });
    follows = true;
    return route.fulfill({ json: {} });
  });
}

test("the list icon is tied to the portrait one step at a time, and a wrong token is told on the spot", async ({ page, context }) => {
  await withSlack(page);
  await context.route("https://api.slack.com/**", (route) => route.fulfill({ body: "slack" }));
  // Reached by a link, so the browser asks for what the test answers.
  await visit(page, "/settings/channels", /チャネル/);
  await page.locator('a[href="/settings"]').first().click();
  const apps = page.getByRole("list", { name: "Slack のアプリ" });
  await expect(apps).toContainText("Acme・Beta");
  await expect(apps).toContainText("つないでいない");

  await apps.getByRole("button", { name: "性別に合わせる" }).click();
  const dialog = page.getByRole("dialog", { name: "一覧のアイコンを性別に合わせる" });
  const now = dialog.locator("[aria-current=step]");
  await expect(now).toContainText("Slack を開く");
  const opened = context.waitForEvent("page");
  await dialog.getByRole("link", { name: "Slack を開く" }).click();
  expect((await opened).url()).toBe("https://api.slack.com/apps");
  await (await opened).close();

  await expect(now).toContainText("トークンを作る");
  await expect(dialog).toContainText("A0E2E");
  await dialog.getByRole("button", { name: "次へ" }).click();
  await expect(now).toContainText("貼る");
  await dialog.getByRole("button", { name: "戻る" }).click();
  await expect(now).toContainText("トークンを作る");
  await dialog.getByRole("button", { name: "次へ" }).click();

  const field = dialog.getByLabel("Refresh Token");
  await expect(field).toHaveAttribute("type", "password");
  await field.fill("xoxe.xoxp-1-a");
  await dialog.getByRole("button", { name: "アイコンを替える" }).click();
  await expect(dialog).toContainText(WRONG);
  await expect(now).toContainText("貼る");
  await field.fill("xoxe-1-old");
  await dialog.getByRole("button", { name: "アイコンを替える" }).click();
  await expect(dialog).toContainText(OLD);
  await expect(field, "what Slack took is not sent twice").toHaveValue("");
  await field.fill("xoxe-1-a");
  await dialog.getByRole("button", { name: "アイコンを替える" }).click();
  await expect(now).toContainText("できた");
  await dialog.getByRole("button", { name: "閉じる" }).last().click();

  await expect(apps).toContainText("つないである");
  await expect(apps.getByRole("button", { name: "つなぎ直す" })).toBeVisible();
});

test("without a Slack app there is nothing to tie", async ({ page }) => {
  await visit(page, "/settings", "設定 · 秘書");
  await expect(page.getByRole("heading", { name: "Slack の一覧のアイコン" })).toHaveCount(0);
});
