import { expect, test } from "@playwright/test";
import { call, openLog, toast, visit } from "./support";

type Listed = { connections: { id: string; name: string }[] };

test.describe("サービス定義", () => {
  test("a definition is added by filling its fields, its receiver's destination is chosen, and its secret replaced", async ({
    page,
    request,
  }) => {
    await visit(page, "/settings/services", "設定 · サービス");
    await page.getByRole("button", { name: "サービスを足す" }).click();
    const dialog = page.getByRole("dialog", { name: "サービスを足す" });
    await dialog.getByRole("button", { name: /E2E Hook/ }).click();
    await dialog.getByLabel("合言葉").fill("e2e-secret");
    await dialog.getByRole("button", { name: "足す", exact: true }).click();
    await expect(toast(page)).toHaveText("つないだ");

    const listed = await call<Listed>(request, "ConnectionService", "ListConnections", {});
    const id = listed.connections.find((c) => c.name === "E2E Hook")?.id ?? "";
    expect(id).not.toBe("");
    try {
      await visit(page, `/settings/services?id=${id}`, `設定 · サービス · ${id}`);
      const receiver = page.getByLabel("渡す先");
      await expect(receiver).toHaveValue("lookout");
      await receiver.selectOption("calendar");
      await expect(toast(page)).toHaveText("カレンダーに渡す");
      await page.reload();
      await expect(page.getByLabel("渡す先")).toHaveValue("calendar");
      await expect(page.getByRole("button", { name: /ログイン/ })).toHaveCount(0);

      await page.getByLabel("渡す先").selectOption("");
      await expect(toast(page)).toHaveText("受け口を閉じた");
      await page.reload();
      await expect(page.getByLabel("渡す先")).toHaveValue("");
      await page.getByLabel("渡す先").selectOption("calendar");
      await expect(toast(page)).toHaveText("カレンダーに渡す");

      const secret = page.locator("#av-token");
      await expect(secret).toHaveValue("");
      await expect(secret).toHaveAttribute("placeholder", /入っている/);
      await secret.fill("e2e-secret-2");
      await page.getByRole("button", { name: "入れ直す" }).click();
      await expect(toast(page)).toHaveText("金庫に入れた");
      await expect(secret).toHaveValue("");
      await expect(page.locator("body")).not.toContainText("e2e-secret");

      await openLog(page, "/settings/vault", "設定 · 金庫", "ゲートウェイの控え", "控えを見る");
      await expect(page.locator("body")).toContainText("events → calendar");
      await expect(page.locator("body")).not.toContainText("e2e-secret");
    } finally {
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });
});
