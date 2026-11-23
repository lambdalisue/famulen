import { createServer, type Server } from "node:http";
import { expect, test } from "@playwright/test";
import { call } from "./support";

type Started = { connection: { id: string }; loginUrl: string };

test.describe("ログインの戻り", () => {
  let service: Server;
  let exchanges = 0;

  test.beforeEach(async () => {
    exchanges = 0;
    service = createServer((request, response) => {
      if (request.url === "/token") exchanges++;
      response.setHeader("content-type", "application/json");
      response.end('{"access_token":"e2e-token"}');
    });
    await new Promise<void>((ready) => service.listen(17793, "127.0.0.1", ready));
  });
  test.afterEach(async () => {
    await new Promise((closed) => service.close(closed));
  });

  test("a login that came back is finished once, and opening the same address again still shows it connected", async ({
    page,
    request,
  }) => {
    const started = await call<Started>(request, "ConnectionService", "AddConnection", {
      target: "e2e-login",
      how: "definition",
      consoleOrigin: "http://127.0.0.1:13000",
    });
    const state = new URL(started.loginUrl).searchParams.get("state") ?? "";
    try {
      await page.goto(`/settings/services?id=${started.connection.id}`);
      await expect(page.getByRole("button", { name: "ログインを続ける" })).toHaveClass(/primary/);

      const back = `/oauth/callback?code=c1&state=${state}`;
      // A look at the address by something that is not the owner's browser
      // finishes nothing.
      await request.get(`http://127.0.0.1:13000${back}`);
      expect(exchanges).toBe(0);

      await page.goto(back);
      await expect(page.getByRole("heading", { name: "E2E Login をつないだ" })).toBeVisible();
      await page.reload();
      await expect(page.getByRole("heading", { name: "E2E Login をつないだ" })).toBeVisible();
      await page.waitForTimeout(1500);
      await expect(page.getByText("つなげなかった")).toHaveCount(0);
      expect(exchanges).toBe(1);

      await page.getByRole("link", { name: "設定へ戻る" }).click();
      await expect(page.getByText("つながっているので、ログインは要らない", { exact: false })).toBeVisible();
      await expect(page.getByRole("button", { name: "ログインする" })).toHaveCount(0);
      await expect(page.getByRole("button", { name: "ログインし直す" })).not.toHaveClass(/primary/);
      await page.getByRole("link", { name: "一覧へ" }).click();
      const defaults = page.locator("section").filter({ has: page.getByRole("heading", { name: "道具の権限の既定" }) });
      await expect(defaults.getByRole("link", { name: /E2E Login/ })).toHaveAttribute(
        "href",
        "/settings/services?kind=e2e-login",
      );

      await page.goto(`/oauth/callback?code=other&state=${state}`);
      await expect(page.getByRole("heading", { name: "つなげなかった" })).toBeVisible();
    } finally {
      await call(request, "ConnectionService", "DeleteConnection", { id: started.connection.id });
    }
  });
});
