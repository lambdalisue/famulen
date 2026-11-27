import { copyFileSync, mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { expect, test } from "@playwright/test";
import { agree, visit } from "./support";

/** What a download leaves, laid down so that nothing is fetched. */
function layDownloaded(...names: string[]) {
  const whisper = join(process.env.E2E_DATA ?? "", "whisper");
  mkdirSync(join(whisper, "engine"), { recursive: true });
  mkdirSync(join(whisper, "models"), { recursive: true });
  copyFileSync(resolve("e2e/fake-whisper.pl"), join(whisper, "engine", "whisper-server"));
  for (const name of names) writeFileSync(join(whisper, "models", `ggml-${name}.bin`), "model");
}

test("a model brought in on this machine is chosen, switched and deleted", async ({ page }) => {
  await visit(page, "/settings/roles/voice", "設定 · 声の聞き取り");
  const here = page.getByRole("region", { name: "このマシンで聞き取る" });
  const row = (name: string) => here.getByRole("listitem").filter({ has: page.getByText(name, { exact: true }) });

  await expect(row("small")).toContainText("488 MB · 速さ: 速い · 精度: 中くらい");
  await expect(row("large-v3-turbo")).toContainText("GB");
  await expect(row("small").getByRole("button", { name: "ダウンロード" })).toBeVisible();

  layDownloaded("small", "base");
  await page.reload();
  await row("small").getByRole("button", { name: "使う" }).click();
  await expect(row("small")).toContainText("使っている", { timeout: 15_000 });
  const chosen = page.getByLabel("使うモデル");
  await expect(chosen).toHaveValue("local-whisper-small");

  await row("base").getByRole("button", { name: "使う" }).click();
  await expect(row("base")).toContainText("使っている", { timeout: 15_000 });
  await expect(row("small").getByRole("button", { name: "使う" })).toBeVisible();
  await expect(chosen).toHaveValue("local-whisper-base");

  for (const name of ["base", "small"]) {
    await row(name).getByRole("button", { name: "削除" }).click();
    await agree(page, "削除する");
    await expect(row(name).getByRole("button", { name: "ダウンロード" })).toBeVisible();
  }
  await expect(chosen).toHaveValue("");
});
