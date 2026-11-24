import { expect, test } from "@playwright/test";
import { call, openLog, toast, visit } from "./support";

type Permissions = { put?: string; delete?: string };

test("the owner chooses on the vault page whether the secretary is asked before changing the vault", async ({
  page,
  request,
}) => {
  await visit(page, "/settings/vault", "設定 · 金庫");
  const put = page.getByRole("radiogroup", { name: "秘密を入れる・置き換える" });
  const remove = page.getByRole("radiogroup", { name: "秘密を消す" });
  await expect(put.getByRole("radio", { name: "許可する" })).toBeChecked();
  await expect(remove.getByRole("radio", { name: "確認する" })).toBeChecked();

  await put.getByText("確認する").click();

  await expect(toast(page)).toHaveText("変えた");
  await expect(put.getByRole("radio", { name: "確認する" })).toBeChecked();
  expect(await call<Permissions>(request, "VaultService", "GetVaultPermissions")).toMatchObject({
    put: "PERMISSION_CONFIRM",
    delete: "PERMISSION_CONFIRM",
  });
  await call(request, "VaultService", "SetVaultPermission", { change: "put" });
});

test("the gateway's log shows what was changed, by whom, without the secret, and narrows by decision and caller", async ({
  page,
  request,
}) => {
  await call(request, "VaultService", "SetSecretValue", { name: "log-probe", value: "log-probe-value" });
  const sheet = await openLog(page, "/settings/vault", "設定 · 金庫", "ゲートウェイの控え", "控えを見る");

  const entry = sheet.locator(".gateway-log-entry").filter({ hasText: "vault.put" }).first();
  await expect(entry).toContainText("通した");
  await expect(entry).toContainText("主人");
  await expect(entry).toContainText("log-probe");
  await expect(page.locator("body")).not.toContainText("log-probe-value");

  const decisions = sheet.getByRole("radiogroup", { name: "判断" });
  await decisions.getByText("断った").click();
  await expect(decisions.getByRole("radio", { name: "断った" })).toBeChecked();
  await expect(sheet.locator(".gateway-log-entry").filter({ hasText: "log-probe" })).toHaveCount(0);

  await decisions.getByText("すべて").click();
  await sheet.getByLabel("呼ぶ者").selectOption("secretary");
  await expect(sheet.locator(".gateway-log-entry").filter({ hasText: "log-probe" })).toHaveCount(0);

  await sheet.getByLabel("呼ぶ者").selectOption("owner");
  await expect(sheet.locator(".gateway-log-entry").filter({ hasText: "log-probe" }).first()).toBeVisible();
  await call(request, "VaultService", "DeleteSecret", { id: "log-probe" });
});
