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

const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
  "base64",
);

test("files picked, pasted or dropped go along with what is said, and are shown in it", async ({ page }) => {
  await visit(page, "/", /^ホーム/);
  await page.getByTestId("picker").setInputFiles([
    { name: "選んだ.png", mimeType: "image/png", buffer: PNG },
    { name: "e2e-外す.txt", mimeType: "text/plain", buffer: Buffer.from("x") },
  ]);
  const give = (kind: "paste" | "drop", name: string) =>
    page.locator(kind === "paste" ? "#say" : ".chat").evaluate(
      (el, { kind, name }) => {
        const data = new DataTransfer();
        data.items.add(new File(["貼った中身"], name, { type: "text/plain" }));
        const init = { bubbles: true, cancelable: true };
        el.dispatchEvent(
          kind === "paste"
            ? new ClipboardEvent("paste", { ...init, clipboardData: data })
            : new DragEvent("drop", { ...init, dataTransfer: data }),
        );
      },
      { kind, name },
    );
  await give("paste", "貼った.txt");
  await give("drop", "落とした.txt");
  await page.getByRole("button", { name: "e2e-外す.txt を外す" }).click();
  await expect(page.getByTestId("draft")).toHaveText(["選んだ.png", "貼った.txt", "落とした.txt"]);
  await page.getByPlaceholder(/に頼む$/).fill("e2e-添えた");
  await page.getByRole("button", { name: "送る" }).click();

  const said = page.getByTestId("message").filter({ hasText: "e2e-添えた" });
  await expect(page.getByTestId("draft")).toHaveCount(0);
  const image = said.getByRole("img", { name: "選んだ.png" });
  await expect.poll(() => image.evaluate((el: HTMLImageElement) => el.naturalWidth)).toBe(1);
  await expect(said.locator(".attached-file")).toHaveText([/貼った\.txt/, /落とした\.txt/]);

  await page.reload();
  await expect(said.getByRole("img", { name: "選んだ.png" })).toBeVisible();
});

test("a file that cannot be taken says why, and nothing goes without it until it is removed", async ({ page }) => {
  await visit(page, "/", /^ホーム/);
  await page.getByTestId("picker").setInputFiles([
    { name: "e2e.zip", mimeType: "application/zip", buffer: Buffer.from([0x50, 0x4b, 3, 4, 0]) },
    { name: "e2e-空.png", mimeType: "image/png", buffer: PNG },
  ]);
  const bad = page.getByTestId("draft").filter({ hasText: "e2e.zip" });
  await expect(bad).toContainText("どれでもない");
  await page.getByRole("button", { name: "送る" }).click();
  await expect(page.locator("#composer")).toContainText("届いていないファイルがある");
  await page.getByRole("button", { name: "e2e.zip を外す" }).click();
  await page.getByRole("button", { name: "送る" }).click();
  await expect(page.getByTestId("message").last().getByRole("img", { name: "e2e-空.png" })).toBeVisible();
});
