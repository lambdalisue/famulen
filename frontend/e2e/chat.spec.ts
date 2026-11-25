import { expect, test } from "@playwright/test";
import { call, sayImage, visit, whollyOnScreen } from "./support";

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

test("an image in the conversation opens over it, whole on the screen, and closes by its button, Escape or a click beside it", async ({ page, request }) => {
  await visit(page, "/", /^ホーム/);
  await sayImage(page, request, "e2e-大きい画像", "大きい.png", { width: 3000, height: 1000 });
  const list = page.locator("#messages");
  const opener = page.getByRole("button", { name: "大きい.png を拡大する" });
  const thumbnail = opener.locator("img");
  await expect.poll(() => thumbnail.evaluate((el: HTMLImageElement) => el.naturalWidth)).toBe(3000);
  const box = page.getByRole("dialog", { name: "大きい.png" });
  const image = box.getByRole("img", { name: "大きい.png" });
  const pages = page.context().pages().length;

  // Read back a little, so that following the newest would show. The image is
  // wholly in sight, as a click would otherwise scroll to it.
  await call(request, "ConsoleService", "PostMessage", { text: `e2e-大きい画像のあと\n\n${"続き。\n\n".repeat(30)}` });
  await expect(page.getByTestId("message").filter({ hasText: "e2e-大きい画像のあと" }).locator(".markdown-body")).toBeVisible();
  const end = () => list.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight);
  await expect.poll(end).toBeLessThan(2);
  await opener.evaluate((el) => el.scrollIntoView({ block: "center" }));
  await expect.poll(end).toBeGreaterThan(100);
  const top = await list.evaluate((el) => el.scrollTop);
  const closings: [string, () => Promise<void>][] = [
    ["button", () => box.getByRole("button", { name: "閉じる" }).click()],
    ["Escape", () => page.keyboard.press("Escape")],
    ["beside", () => page.mouse.click(230, 850)],
  ];
  for (const [how, close] of closings) {
    await opener.click();
    await expect(box, how).toBeVisible();
    await expect(box).toHaveAttribute("aria-modal", "true");
    await expect(box.getByRole("button", { name: "閉じる" })).toBeFocused();
    await expect.poll(() => whollyOnScreen(page, image)).toBe(true);
    const size = await image.boundingBox();
    expect(size?.width ?? 0).toBeGreaterThan(1200);
    expect((size?.width ?? 0) / (size?.height ?? 1)).toBeCloseTo(3, 1);
    await image.click();
    await expect(box, "a click on the image itself").toBeVisible();
    await page.mouse.move(230, 850);
    await page.mouse.wheel(0, -400);
    await close();
    await expect(box, how).toHaveCount(0);
    await expect(opener, how).toBeFocused();
    expect(await list.evaluate((el) => el.scrollTop), how).toBe(top);
  }
  expect(page.context().pages().length).toBe(pages);

  await opener.click();
  const link = box.getByRole("link", { name: "新しいタブで開く" });
  await expect(link).toHaveAttribute("target", "_blank");
  await expect(link).toHaveAttribute("href", (await image.getAttribute("src")) ?? "");
});

test("an image stays open when the conversation is read again", async ({ page, request }) => {
  await visit(page, "/", /^ホーム/);
  await sayImage(page, request, "e2e-開いた画像", "開いた.png", { width: 400, height: 300 });
  await page.getByRole("button", { name: "開いた.png を拡大する" }).click();
  const box = page.getByRole("dialog", { name: "開いた.png" });
  await expect(box).toBeVisible();
  await call(request, "ConsoleService", "PostMessage", { text: "e2e-開いている間に来た" });
  await expect(page.getByTestId("message").filter({ hasText: "e2e-開いている間に来た" })).toBeAttached();
  await expect(box).toBeVisible();
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

test("what the secretary is doing is shown in the conversation from the moment it is asked until it is done", async ({ page, browser }) => {
  await visit(page, "/", /^ホーム/);
  const working = page.getByTestId("working");
  await expect(working).toHaveCount(0);
  await page.getByPlaceholder(/に頼む$/).fill("e2e-work で調べて");
  await page.getByRole("button", { name: "送る" }).click();
  await expect(working).toHaveText("考えています…", { timeout: 1000 });
  await expect(working).toHaveText("ウェブを調べています…");

  const other = await browser.newPage();
  await visit(other, "/", /^ホーム/);
  await expect(other.getByTestId("working")).toHaveText("ウェブを調べています…");
  await other.close();

  await expect(working).toHaveText("考えています…");
  await expect(working).toHaveCount(0);
});

test("an answer takes away what the secretary is doing before it is done", async ({ page, request }) => {
  await visit(page, "/", /^ホーム/);
  const working = page.getByTestId("working");
  await page.getByPlaceholder(/に頼む$/).fill("e2e-work に答えて");
  await page.getByRole("button", { name: "送る" }).click();
  await expect(working).toHaveText("ウェブを調べています…");
  await call(request, "ConsoleService", "PostMessage", { text: "e2e-答えた" });
  await expect(page.getByTestId("message").last()).toContainText("e2e-答えた");
  await expect(working).toHaveCount(0, { timeout: 1000 });
  await expect.poll(async () => (await call(request, "ConsoleService", "GetStatus")).state, { timeout: 15_000 }).toBe("waiting");
});
