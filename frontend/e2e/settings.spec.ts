import { existsSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type APIRequestContext, expect, type Page, test } from "@playwright/test";
import { call, dayFromToday, expectView, openLog, toast, visit } from "./support";

type Models = {
  roles: { role: string; label: string; choices: string[] }[];
  models: { id: string; name: string }[];
};

/** Open the secretary's state from the indicator at the top right. */
async function openStatus(page: Page) {
  await visit(page, "/", /^ホーム · /);
  await page.getByRole("button", { name: "秘書の状態" }).click();
}

const PAGES: [string, string, string][] = [
  ["ワークグループ", "/settings/workgroups", "ワークグループ"],
  ["秘書", "/settings", "秘書"],
  ["夢", "/settings/roles/dream", "夢"],
  ["見張り", "/settings/lookout", "見張り"],
  ["トリアージ", "/settings/roles/triage", "トリアージ"],
  ["見極め", "/settings/roles/assess", "見極め"],
  ["登録したモデル", "/settings/models", "登録したモデル"],
  ["使った量", "/settings/usage", "使った量"],
  ["チャネル", "/settings/channels", "チャネル"],
  ["サービス", "/settings/services", "サービス"],
  ["金庫", "/settings/vault", "金庫"],
];

test("every settings page opens from the side list", async ({ page }) => {
  await visit(page, "/settings", "設定 · 秘書");
  const nav = page.getByRole("navigation", { name: "設定" });
  for (const [link, path, heading] of PAGES) {
    await nav.getByRole("link", { name: link, exact: true }).click();
    await expect(page).toHaveURL(new RegExp(`${path}$`));
    await expect(page.getByRole("heading", { name: heading, exact: true, level: 1 })).toBeVisible();
  }
});

test.describe("秘書", () => {
  test("the notebook moves to the place written and back to its default", async ({ page, request }) => {
    const { path: before } = await call<{ path: string }>(request, "SettingsService", "GetNotebookPlace");
    await call(request, "RecordService", "SaveRecord", { kind: "memos", title: "移る前のメモ" });
    const target = realpathSync(mkdtempSync(join(tmpdir(), "famulen-notes-")));
    writeFileSync(join(target, "INDEX.md"), "mine");

    await visit(page, "/settings", "設定 · 秘書");
    const section = page.getByRole("region", { name: "手帳の場所" });
    await section.getByLabel("手帳の新しい場所").fill(target);
    await section.getByRole("button", { name: "移す", exact: true }).click();

    const current = section.getByRole("list", { name: "今の手帳の場所" });
    await expect(current).toContainText(target, { timeout: 30_000 });
    expect(existsSync(join(target, "memory", "core.md"))).toBe(true);
    expect(existsSync(before)).toBe(false);
    await expect.poll(async () => JSON.stringify(await call(request, "RecordService", "ListRecords", { kind: "memos" }))).toContain("移る前のメモ");

    await section.getByRole("button", { name: "既定の場所に戻す" }).click();
    await expect(current).toContainText(before, { timeout: 30_000 });
    expect(existsSync(join(target, "INDEX.md"))).toBe(true);
    expect(existsSync(join(target, "memory"))).toBe(false);
  });

  test("a place that cannot hold the notebook is refused", async ({ page }) => {
    await visit(page, "/settings", "設定 · 秘書");
    const section = page.getByRole("region", { name: "手帳の場所" });
    await section.getByLabel("手帳の新しい場所").fill("notes");
    await section.getByRole("button", { name: "移す", exact: true }).click();
    await expect(toast(page)).toContainText("write a whole path");
  });

  test("a new secretary name shows in the chat placeholder and the top bar, and survives a reload", async ({ page }) => {
    await visit(page, "/settings", "設定 · 秘書");
    await page.getByLabel("秘書の名前").fill("ミネルヴァ");
    await page.getByLabel("秘書の名前").press("Tab");

    await expect(toast(page)).toHaveText("保存した");
    await expect(page.getByPlaceholder("ミネルヴァに頼む")).toBeVisible();
    await expect(page.locator(".brand b")).toHaveText("ミネルヴァ");
    await page.reload();
    await expect(page.getByLabel("秘書の名前")).toHaveValue("ミネルヴァ");
    await expect(page.getByPlaceholder("ミネルヴァに頼む")).toBeVisible();
  });

  test("an empty name is refused and the old one is put back", async ({ page }) => {
    await visit(page, "/settings", "設定 · 秘書");
    const before = await page.getByLabel("秘書の名前").inputValue();
    await page.getByLabel("秘書の名前").fill("");
    await page.getByLabel("秘書の名前").press("Tab");
    await expect(toast(page)).toHaveText("秘書には名前が要る");
    await expect(page.getByLabel("秘書の名前")).toHaveValue(before);
  });

  test("the page says where the name is used and that Slack's names come with a workspace", async ({ page }) => {
    await visit(page, "/settings", "設定 · 秘書");
    await expect(page.getByText("秘書の自己紹介", { exact: false })).toBeVisible();
    await expect(page.getByText("Slack のワークスペースを足すと、そこでの名前がここに出る", { exact: false })).toBeVisible();
  });

  test("a way of speaking is picked from the presets and stays picked after a reload", async ({ page }) => {
    await visit(page, "/settings", "設定 · 秘書");
    await expect(page.getByLabel("話し方を自分で書く")).toBeHidden();
    await page.getByText("くだけた", { exact: true }).click();
    await expect(toast(page)).toHaveText("保存した");
    await page.reload();
    await expect(page.getByRole("radio", { name: /くだけた/ })).toBeChecked();
  });

  test("a way of speaking written by hand survives choosing a preset and comes back", async ({ page }) => {
    await visit(page, "/settings", "設定 · 秘書");
    await page.getByText("自分で書く", { exact: true }).click();
    await page.getByLabel("話し方を自分で書く").fill("関西弁で、気さくに。");
    await page.getByLabel("話し方を自分で書く").press("Tab");
    await expect(toast(page)).toHaveText("保存した");

    await page.getByText("箇条書き", { exact: true }).click();
    await expect(page.getByLabel("話し方を自分で書く")).toBeHidden();
    await page.reload();
    await expect(page.getByRole("radio", { name: /箇条書き/ })).toBeChecked();

    await page.getByText("自分で書く", { exact: true }).click();
    await expect(page.getByLabel("話し方を自分で書く")).toHaveValue("関西弁で、気さくに。");
    await page.reload();
    await expect(page.getByRole("radio", { name: /自分で書く/ })).toBeChecked();
  });

  test("the owner adds a place the secretary may write in and takes it away again", async ({ page }) => {
    await visit(page, "/settings", "設定 · 秘書");
    await expect(page.getByRole("heading", { name: "秘書が書ける場所" })).toBeVisible();
    await expect(page.getByText("まだない。")).toBeVisible();

    await page.getByLabel("書ける場所を足す").fill("/srv/e2e-notes");
    await page.getByRole("button", { name: "足す" }).click();
    await expect(toast(page)).toHaveText("足した");
    const list = page.getByRole("list", { name: "秘書が書ける場所" });
    await expect(list.getByText("/srv/e2e-notes")).toBeVisible();
    await expect(page.getByLabel("書ける場所を足す")).toHaveValue("");
    await page.reload();
    await expect(list.getByText("/srv/e2e-notes")).toBeVisible();

    await page.getByLabel("書ける場所を足す").fill("notes");
    await page.getByRole("button", { name: "足す" }).click();
    await expect(toast(page)).toContainText("whole path");

    await page.getByRole("button", { name: "/srv/e2e-notes を外す" }).click();
    await expect(toast(page)).toHaveText("外した");
    await expect(page.getByText("まだない。")).toBeVisible();
  });

  test("several places are picked at once in a wide dialog, and a folder under one picked cannot be", async ({
    page,
    request,
  }) => {
    try {
      await page.setViewportSize({ width: 1400, height: 900 });
      await visit(page, "/settings", "設定 · 秘書");
      const field = page.getByLabel("書ける場所を足す");
      await expect(field).toHaveAttribute("placeholder", "/ か ~/ から書く");
      await field.fill("/");
      await page.getByRole("button", { name: "選ぶ", exact: true }).click();

      const dialog = page.getByRole("dialog", { name: "フォルダを選ぶ" });
      // The box the owner clicks is the label around the visually hidden input.
      const pick = (path: string) =>
        dialog.locator("label", { has: page.getByRole("checkbox", { name: `${path} を選ぶ` }) }).click();
      expect((await dialog.boundingBox())?.width ?? 0).toBeGreaterThan(700);
      await expect(dialog.getByLabel("いまのフォルダ")).toHaveText("/");
      await expect(dialog.getByRole("button", { name: "ひとつ上へ" })).toBeDisabled();
      await expect(dialog.getByRole("checkbox", { name: "/ を選ぶ" })).toBeDisabled();

      await pick("/usr");
      await pick("/var");
      await dialog.getByRole("list", { name: "中のフォルダ" }).getByRole("button", { name: "usr", exact: true }).click();
      await expect(dialog.getByLabel("いまのフォルダ")).toHaveText("/usr");
      const below = dialog.getByRole("checkbox", { name: "/usr/bin を選ぶ" });
      await expect(below).toBeDisabled();
      await expect(below).toBeChecked();
      await expect(dialog.getByText("/usr を選んである").first()).toBeVisible();

      const chosen = dialog.getByRole("list", { name: "選んだフォルダ" });
      await expect(chosen.getByRole("listitem")).toHaveText(["/usr", "/var"]);
      await dialog.getByRole("button", { name: "2 件を足す" }).click();
      await expect(toast(page)).toHaveText("2 件足した");
      await expect(dialog).toHaveCount(0);
      const list = page.getByRole("list", { name: "秘書が書ける場所" });
      await expect(list.getByRole("listitem")).toHaveCount(2);

      await field.fill("/usr/local");
      await expect(page.getByText("/usr の下なので、足さなくても書ける。")).toBeVisible();
      await expect(page.getByRole("button", { name: "足す", exact: true })).toBeDisabled();

      await page.getByRole("button", { name: "選ぶ", exact: true }).click();
      await expect(dialog.getByRole("checkbox", { name: "/usr/local を選ぶ" })).toBeDisabled();
      await expect(dialog.getByText("/usr で書ける").first()).toBeVisible();
    } finally {
      for (const path of ["/usr", "/var"]) await call(request, "SettingsService", "RemoveWritablePlace", { path });
    }
  });

  test("a place above some already there takes their place", async ({ page, request }) => {
    try {
      await call(request, "SettingsService", "AddWritablePlace", { path: "/srv/e2e-sandbox/a" });
      await call(request, "SettingsService", "AddWritablePlace", { path: "/srv/e2e-sandbox/b" });
      await visit(page, "/settings", "設定 · 秘書");
      await page.getByLabel("書ける場所を足す").fill("/srv/e2e-sandbox");
      await expect(page.getByText("下にある /srv/e2e-sandbox/a、/srv/e2e-sandbox/b はこの場所にまとまる", { exact: false })).toBeVisible();
      await page.getByRole("button", { name: "足す", exact: true }).click();
      await expect(toast(page)).toHaveText("足した");
      await expect(page.getByRole("list", { name: "秘書が書ける場所" }).getByRole("listitem")).toHaveText([
        /\/srv\/e2e-sandbox\s*外す/,
      ]);
    } finally {
      await call(request, "SettingsService", "RemoveWritablePlace", { path: "/srv/e2e-sandbox" });
    }
  });

  test("a folder dropped with its whole path fills the field", async ({ page }) => {
    await visit(page, "/settings", "設定 · 秘書");
    const zone = page.getByRole("form", { name: "フォルダを落とすところ" });
    const data = await page.evaluateHandle(() => {
      const d = new DataTransfer();
      d.setData("text/uri-list", "file:///srv/e2e%20drop");
      return d;
    });
    await zone.dispatchEvent("dragover", { dataTransfer: data });
    await expect(zone).toHaveClass(/over/);
    await zone.dispatchEvent("drop", { dataTransfer: data });
    await expect(zone).not.toHaveClass(/over/);
    await expect(page.getByLabel("書ける場所を足す")).toHaveValue("/srv/e2e drop");
  });
});

test.describe("ワークグループ", () => {
  test("全体 heads the workgroup list and holds the time zone, which the secretary's page no longer has", async ({ page }) => {
    await visit(page, "/settings", "設定 · 秘書");
    await expect(page.getByLabel("タイムゾーンを探す")).toHaveCount(0);

    await visit(page, "/settings/workgroups?id=work", "設定 · ワークグループ");
    const list = page.getByRole("navigation", { name: "ワークグループの一覧" });
    await expect(list.getByRole("button").first()).toHaveText("全体");
    await list.getByRole("button", { name: "全体" }).click();
    await expect(page).toHaveURL(/id=all/);
    await expect(page.getByRole("heading", { name: "全体", level: 2 })).toBeVisible();
    await expect(page.getByText("どのワークグループを選んでいても同じように効く", { exact: false })).toBeVisible();
    await expect(page.getByLabel("タイムゾーンを探す")).toBeVisible();
    await expect(page.getByRole("button", { name: "名前と色" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "消す", exact: true })).toHaveCount(0);
  });

  test("a time zone is found by name under 全体, and the map picks the nearest one", async ({ page, request }) => {
    try {
      await visit(page, "/settings/workgroups?id=all", "設定 · ワークグループ");
      await page.getByLabel("タイムゾーンを探す").fill("new_york");
      await page.getByRole("option", { name: /America\/New_York/ }).click();
      await expect(toast(page)).toHaveText("保存した");
      await expect(page.locator(".zone-now")).toContainText("America/New_York");
      await expect(page.getByLabel("タイムゾーンを探す")).toHaveValue("");

      // Tokyo on the 720 x 360 map is at (639.5, 108.7); the view starts at y = 20.
      const map = page.getByRole("img", { name: "タイムゾーンの地図" });
      await map.scrollIntoViewIfNeeded();
      const box = await map.boundingBox();
      if (!box) throw new Error("the map is not on screen");
      const scale = box.width / 720;
      await page.mouse.click(box.x + 639.5 * scale, box.y + (108.7 - 20) * scale);
      await expect(page.locator(".zone-now")).toContainText("Asia/Tokyo");
    } finally {
      // The other tests count days in Tokyo.
      await call(request, "SettingsService", "UpdatePersona", { timezone: "Asia/Tokyo" });
    }
  });

  test("a workgroup is added in a dialog, joins the top bar, and is renamed and deleted in dialogs", async ({ page, request }) => {
    await visit(page, "/settings/workgroups", "設定 · ワークグループ");
    await expect(page.getByLabel("名前", { exact: true })).toHaveCount(0);
    await page.getByRole("button", { name: "ワークグループを足す" }).click();
    const adding = page.getByRole("dialog", { name: "ワークグループを足す" });
    await adding.getByLabel("名前", { exact: true }).fill("趣味");
    await adding.getByText("紫", { exact: true }).click();
    await adding.getByRole("button", { name: "足す", exact: true }).click();

    await expect(toast(page)).toHaveText("ワークグループを足した");
    await expect(adding).toBeHidden();
    const list = page.getByRole("navigation", { name: "ワークグループの一覧" });
    await expect(list.getByRole("button", { name: "趣味" })).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByRole("tab", { name: "趣味" })).toBeVisible();
    const added = (await call<{ workgroups: { name: string; color: string }[] }>(request, "WorkgroupService", "ListWorkgroups")).workgroups;
    expect(added.find((c) => c.name === "趣味")?.color.toLowerCase()).toBe("#6250d6");

    await page.getByRole("button", { name: "名前と色" }).click();
    const editing = page.getByRole("dialog", { name: "名前と色を変える" });
    await editing.getByLabel("名前", { exact: true }).fill("山歩き");
    await editing.getByRole("button", { name: "変える" }).click();
    await expect(toast(page)).toHaveText("保存した");
    await expect(page.getByRole("tab", { name: "山歩き" })).toBeVisible();
    await page.reload();
    await expect(list.getByRole("button", { name: "山歩き" })).toBeVisible();

    await list.getByRole("button", { name: "山歩き" }).click();
    await page.getByRole("button", { name: "消す", exact: true }).click();
    const deleting = page.getByRole("dialog", { name: "「山歩き」を消す" });
    await deleting.getByRole("button", { name: "消す", exact: true }).click();
    await expect(toast(page)).toHaveText("ワークグループを消した");
    await expect(page.getByRole("tab", { name: "山歩き" })).toHaveCount(0);
    await expect(list.getByRole("button", { name: "山歩き" })).toHaveCount(0);
  });

  test("a calendar's way of showing in a workgroup is chosen per workgroup and kept", async ({ page, request }) => {
    const cal = await call<{ id: string }>(request, "CalendarService", "AddCalendar", {
      name: "共有の予定表",
      googleConnection: "google-e2e",
    });
    try {
      await visit(page, "/settings/workgroups?id=work", "設定 · ワークグループ");
      const modes = page.getByRole("radiogroup", { name: "共有の予定表の出し方" });
      await modes.getByText("薄く出す", { exact: true }).click();
      await expect(toast(page)).toHaveText("保存した");
      await expect(modes.getByRole("radio", { name: "薄く出す" })).toBeChecked();
      await page.reload();
      await expect(modes.getByRole("radio", { name: "薄く出す" })).toBeChecked();
      await expect(modes.getByRole("radio", { name: "出す", exact: true })).not.toBeChecked();
    } finally {
      await call(request, "CalendarService", "RemoveCalendar", { id: cal.id });
    }
  });

  test("something is put into a workgroup with its switch and taken out again", async ({ page }) => {
    await visit(page, "/settings/workgroups?id=work", "設定 · ワークグループ");
    const row = page.locator(".wg-switch").first();
    const was = await row.getAttribute("data-state");
    await row.click();
    await expect(toast(page)).toHaveText("保存した");
    await expect(row).toHaveAttribute("data-state", was === "checked" ? "unchecked" : "checked");
    await row.click();
    await expect(row).toHaveAttribute("data-state", was ?? "unchecked");
  });
});

test.describe("役のモデル", () => {
  test.describe.configure({ mode: "serial" });

  const modelPicker = (page: Page) => page.getByRole("combobox", { name: "使うモデル" });

  /** Pick from the role's model menu, the way the owner does. */
  async function chooseModel(page: Page, name: string) {
    const picker = modelPicker(page);
    await picker.scrollIntoViewIfNeeded();
    await picker.click();
    await page.getByRole("option", { name, exact: true }).click();
  }

  test("a model chosen on a role's page is kept after a reload and leaves the other roles alone", async ({ page, request }) => {
    const models = await call<Models>(request, "ModelService", "GetModels");
    const secretary = models.roles.find((r) => r.role === "ROLE_SECRETARY");
    const dream = models.roles.find((r) => r.role === "ROLE_DREAM");
    const pick = (ids: string[] | undefined) => {
      const all = models.models;
      return all.find((m) => ids?.includes(m.id));
    };
    const forSecretary = pick(secretary?.choices);
    const forDream = pick(dream?.choices);
    expect(forSecretary).toBeDefined();
    expect(forDream).toBeDefined();

    await visit(page, "/settings", "設定 · 秘書");
    await chooseModel(page, forSecretary?.name ?? "");
    await expect(toast(page)).toHaveText("保存した");
    await visit(page, "/settings/roles/dream", "設定 · 夢");
    await chooseModel(page, forDream?.name ?? "");
    await expect(toast(page)).toHaveText("保存した");

    await page.reload();
    await expect(modelPicker(page)).toHaveText(forDream?.name ?? "");
    await visit(page, "/settings", "設定 · 秘書");
    await expect(modelPicker(page)).toHaveText(forSecretary?.name ?? "");

    await visit(page, "/settings/models", "設定 · 登録したモデル");
    const registered = page.locator("section").filter({ has: page.getByRole("heading", { name: "一覧", exact: true }) });
    const used = registered.locator(".item").filter({ hasText: forDream?.name ?? "" }).first();
    await expect(used.getByRole("link", { name: "夢", exact: true })).toHaveAttribute("href", "/settings/roles/dream");
  });

  test("the lookout and triage each choose their model on their own page", async ({ page }) => {
    await visit(page, "/settings/lookout", "設定 · 見張り");
    await expect(modelPicker(page)).toBeVisible();
    await visit(page, "/settings/roles/triage", "設定 · トリアージ");
    await expect(modelPicker(page)).toBeVisible();
    await visit(page, "/settings/models", "設定 · 登録したモデル");
    await expect(modelPicker(page)).toHaveCount(0);
  });

  test("the assessor chooses its model on its own page and opens what it decided in a log sheet", async ({ page }) => {
    await visit(page, "/settings/roles/assess", "設定 · 見極め");
    await expect(modelPicker(page)).toBeVisible();
    const sheet = await openLog(page, "/settings/roles/assess", "設定 · 見極め", "見極めのログ");
    await expect(sheet.getByLabel("呼ぶ者")).toHaveCount(0);
    await expect(sheet.getByLabel("つなぎ先")).toBeVisible();
  });

  test("a daily cap is saved for its own role only", async ({ page, request }) => {
    const capOf = async (role: string) =>
      (await call<{ roles: { role: string; cap?: number }[] }>(request, "ModelService", "GetModels")).roles.find(
        (r) => r.role === role,
      )?.cap ?? 0;
    const triageBefore = await capOf("ROLE_TRIAGE");

    await visit(page, "/settings/roles/dream", "設定 · 夢");
    const cap = page.getByLabel("一日の上限");
    await cap.fill("3");
    await cap.press("Tab");
    await expect(toast(page)).toHaveText("保存した");
    await page.reload();
    await expect(page.getByLabel("一日の上限")).toHaveValue("3");
    expect(await capOf("ROLE_DREAM")).toBe(3);
    expect(await capOf("ROLE_TRIAGE")).toBe(triageBefore);
  });

  test("tidying records is turned off on the dream's page and stays so after a reload", async ({ page }) => {
    await visit(page, "/settings/roles/dream", "設定 · 夢");
    const tidy = page.getByRole("checkbox", { name: "記録も整える" });
    await expect(tidy).toBeChecked();
    await page.getByText("記録も整える", { exact: true }).click();
    await expect(toast(page)).toHaveText("保存した");
    await page.reload();
    await expect(tidy).not.toBeChecked();
  });

  test("choosing 使わない takes the model off a role", async ({ page }) => {
    await visit(page, "/settings/roles/dream", "設定 · 夢");
    await chooseModel(page, "使わない");
    await expect(toast(page)).toHaveText("保存した");
    await page.reload();
    await expect(modelPicker(page)).toHaveText("使わない");
  });

  test("the dream's effort is set on its slider, kept after a reload and apart from the secretary's", async ({
    page,
    request,
  }) => {
    const roles = async () =>
      (
        await call<{ roles: { role: string; model?: string; effort?: string }[] }>(request, "ModelService", "GetModels")
      ).roles;
    const secretaryBefore = (await roles()).find((r) => r.role === "ROLE_SECRETARY")?.effort ?? "";
    await useClaudeForDream(request);

    await visit(page, "/settings/roles/dream", "設定 · 夢");
    const effort = page.getByRole("slider", { name: "考える深さ" });
    await effort.scrollIntoViewIfNeeded();
    await expect(page.getByText("モデルの既定", { exact: true })).toBeVisible();
    await effort.focus();
    await effort.press("End");
    await expect(toast(page)).toHaveText("保存した");
    await expect(effort).toHaveAttribute("aria-valuetext", "最大");

    await page.reload();
    await expect(page.getByRole("slider", { name: "考える深さ" })).toHaveAttribute("aria-valuetext", "最大");
    const after = await roles();
    expect(after.find((r) => r.role === "ROLE_DREAM")?.effort).toBe("max");
    expect(after.find((r) => r.role === "ROLE_SECRETARY")?.effort ?? "").toBe(secretaryBefore);
  });

  test("a step on the effort slider is saved, and 既定に戻す returns to the agent's own default", async ({ page, request }) => {
    const roles = async () =>
      (await call<{ roles: { role: string; model?: string; effort?: string }[] }>(request, "ModelService", "GetModels"))
        .roles;
    await useClaudeForDream(request);
    await call(request, "ModelService", "AssignRole", {
      role: "ROLE_DREAM",
      model: (await roles()).find((r) => r.role === "ROLE_DREAM")?.model,
      effort: "high",
    });

    await visit(page, "/settings/roles/dream", "設定 · 夢");
    const effort = page.getByRole("slider", { name: "考える深さ" });
    await effort.scrollIntoViewIfNeeded();
    await effort.focus();
    await effort.press("ArrowLeft");
    await expect(toast(page)).toHaveText("保存した");
    await expect(effort).toHaveAttribute("aria-valuetext", "ふつう");

    const reset = page.getByRole("button", { name: "既定に戻す" });
    await expect(reset).toHaveAttribute("title", "このエージェントの既定は「とても深い」");
    await reset.scrollIntoViewIfNeeded();
    await reset.click();
    await expect(effort).toHaveAttribute("aria-valuetext", "とても深い");
    await expect(reset).toHaveCount(0);
    await page.reload();
    await expect(effort).toHaveAttribute("aria-valuetext", "とても深い");
    expect((await roles()).find((r) => r.role === "ROLE_DREAM")?.effort).toBe("xhigh");
  });

  test("the secretary's page no longer asks for a fallback executable", async ({ page }) => {
    await visit(page, "/settings", "設定 · 秘書");
    await expect(modelPicker(page)).toBeVisible();
    await expect(page.getByLabel("割り当てがないときの実行ファイル")).toHaveCount(0);
  });
});

test.describe("夢の頁", () => {
  test.describe.configure({ mode: "serial" });

  type Plan = { at?: string; frequency?: string; weekday?: number; earlyAt?: number; coreLines?: number; next?: string };
  const plan = (request: Parameters<typeof call>[0]) => call<Plan>(request, "MemoryService", "GetDreamSettings");

  test.afterAll(async ({ request }) => {
    await call(request, "MemoryService", "SetDreamSettings", {
      at: "04:00",
      frequency: "DREAM_FREQUENCY_NIGHTLY",
      earlyAt: 30,
      coreLines: 150,
    });
  });

  test("when to dream is chosen on the page, kept after a reload, and says the next dream", async ({
    page,
    request,
  }) => {
    await visit(page, "/settings/roles/dream", "設定 · 夢");
    const when = page.locator("section.pref").filter({ has: page.getByRole("heading", { name: "いつ見るか" }) });
    await expect(when.getByRole("radio", { name: "毎晩" })).toBeChecked();
    await expect(when.getByRole("radiogroup", { name: "曜日" })).toHaveCount(0);

    await when.getByText("週に一度", { exact: true }).click();
    await expect(toast(page)).toHaveText("保存した");
    await when.getByText("水", { exact: true }).click();
    await expect.poll(async () => (await plan(request)).weekday).toBe(3);

    await when.getByRole("combobox", { name: "時刻" }).click();
    await page.getByRole("option", { name: "23:30", exact: true }).click();
    await expect.poll(async () => (await plan(request)).at).toBe("23:30");

    const early = when.getByLabel("早めに見る");
    await early.fill("0");
    await early.press("Tab");
    await expect.poll(async () => (await plan(request)).earlyAt ?? 0).toBe(0);

    await page.reload();
    await expect(when.getByRole("radio", { name: "週に一度" })).toBeChecked();
    await expect(when.getByRole("radio", { name: "水" })).toBeChecked();
    await expect(when.getByRole("combobox", { name: "時刻" })).toHaveText(/23:30/);
    await expect(when.getByLabel("早めに見る")).toHaveValue("0");
    const saved = await plan(request);
    expect(saved.frequency).toBe("DREAM_FREQUENCY_WEEKLY");
    await expect(when.getByText(`次は ${saved.next}`)).toBeVisible();
  });

  for (const [label, frequency] of [
    ["早めに見る", "一日おき"],
    ["一日の上限", "毎晩"],
  ] as const) {
    test(`a number being typed into ${label} outlives the reload after another save`, async ({ page }) => {
      await visit(page, "/settings/roles/dream", "設定 · 夢");
      let release = () => {};
      const held = new Promise<void>((done) => {
        release = done;
      });
      await page.route("**/famulen.v1.*/Get*", async (route) => {
        await held;
        await route.continue();
      });
      await page.route("**/famulen.v1.*/List*", async (route) => {
        await held;
        await route.continue();
      });
      const when = page.locator("section.pref").filter({ has: page.getByRole("heading", { name: "いつ見るか" }) });
      await when.getByText(frequency, { exact: true }).click();
      const field = page.getByRole("spinbutton", { name: label });
      await field.fill("7");
      release();
      await expect(toast(page)).toHaveText("保存した");
      await expect(field).toHaveValue("7");
    });
  }

  test("the core limit is set on the page and kept after a reload", async ({ page, request }) => {
    await visit(page, "/settings/roles/dream", "設定 · 夢");
    const lines = page.getByLabel("記憶の核");
    await expect(lines).toHaveValue("150");
    await lines.fill("80");
    await lines.press("Tab");
    await expect(toast(page)).toHaveText("保存した");
    await page.reload();
    await expect(page.getByLabel("記憶の核")).toHaveValue("80");
    expect((await plan(request)).coreLines).toBe(80);
  });

  test("今すぐ夢を見る sits at the foot of the page and is off while no dream model is chosen", async ({
    page,
    request,
  }) => {
    await call(request, "ModelService", "AssignRole", { role: "ROLE_DREAM", model: "" });
    await visit(page, "/settings/roles/dream", "設定 · 夢");
    await expect(page.getByRole("button", { name: "今すぐ夢を見る" })).toBeDisabled();
    await expect(page.getByText("夢のモデルが決まっていない。設定の夢で決める")).toBeVisible();

    await useClaudeForDream(request);
    await page.reload();
    await expect(page.getByRole("button", { name: "今すぐ夢を見る" })).toBeEnabled();
    await call(request, "ModelService", "AssignRole", { role: "ROLE_DREAM", model: "" });
  });
});

/** Give the dream a Claude Code model with the model's own effort, whatever earlier tests left. */
async function useClaudeForDream(request: Parameters<typeof call>[0]) {
  type Listed = { id: string; kind?: string };
  const models = await call<{
    roles: { role: string; choices: string[] }[];
    models: Listed[];
  }>(request, "ModelService", "GetModels");
  const choices = models.roles.find((r) => r.role === "ROLE_DREAM")?.choices ?? [];
  const claude = models.models.find(
    (m) => m.kind === "MODEL_KIND_AGENT_CLI" && choices.includes(m.id),
  );
  expect(claude, "an agent CLI the dream can use").toBeDefined();
  await call(request, "ModelService", "AssignRole", { role: "ROLE_DREAM", model: claude?.id, effort: "" });
}

test.describe("モデル", () => {
  test("a priced model is registered from the form with its price fields and can be removed", async ({ page }) => {
    await visit(page, "/settings/models", "設定 · 登録したモデル");
    await page.getByRole("button", { name: "モデルの追加" }).click();
    const dialog = page.getByRole("dialog", { name: "モデルの追加" });
    await expect(dialog.getByLabel("API キー")).toHaveCount(0);
    await dialog.locator(".model-way").filter({ hasText: "鍵で呼ぶ" }).click();
    await expect(dialog.getByLabel("URL")).toHaveValue("https://api.openai.com/v1");
    await dialog.getByText("ほか", { exact: true }).click();
    await dialog.getByLabel("URL").fill("https://api.example.com/v1");
    await dialog.getByLabel("API キー").fill("sk-e2e");
    await dialog.getByLabel("モデル名", { exact: true }).fill("gpt-e2e");
    await dialog.getByLabel("入力 100 万トークンあたり").fill("150");
    await dialog.getByLabel(/^名前/).fill("仕事用の GPT");
    await dialog.getByRole("button", { name: "追加", exact: true }).click();

    await expect(toast(page)).toHaveText("足した");
    const registered = page.locator("section").filter({ has: page.getByRole("heading", { name: "一覧", exact: true }) });
    const row = registered.locator(".item").filter({ hasText: "仕事用の GPT" });
    await expect(row).toContainText("API");
    await expect(row).toContainText("従量");
    await expect(dialog).toBeHidden();
    await page.reload();
    await expect(row).toContainText("gpt-e2e");
    await expect(row).toContainText("API キーは金庫にある");
    await expect(row).not.toContainText("sk-e2e");

    const replaced = "sk-e2e-replaced-0123456789abcdef";
    await row.getByRole("button", { name: "鍵を入れ替える" }).click();
    await row.getByLabel("新しい API キー").fill(replaced);
    await row.getByRole("button", { name: "入れ替える", exact: true }).click();
    await expect(toast(page)).toHaveText("入れ替えた");
    await expect(row.getByLabel("新しい API キー")).toHaveCount(0);
    await expect(row).toContainText("API キーは金庫にある");
    for (const fragment of [replaced, replaced.slice(0, 4), replaced.slice(-4)]) {
      await expect(page.locator("body")).not.toContainText(fragment);
    }

    await row.getByRole("button", { name: "外す" }).click();
    await expect(toast(page)).toHaveText("外した");
    await expect(row).toHaveCount(0);
  });
});

test.describe("モデルの自動の登録と追加", () => {
  const list = (page: Page) =>
    page.locator("section").filter({ has: page.getByRole("heading", { name: "一覧", exact: true }) });
  const openAdd = async (page: Page) => {
    await page.getByRole("button", { name: "モデルの追加" }).click();
    return page.getByRole("dialog", { name: "モデルの追加" });
  };

  test("Claude Code's models are listed by themselves; a removed one stays out until brought back", async ({ page }) => {
    await visit(page, "/settings/models", "設定 · 登録したモデル");
    const row = list(page).locator(".item").filter({ hasText: "claude-opus-5" });
    await expect(row).toContainText("Claude Opus 5");
    await row.getByRole("button", { name: "外す" }).click();
    await expect(toast(page)).toHaveText("外した");
    await page.getByRole("button", { name: "探し直す" }).click();
    await expect(toast(page)).toHaveText("探し直した");
    await expect(row).toHaveCount(0);

    const dialog = await openAdd(page);
    await dialog.getByRole("button", { name: "Claude Opus 5を戻す" }).click();
    await expect(toast(page)).toHaveText("戻した");
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect(row).toHaveCount(1);
  });

  test("a local server's model is picked from what the server has", async ({ page }) => {
    const server = createServer((_, res) => {
      res.setHeader("content-type", "application/json");
      res.end(JSON.stringify({ data: [{ id: "local-a" }, { id: "local-b" }] }));
    });
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const address = server.address();
    const port = typeof address === "object" && address ? address.port : 0;
    await visit(page, "/settings/models", "設定 · 登録したモデル");
    const dialog = await openAdd(page);
    await dialog.locator(".model-way").filter({ hasText: "ローカルのサーバー" }).click();
    await expect(dialog.getByLabel("API キー")).toHaveCount(0);
    await dialog.getByLabel("URL").fill(`http://127.0.0.1:${port}/v1`);
    await dialog.getByRole("button", { name: "モデルを探す" }).click();
    await dialog.getByRole("button", { name: "local-b" }).click();
    await expect(dialog.getByLabel("モデル名", { exact: true })).toHaveValue("local-b");
    await dialog.getByRole("button", { name: "追加", exact: true }).click();
    await expect(toast(page)).toHaveText("足した");
    server.close();

    const row = list(page).locator(".item").filter({ hasText: "local-b" });
    await expect(row).toContainText("ローカル");
    await row.getByRole("button", { name: "外す" }).click();
    await expect(row).toHaveCount(0);
  });

  test("an agent CLI is asked only for its program, model and name", async ({ page }) => {
    await visit(page, "/settings/models", "設定 · 登録したモデル");
    const dialog = await openAdd(page);
    await dialog.locator(".model-way").filter({ hasText: "エージェント CLI" }).click();
    await dialog.getByText("Codex", { exact: true }).click();
    await expect(dialog.getByText("秘書には使えない。")).toBeVisible();
    await expect(dialog.getByLabel("URL")).toHaveCount(0);
    await expect(dialog.getByLabel("API キー")).toHaveCount(0);
    await dialog.getByRole("button", { name: "やめる" }).click();
    await expect(dialog).toBeHidden();
  });
});

test("the secretary's state opens from the indicator at the top, not from a tab or settings", async ({ page }) => {
  await openStatus(page);
  await expect(page.getByRole("dialog", { name: "秘書の状態" })).toBeVisible();
  await expect(page.getByRole("link", { name: "状態", exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "秘書の状態" })).toBeHidden();
});

test("the status page says what the secretary is doing as a badge, and what waits for the owner", async ({ page }) => {
  await openStatus(page);
  const sheet = page.getByRole("dialog", { name: "秘書の状態" });
  await expect(sheet.getByRole("status")).toHaveText(/^(仕事をしている|待っている|止まっている)$/);
  await expect(sheet.getByText("待っている入力")).toBeVisible();
  await expect(sheet.getByRole("heading", { name: "あなたを待っているもの" })).toBeVisible();
});

test("the status page lists the latest inputs as cards with where each came from and how far it got", async ({
  page,
  request,
}) => {
  const text = "状態に出る入力。狭いシートでも本文が一文字ずつ縦に折れずに、横へ読める幅で並ぶ";
  await call(request, "ConsoleService", "Send", { text });
  await openStatus(page);
  const recent = page.locator("section").filter({ has: page.getByRole("heading", { name: "最近の入力" }) });
  const card = recent.getByRole("listitem").filter({ hasText: text });
  await expect(card).toBeVisible();
  await expect(card.getByText("コンソール")).toBeVisible();
  await expect(card.locator(".tag")).toHaveText(/^(待ち|仕事中|済|止まった|失敗)$/);
  const body = await card.getByText(text).boundingBox();
  const width = (await card.boundingBox())?.width ?? 0;
  expect(body?.width ?? 0).toBeGreaterThan(width * 0.8);
});

test("the latest inputs download as JSON Lines, one input a line", async ({ page, request }) => {
  await call(request, "ConsoleService", "Send", { text: "書き出しに出る入力" });
  await openStatus(page);
  const recent = page.locator("section").filter({ has: page.getByRole("heading", { name: "最近の入力" }) });
  await expect(recent.getByText("書き出しに出る入力")).toBeVisible();
  const [download] = await Promise.all([
    page.waitForEvent("download"),
    recent.getByRole("button", { name: "JSONL で書き出す" }).click(),
  ]);
  expect(download.suggestedFilename()).toMatch(/\.jsonl$/);
  const path = await download.path();
  const lines = (await import("node:fs")).readFileSync(path, "utf8").trimEnd().split("\n");
  const inputs = lines.map((line) => JSON.parse(line) as { text: string; source: string; status: string });
  const sent = inputs.find((i) => i.text === "書き出しに出る入力");
  expect(sent?.source).toBe("console");
  expect(sent?.status).toMatch(/^(pending|running|done|held|failed)$/);
});

test("a pending wake-up shows among what is coming with why it was set, and is gone once cancelled", async ({
  page,
  request,
}) => {
  await call(request, "RoutineService", "WakeAt", {
    inMinutes: 90,
    note: "状態に出る起床予約",
    why: "見積書の返事を待っている",
  });
  await openStatus(page);
  const coming = page.locator("section").filter({ has: page.getByRole("heading", { name: "これからの予定" }) });
  await expect(coming.getByText("起床予約 · 状態に出る起床予約")).toBeVisible();
  await expect(coming.getByText("見積書の返事を待っている")).toBeVisible();

  await coming.getByRole("button", { name: "起床予約「状態に出る起床予約」を取り消す" }).click();
  await expect(toast(page)).toHaveText("起床予約を取り消した");
  await expect(coming.getByText("起床予約 · 状態に出る起床予約")).toHaveCount(0);
  await page.reload();
  await expect(coming.getByText("起床予約 · 状態に出る起床予約")).toHaveCount(0);
});

test("the log shows a turn the owner asked for, and the filters narrow it", async ({ page }) => {
  await visit(page, "/", /^ホーム/);
  await page.getByPlaceholder(/に頼む$/).fill("ログに残る頼みごと");
  await page.getByRole("button", { name: "送る" }).click();
  await expect(page.getByTestId("message").filter({ hasText: "ログに残る頼みごと" })).toBeVisible();

  await page.getByRole("navigation", { name: "セクション" }).first().getByRole("link", { name: "設定" }).click();
  await expectView(page, "設定 · 秘書");
  await page.getByRole("button", { name: "ログを見る" }).click();
  const sheet = page.getByRole("dialog", { name: "秘書のログ" });
  const turn = sheet.getByRole("button").filter({ hasText: "ログに残る頼みごと" });
  await expect(turn).toBeVisible();

  await sheet.getByRole("button", { name: "夢・見張り・設定" }).click();
  await expect(sheet.getByRole("button", { name: "夢・見張り・設定" })).toHaveAttribute("aria-pressed", "true");
  await expect(turn).toHaveCount(0);
  await sheet.getByRole("button", { name: "秘書のやりとり" }).click();
  await expect(turn).toBeVisible();
  await turn.click();
  await expect(turn).toHaveAttribute("aria-expanded", "true");
});

const LOGS: [string, string, string, string, string][] = [
  ["/settings", "設定 · 秘書", "秘書のログ", "ログを見る", "すべて"],
  ["/settings/roles/dream", "設定 · 夢", "夢のログ", "ログを見る", "夜ごとに夢を見たか"],
  ["/settings/lookout", "設定 · 見張り", "見張りのログ", "ログを見る", "見張りが読んだものと"],
  ["/settings/roles/triage", "設定 · トリアージ", "トリアージのログ", "ログを見る", "注視しているチャンネルの発言ごとに"],
  ["/settings/vault", "設定 · 金庫", "ゲートウェイの控え", "控えを見る", "ゲートウェイが通したもの"],
];

test("each agent's page opens its own log in a sheet from the right, and the list of pages has no log page", async ({
  page,
}) => {
  for (const [path, view, title, button, inside] of LOGS) {
    const sheet = await openLog(page, path, view, title, button);
    await expect(sheet.getByText(inside, { exact: false }).first()).toBeVisible();
    await sheet.getByRole("button", { name: "閉じる" }).click();
    await expect(sheet).toHaveCount(0);
  }
  await expect(page.getByRole("navigation", { name: "設定" }).getByRole("link", { name: "ログ" })).toHaveCount(0);
});

async function useSomething(request: APIRequestContext) {
  await call(request, "ConsoleService", "Send", { text: "使った量に出る頼みごと" });
  await expect
    .poll(async () => Number((await call<{ callsToday?: string }>(request, "ModelService", "GetUsage", {})).callsToday ?? 0))
    .toBeGreaterThan(0);
}

test("the usage page opens on this month by agent, and 次へ is off on it", async ({ page }) => {
  await visit(page, "/settings/usage", "設定 · 使った量");
  await expect(page.getByRole("heading", { name: "使った量", level: 1 })).toBeVisible();
  const [y, m] = dayFromToday(0).split("-").map(Number);
  await expect(page.getByText(`${y}年${m}月`, { exact: true })).toBeVisible();
  for (const label of ["呼び出し", "トークン（入力＋出力）", "従量課金"]) {
    await expect(page.getByText(label, { exact: true }).first()).toBeVisible();
  }
  await expect(page.getByRole("radio", { name: "エージェントごと" })).toBeChecked();
  await expect(page.getByRole("button", { name: "次へ" })).toBeDisabled();
});

test("the usage page moves a period back and comes forward to this one", async ({ page }) => {
  const [y, m, d] = dayFromToday(0).split("-").map(Number);
  const before = new Date(Date.UTC(y, m - 2, 1));
  const monday = new Date(Date.UTC(y, m - 1, d - ((new Date(Date.UTC(y, m - 1, d)).getUTCDay() + 6) % 7)));
  const week = (from: Date) => {
    const to = new Date(from.getTime() + 6 * 86400_000);
    return `${from.getUTCFullYear()}年${from.getUTCMonth() + 1}/${from.getUTCDate()}〜${to.getUTCMonth() + 1}/${to.getUTCDate()}`;
  };
  for (const [span, now, back] of [
    ["月", `${y}年${m}月`, `${before.getUTCFullYear()}年${before.getUTCMonth() + 1}月`],
    ["週", week(monday), week(new Date(monday.getTime() - 7 * 86400_000))],
    ["年", `${y}年`, `${y - 1}年`],
  ]) {
    await visit(page, "/settings/usage", "設定 · 使った量");
    await page.getByText(span, { exact: true }).click();
    await expect(page.getByText(now, { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "前へ" }).click();
    await expect(page.getByText(back, { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "次へ" }).click();
    await expect(page.getByText(now, { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "次へ" })).toBeDisabled();
  }
});

test("a day of the usage lists its calls, tokens and yen by agent, then by model", async ({ page, request }) => {
  await useSomething(request);
  await visit(page, "/settings/usage", "設定 · 使った量");
  const [, m, d] = dayFromToday(0).split("-").map(Number);
  const day = `${m}/${d}`;
  const chart = page.getByRole("region", { name: "値段のないモデルの回数" });
  const bar = chart.getByRole("button", { name: new RegExp(`^${day} · \\d+ 回$`) });

  await bar.click();
  const detail = page.getByRole("table", { name: `${day} の使った量` });
  await expect(detail.getByRole("columnheader")).toHaveText(["エージェント", "回数", "入力", "出力", "金額"]);
  await expect(detail.getByRole("row").filter({ hasText: "秘書" }).getByRole("cell")).toHaveText([
    "秘書",
    /^[1-9]\d* 回$/,
    /^[1-9]\d*k?$/,
    /^[1-9]\d*k?$/,
    /^\d[\d,]* 円$/,
  ]);
  await expect(chart.getByRole("listitem").filter({ hasText: "秘書" })).toBeVisible();

  await page.getByText("モデルごと", { exact: true }).click();
  await expect(detail.getByRole("columnheader").first()).toHaveText("モデル");
  await expect(chart.getByRole("listitem").filter({ hasText: "秘書" })).toHaveCount(0);
  await expect(page.getByRole("table", { name: "期間の合計" }).getByRole("cell", { name: "秘書" }).first()).toBeVisible();
});

test("hovering a bar of the usage tells what it is made of", async ({ page, request }) => {
  await useSomething(request);
  await visit(page, "/settings/usage?span=year", "設定 · 使った量");
  const month = `${Number(dayFromToday(0).split("-")[1])}月`;
  const chart = page.getByRole("region", { name: "値段のないモデルの回数" });
  await expect(chart.getByRole("button")).toHaveCount(12);
  await chart.getByRole("button", { name: new RegExp(`^${month} · \\d+ 回$`) }).hover();
  await expect(page.getByRole("tooltip")).toContainText("秘書");
});

for (const [name, open] of [
  ["the secretary's state", (page: Page) => openStatus(page).then(() => page.getByRole("dialog", { name: "秘書の状態" }))],
  ["an agent's log", (page: Page) => openLog(page, "/settings/roles/triage", "設定 · トリアージ", "トリアージのログ")],
] as const) {
  test(`${name} sheet widens by dragging its edge, keeps the width when opened again, and goes back with one button`, async ({
    page,
  }) => {
    let sheet = await open(page);
    const width = async () => (await sheet.boundingBox())?.width ?? 0;
    const before = await width();
    expect(before).toBe(720);
    await expect(sheet.getByRole("button", { name: "元の幅に戻す" })).toBeDisabled();

    const edge = (await sheet.getByRole("separator", { name: "幅を変える" }).boundingBox())!;
    await page.mouse.move(edge.x + edge.width / 2, edge.y + edge.height / 2);
    await page.mouse.down();
    await page.mouse.move(edge.x - 150, edge.y + edge.height / 2, { steps: 5 });
    await page.mouse.up();
    await expect.poll(width).toBeGreaterThan(before + 100);
    const widened = await width();

    await sheet.getByRole("button", { name: "閉じる" }).click();
    sheet = await open(page);
    await expect.poll(width).toBe(widened);

    await sheet.getByRole("button", { name: "元の幅に戻す" }).click();
    await expect.poll(width).toBe(before);
    await expect(sheet.getByRole("button", { name: "元の幅に戻す" })).toBeDisabled();
  });
}

test("the secretary's page shows where the notebook strays, and fixes only when asked", async ({ page, request }) => {
  const t = await call<{ id: string }>(request, "RecordService", "SaveRecord", {
    kind: "tasks",
    title: "書き方の古いタスク",
    fields: [{ name: "Due", value: "2027-01-05T09:00" }],
  });
  await visit(page, "/settings", "設定 · 秘書");
  const check = page.locator("section").filter({ has: page.getByRole("heading", { name: "手帳の書き方" }) });
  await expect(check.getByText("Due is written YYYY-MM-DD HH:MM")).toBeVisible();
  await check.getByRole("button", { name: "決まりどおりに直す" }).click();
  await expect(check.getByText("Due is written YYYY-MM-DD HH:MM")).toHaveCount(0);
  const now = await call<{ markdown: string }>(request, "RecordService", "GetRecord", { kind: "tasks", id: t.id });
  expect(now.markdown).toContain("- Due: 2027-01-05 09:00");
});
