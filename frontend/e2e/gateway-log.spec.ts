import { expect, test } from "@playwright/test";
import { call, openLog, toast, visit } from "./support";

type Permissions = { put?: string; delete?: string };

test("the owner chooses on the vault page whether the secretary is asked before changing the vault", async ({
  page,
  request,
}) => {
  await visit(page, "/settings/vault", "設定 · 金庫");
  const put = page.getByRole("group", { name: "秘密を入れる・置き換える" });
  const remove = page.getByRole("group", { name: "秘密を消す" });
  await expect(put.getByRole("button", { name: "尋ねない" })).toHaveAttribute("aria-pressed", "true");
  await expect(remove.getByRole("button", { name: "尋ねる" })).toHaveAttribute("aria-pressed", "true");

  await put.getByRole("button", { name: "尋ねる" }).click();

  await expect(toast(page)).toHaveText("変えた");
  await expect(put.getByRole("button", { name: "尋ねる" })).toHaveAttribute("aria-pressed", "true");
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

  await sheet.getByRole("button", { name: "断った" }).click();
  await expect(sheet.getByRole("button", { name: "断った" })).toHaveAttribute("aria-pressed", "true");
  await expect(sheet.locator(".gateway-log-entry").filter({ hasText: "log-probe" })).toHaveCount(0);

  await sheet.getByRole("button", { name: "すべて" }).click();
  await sheet.getByLabel("呼ぶ者").selectOption("secretary");
  await expect(sheet.locator(".gateway-log-entry").filter({ hasText: "log-probe" })).toHaveCount(0);

  await sheet.getByLabel("呼ぶ者").selectOption("owner");
  await expect(sheet.locator(".gateway-log-entry").filter({ hasText: "log-probe" }).first()).toBeVisible();
  await call(request, "VaultService", "DeleteSecret", { id: "log-probe" });
});
