import { expect, test } from "@playwright/test";
import { call, visit } from "./support";

test("a message sent from the console stays in the chat while pages change", async ({ page }) => {
  await page.goto("/");
  await page.getByPlaceholder(/に頼む$/).fill("今日の予定を教えて");
  await page.getByRole("button", { name: "送る" }).click();
  await expect(page.getByTestId("message").filter({ hasText: "今日の予定を教えて" })).toBeVisible();

  await page.getByRole("navigation", { name: "セクション" }).first().getByRole("link", { name: "タスク" }).click();
  await expect(page).toHaveURL(/\/tasks$/);
  await expect(page.getByTestId("message").filter({ hasText: "今日の予定を教えて" })).toBeVisible();
});

test("choosing a workgroup colors the console and survives a reload", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("tab", { name: "仕事" }).click();
  await expect(page.getByRole("tab", { name: "仕事" })).toHaveAttribute("aria-selected", "true");
  await page.reload();
  await expect(page.getByRole("tab", { name: "仕事" })).toHaveAttribute("aria-selected", "true");
  await page.getByRole("tab", { name: "すべて" }).click();
});

test("after the workgroup changes, only its messages are pushed to the chat", async ({ page, request }) => {
  const { workgroups } = await call<{ workgroups: { id: string; name: string }[] }>(request, "WorkgroupService", "ListWorkgroups");
  const idOf = (name: string) => workgroups.find((w) => w.name === name)?.id;
  const followed: string[] = [];
  page.on("request", (r) => {
    const body = r.url().endsWith("/SubscribeEvents") ? r.postDataBuffer()?.toString() : undefined;
    if (body) followed.push(body);
  });
  await visit(page, "/", /^ホーム/);
  await page.getByRole("tab", { name: "仕事" }).click();
  await expect(page.getByRole("tab", { name: "仕事" })).toHaveAttribute("aria-selected", "true");
  await expect.poll(() => followed.at(-1)).toContain(`"${idOf("仕事")}"`);

  const say = (text: string, name: string) => call(request, "ConsoleService", "Send", { text, workgroup: idOf(name) });
  await say("e2e: 個人の話", "個人");
  await say("e2e: 仕事の話", "仕事");
  await expect(page.getByTestId("message").filter({ hasText: "e2e: 仕事の話" })).toBeVisible();
  await expect(page.getByTestId("message").filter({ hasText: "e2e: 個人の話" })).toHaveCount(0);
  await page.getByRole("tab", { name: "すべて" }).click();
});

test("the page hydrates without errors", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && !m.text().includes("SubscribeEvents")) errors.push(m.text());
  });
  for (const path of ["/", "/calendar", "/tasks", "/memos", "/memory", "/settings", "/settings/services"]) {
    await page.goto(path);
    await expect(page.locator("#pane")).not.toContainText("読み込んでいる");
  }
  expect(errors).toEqual([]);
});

test("the console refuses another host and changes from another site", async ({ request }) => {
  const stranger = await request.get("/", { headers: { Host: "evil.example" } });
  expect(stranger.status()).toBe(421);
  const crossSite = await request.post("/famulen.v1.RecordService/SaveRecord", {
    headers: { Origin: "http://evil.example", "content-type": "application/json" },
    data: { title: "x" },
  });
  expect(crossSite.status()).toBe(403);
});
