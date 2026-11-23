import { expect, type Page, test } from "@playwright/test";
import { call, toast, visit } from "./support";

type Listed = {
  secrets?: { name: string; description?: string; by: string; automatic?: boolean; model?: string }[];
};

function added(page: Page) {
  return page.getByRole("region", { name: "足した秘密" });
}

function row(page: Page, name: string) {
  return page.locator(".vault-row").filter({ has: page.locator(".vault-name", { hasText: name }) });
}

/** No part of `value` is on the page or in the vault's answer: not whole, not its first or last characters. */
async function nothingOf(page: Page, listed: Listed, value: string) {
  for (const fragment of [value, value.slice(0, 4), value.slice(-4)]) {
    await expect(page.locator("body")).not.toContainText(fragment);
    expect(JSON.stringify(listed)).not.toContain(fragment);
  }
}

test("a secret put in shows without its value, the field empties, and it is taken out after a check on the page", async ({
  page,
  request,
}) => {
  await visit(page, "/settings/vault", "設定 · 金庫");
  await page.getByLabel("名前", { exact: true }).fill("bank-password");
  await page.getByLabel("何に使うか").fill("銀行のサイト");
  await page.getByLabel("中身", { exact: true }).fill("correct-horse-battery");
  await expect(page.getByLabel("中身", { exact: true })).toHaveAttribute("type", "password");
  await page.getByRole("button", { name: "入れる" }).click();

  await expect(toast(page)).toHaveText("金庫に入れた");
  await expect(added(page).locator(row(page, "bank-password"))).toContainText("銀行のサイト");
  await expect(row(page, "bank-password")).toContainText("主人が");
  await expect(page.getByLabel("中身", { exact: true })).toHaveValue("");
  const listed = await call<Listed>(request, "VaultService", "ListSecrets");
  await nothingOf(page, listed, "correct-horse-battery");
  expect(listed.secrets?.[0]).toMatchObject({ name: "bank-password", description: "銀行のサイト", by: "owner" });
  expect(listed.secrets?.[0]).not.toHaveProperty("masked");

  await page.getByRole("button", { name: "bank-passwordを消す" }).click();
  const check = page.getByRole("group", { name: "bank-passwordを消すか確かめる" });
  await check.getByRole("button", { name: "やめる" }).click();
  await expect(row(page, "bank-password")).toBeVisible();

  await page.getByRole("button", { name: "bank-passwordを消す" }).click();
  await check.getByRole("button", { name: "消す" }).click();
  await expect(toast(page)).toHaveText("消した");
  await expect(row(page, "bank-password")).toHaveCount(0);
  await expect(added(page).getByText("まだない")).toBeVisible();
});

test("a secret is replaced from its row, keeps its description, and the new value is not shown either", async ({
  page,
  request,
}) => {
  await call(request, "VaultService", "SetSecretValue", { name: "wifi", value: "first-wifi-pass", purpose: "家" });
  await visit(page, "/settings/vault", "設定 · 金庫");

  await page.getByRole("button", { name: "wifiを置き換える" }).click();
  await page.getByLabel("wifiの新しい中身").fill("zq7-second-wifi-k3v9");
  await page.getByRole("button", { name: "置き換える", exact: true }).click();

  await expect(toast(page)).toHaveText("置き換えた");
  await expect(page.getByLabel("wifiの新しい中身")).toHaveCount(0);
  await expect(row(page, "wifi")).toContainText("家");
  await nothingOf(page, await call<Listed>(request, "VaultService", "ListSecrets"), "zq7-second-wifi-k3v9");
  await call(request, "VaultService", "DeleteSecret", { id: "wifi" });
});

test("a description is rewritten in place, and the secret is still there to replace", async ({ page, request }) => {
  await call(request, "VaultService", "SetSecretValue", { name: "door", value: "dc7-first-code-q4x2", purpose: "玄関" });
  await visit(page, "/settings/vault", "設定 · 金庫");

  await page.getByRole("button", { name: "doorの説明を直す" }).click();
  await page.getByLabel("doorの説明", { exact: true }).fill("裏口");
  await page.getByLabel("doorの説明", { exact: true }).press("Escape");
  await expect(page.getByLabel("doorの説明", { exact: true })).toHaveCount(0);
  await expect(row(page, "door")).toContainText("玄関");

  await page.getByRole("button", { name: "doorの説明を直す" }).click();
  await page.getByLabel("doorの説明", { exact: true }).fill("玄関と勝手口の暗証番号");
  await page.getByRole("button", { name: "直す", exact: true }).click();

  await expect(toast(page)).toHaveText("説明を直した");
  await expect(row(page, "door")).toContainText("玄関と勝手口の暗証番号");
  await expect(page.getByLabel("中身", { exact: true })).toHaveValue("");
  await page.reload();
  await expect(row(page, "door")).toContainText("玄関と勝手口の暗証番号");
  const listed = await call<Listed>(request, "VaultService", "ListSecrets");
  expect(listed.secrets?.find((s) => s.name === "door")).toMatchObject({ description: "玄関と勝手口の暗証番号" });
  await nothingOf(page, listed, "dc7-first-code-q4x2");

  await page.getByRole("button", { name: "doorを置き換える" }).click();
  await page.getByLabel("doorの新しい中身").fill("dc7-second-code-r8y5");
  await page.getByRole("button", { name: "置き換える", exact: true }).click();
  await expect(toast(page)).toHaveText("置き換えた");
  await expect(row(page, "door")).toContainText("玄関と勝手口の暗証番号");
  await call(request, "VaultService", "DeleteSecret", { id: "door" });
});

test("secrets the gateway put in are grouped apart, start folded, say what they belong to, and can be described", async ({
  page,
  request,
}) => {
  const key = "sk-e2e-automatic-0123456789-wxyz";
  const model = await call<{ id: string }>(request, "ModelService", "RegisterModel", {
    name: "e2e の鍵",
    kind: "MODEL_KIND_API",
    model: "m",
    endpoint: "http://127.0.0.1:9/v1",
    apiKey: key,
  });
  const name = `model:${model.id}:api_key`;
  await call(request, "VaultService", "SetSecretValue", { name: "mine", value: "my-own-value", purpose: "自分の" });
  await visit(page, "/settings/vault", "設定 · 金庫");

  await expect(added(page).locator(row(page, "mine"))).toBeVisible();
  await expect(added(page).locator(row(page, name))).toHaveCount(0);
  const folded = page.getByRole("button", { name: /自動で入った秘密/ });
  const listed = await call<Listed>(request, "VaultService", "ListSecrets");
  const automatic = listed.secrets?.filter((s) => s.automatic) ?? [];
  expect(automatic.find((s) => s.name === name)).toMatchObject({ model: "e2e の鍵" });
  await expect(folded).toContainText(String(automatic.length));
  await expect(folded).toHaveAttribute("aria-expanded", "false");
  await expect(row(page, name)).toBeHidden();

  await folded.click();
  await expect(folded).toHaveAttribute("aria-expanded", "true");
  await expect(row(page, name)).toBeVisible();
  await expect(row(page, name)).toContainText("モデル「e2e の鍵」の鍵");
  await expect(page.getByRole("button", { name: `${name}を消す` })).toHaveCount(0);
  await nothingOf(page, listed, key);

  await page.getByRole("button", { name: `${name}の説明を直す` }).click();
  await page.getByLabel(`${name}の説明`).fill("仕事で使う API");
  await page.getByRole("button", { name: "直す", exact: true }).click();
  await expect(toast(page)).toHaveText("説明を直した");
  await expect(row(page, name)).toContainText("仕事で使う API");
  await nothingOf(page, await call<Listed>(request, "VaultService", "ListSecrets"), key);
  const models = await call<{ models?: { id: string; hasKey?: boolean }[] }>(request, "ModelService", "GetModels");
  expect(models.models?.find((m) => m.id === model.id)).toMatchObject({ hasKey: true });

  await call(request, "VaultService", "DeleteSecret", { id: "mine" });
  await call(request, "ModelService", "UnregisterModel", { id: model.id });
});
