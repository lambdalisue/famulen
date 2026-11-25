import { expect, test } from "@playwright/test";
import { call, visit } from "./support";

test("what the secretary says is drawn, with its lines where they end", async ({ page, request }) => {
  await call(request, "ConsoleService", "PostMessage", {
    text: "**報告** です\n次の行\n\n- 一つ目\n- 二つ目\n\n`just ci` と *Slack の太字* と <https://example.com/a|リンク>\n\n```\n*そのまま*\n```",
  });
  await visit(page, "/", /^ホーム/);
  const said = page.getByTestId("message").filter({ hasText: "次の行" }).last();
  await expect(said.locator("strong")).toHaveText(["報告", "Slack の太字"]);
  await expect(said.locator("p br")).toHaveCount(1);
  await expect(said.locator("li")).toHaveText(["一つ目", "二つ目"]);
  await expect(said.locator("p code")).toHaveText("just ci");
  await expect(said.getByRole("link", { name: "リンク" })).toHaveAttribute("href", "https://example.com/a");
  await expect(said.locator("pre")).toContainText("*そのまま*");
});

test("what is said runs nothing and fetches nothing", async ({ page, request }) => {
  const asked: string[] = [];
  page.on("request", (r) => asked.push(r.url()));
  await call(request, "ConsoleService", "PostMessage", { text: "e2e-素の HTML <img src=https://evil.example/x.png onerror=alert(1)> ![絵](https://evil.example/y.png)" });
  await visit(page, "/", /^ホーム/);
  await expect(page.getByTestId("message").filter({ hasText: "e2e-素の HTML" }).last().locator(".markdown-body")).toBeVisible();
  expect(asked.filter((u) => u.includes("evil.example"))).toEqual([]);
});

test("the chat is made wider by its edge, and stays so", async ({ page }) => {
  await visit(page, "/", /^ホーム/);
  const chat = page.locator(".chat");
  const width = async () => (await chat.boundingBox())?.width ?? 0;
  expect(await width()).toBe(460);
  await page.mouse.move(460, 400);
  await page.mouse.down();
  await page.mouse.move(520, 400);
  await page.mouse.move(600, 400);
  await page.mouse.up();
  await expect.poll(width).toBe(600);
  await page.reload();
  await expect.poll(width).toBe(600);
  await page.getByRole("separator", { name: "幅を変える" }).dblclick();
  await expect.poll(width).toBe(460);
});

test("a held call says what it does and with what, and keeps its data a press away", async ({ page, request }) => {
  await visit(page, "/", /^ホーム/);
  await page.getByPlaceholder(/に頼む$/).fill("e2e-bash で ls して");
  await page.getByRole("button", { name: "送る" }).click();
  const card = page.getByTestId("approval");
  await expect(card).toContainText("コマンドを実行する");
  const given = card.locator(".asked-rows > div");
  await expect(given).toHaveText(["コマンドls"]);
  await expect(card.locator("pre")).toBeHidden();
  await card.getByRole("button", { name: "元のデータ" }).click();
  await expect(card.locator("pre")).toContainText('"command": "ls"');
  await card.getByRole("button", { name: "却下" }).click();
  await expect(card).toHaveCount(0);
});
