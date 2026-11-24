import { expect, test } from "@playwright/test";
import { call, openLog, toast, visit } from "./support";

type Tools = { tools: { name: string; permission?: string }[] };

test("the owner forbids the secretary's shell on the built-in tools page and the secretary is refused", async ({
  page,
  request,
}) => {
  await page.goto("/");
  await page.getByPlaceholder(/に頼む$/).fill("道具を見せて");
  await page.getByRole("button", { name: "送る" }).click();
  await expect
    .poll(async () => (await call<Tools>(request, "ConnectionService", "ListBuiltinTools")).tools?.length ?? 0)
    .toBeGreaterThan(0);

  try {
    await visit(page, "/settings/services?builtin=1", "設定 · サービス · 組み込みの道具");
    const bash = page.locator(".list > .item").filter({ has: page.getByText("Bash", { exact: true }) });
    await bash.getByText("上書き", { exact: true }).click();
    await bash.getByText("拒否する", { exact: true }).click();
    await expect(toast(page)).toHaveText("保存した");
    await expect(bash.getByRole("radio", { name: "拒否する" })).toBeChecked();

    await page.getByPlaceholder(/に頼む$/).fill("e2e-bash で ls して");
    await page.getByRole("button", { name: "送る" }).click();

    const sheet = await openLog(page, "/settings/vault", "設定 · 金庫", "ゲートウェイの控え", "控えを見る");
    await sheet.getByRole("radiogroup", { name: "判断" }).getByText("断った").click();
    await expect(sheet.locator(".gateway-log-entry").filter({ hasText: "Bash" }).first()).toContainText("断った");
  } finally {
    await call(request, "ConnectionService", "SetPermission", { scope: "", tool: "Bash", permission: "PERMISSION_UNSPECIFIED" });
  }
});

test("a built-in tool nobody chose a permission for shows まかせる as its default", async ({ page, request }) => {
  await page.goto("/");
  await page.getByPlaceholder(/に頼む$/).fill("道具を見せて");
  await page.getByRole("button", { name: "送る" }).click();
  await expect
    .poll(async () => (await call<Tools>(request, "ConnectionService", "ListBuiltinTools")).tools?.length ?? 0)
    .toBeGreaterThan(0);

  await visit(page, "/settings/services?builtin=1", "設定 · サービス · 組み込みの道具");
  const read = page.locator(".list > .item").filter({ has: page.getByText("Read", { exact: true }) });
  await expect(read.getByRole("radio", { name: "既定（まかせる）" })).toBeChecked();
  const bash = page.locator(".list > .item").filter({ has: page.getByText("Bash", { exact: true }) });
  await expect(bash.getByText("必ず確認する")).toBeVisible();
});
