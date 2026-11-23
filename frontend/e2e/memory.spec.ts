import { resolve } from "node:path";
import { expect, test } from "@playwright/test";
import { call, toast, visit } from "./support";

type Models = { roles: { role: string; choices: string[] }[] };
type Dreams = { dreams?: { id: string }[] };

const LAYERS: [string, string][] = [
  ["保留中の判断", "保留中の判断"],
  ["エピソード記憶", "エピソード記憶"],
  ["作業記憶", "作業記憶"],
  ["心得", "心得"],
  ["長期記憶", "長期記憶"],
  ["忘却", "忘却"],
  ["夢", "夢"],
];

test("every memory layer opens from the side list with its own heading", async ({ page }) => {
  await visit(page, "/memory", "記憶 · 夢");
  const nav = page.getByRole("navigation", { name: "記憶の層" });
  for (const [link, heading] of LAYERS) {
    await nav.getByRole("link", { name: new RegExp(`^${link}`) }).click();
    await expect(page.getByRole("heading", { name: heading, exact: true, level: 1 })).toBeVisible();
  }
});

test("empty layers say there is nothing yet", async ({ page }) => {
  await visit(page, "/memory/pending", "記憶 · 保留中の判断");
  await expect(page.getByText("いまはない")).toBeVisible();
  await visit(page, "/memory/working", "記憶 · 作業記憶");
  await expect(page.getByText("いま進めている仕事はない")).toBeVisible();
  await visit(page, "/memory/long", "記憶 · 長期記憶");
  for (const group of ["意味記憶", "手続き記憶", "展望記憶"]) {
    await expect(page.getByRole("heading", { name: group })).toBeVisible();
  }
});

test("an unknown layer says so", async ({ page }) => {
  await page.goto("/memory/nowhere");
  await expect(page.getByText("そういう層はない")).toBeVisible();
});

test("a working memory the secretary keeps shows what is done, next and waited for", async ({ page, request }) => {
  await call(request, "MemoryService", "SetWorking", {
    name: "e2e-trip",
    done: "宿を三つ比べた",
    next: "主人に選んでもらう",
    waiting: "返事",
    clues: "booking.example",
  });
  await visit(page, "/memory/working", "記憶 · 作業記憶");
  await expect(page.getByRole("heading", { name: "e2e-trip" })).toBeVisible();
  await expect(page.getByText("宿を三つ比べた")).toBeVisible();
  await expect(page.getByText("主人に選んでもらう")).toBeVisible();
});

test("long-term memory shows each memory, one per file, with where it came from", async ({ page, request }) => {
  await call(request, "MemoryService", "WriteFile", {
    path: "long/semantic/e2e-morning-meetings.md",
    text: "---\nid: e2e-l1\nsummary: 会議は午前に入れる\nsrc: said\n---\n\n会議は午前に入れる。\n",
  });
  await call(request, "MemoryService", "WriteFile", {
    path: "long/semantic/e2e-free-friday.md",
    text: "---\nid: e2e-l2\nsummary: 金曜の午後は空けておく\nsrc: inferred\nev: 2\n---\n\n金曜の午後は空けておく。\n",
  });
  await visit(page, "/memory/long", "記憶 · 長期記憶");

  const prefs = page.locator("section").filter({ has: page.getByRole("heading", { name: "意味記憶" }) });
  const said = prefs.locator(".item").filter({ hasText: "会議は午前に入れる" });
  await expect(said.locator(".tag")).toHaveText("あなたが言った");
  const inferred = prefs.locator(".item").filter({ hasText: "金曜の午後は空けておく" });
  await expect(inferred.locator(".tag")).toHaveText("推し量り");
  await expect(inferred).toContainText("根拠 2");
});

test("an episodic note shows the time it was written apart from its text", async ({ page, request }) => {
  await call(request, "MemoryService", "Remember", { text: "e2e: 朝の連絡はメールがいい", said: true });
  await visit(page, "/memory/episodic", "記憶 · エピソード記憶");

  const row = page.locator(".list .item").filter({ hasText: "e2e: 朝の連絡はメールがいい" });
  await expect(row.locator(".mono")).toHaveText(/^\d{2}:\d{2}$/);
  await expect(row.locator(".grow")).toHaveText("e2e: 朝の連絡はメールがいい");
  await expect(row.locator(".tag")).toHaveText("あなたが言った");
});

test("a working memory shows the title its file was given", async ({ page, request }) => {
  await call(request, "MemoryService", "WriteFile", {
    path: "working/e2e-invoice.md",
    text: "# アクメの請求書\n\n## ここまで\n\n下書きを作った\n",
  });
  await visit(page, "/memory/working", "記憶 · 作業記憶");
  await expect(page.getByRole("heading", { name: "アクメの請求書" })).toBeVisible();
});

test("a memory file written by the owner reads back and shows in its history", async ({ page }) => {
  await visit(page, "/memory/file?path=core.md&mode=write", "記憶 · core.md");
  const editor = page.getByLabel("core.md を書く");
  await editor.fill("# 心得\n\n- 返事は短く、結論から");
  await editor.press("Tab");
  await expect(toast(page)).toHaveText("保存した");
  await page.reload();
  await expect(editor).toHaveValue("# 心得\n\n- 返事は短く、結論から");

  await page.getByRole("link", { name: "読む", exact: true }).click();
  await expect(page.locator("article.markdown").getByRole("listitem")).toHaveText("返事は短く、結論から");

  await page.getByRole("link", { name: "経緯", exact: true }).click();
  await expect(page.locator(".list .item").first()).toContainText("あなた");

  await visit(page, "/memory/core", "記憶 · 心得");
  await expect(page.getByText("返事は短く、結論から")).toBeVisible();
});

test.describe("the dream button", () => {
  test.describe.configure({ mode: "serial" });

  test("is off while no dream model is chosen, and says where to choose one", async ({ page, request }) => {
    await call(request, "ModelService", "AssignRole", { role: "ROLE_DREAM", model: "" });
    await visit(page, "/memory", "記憶 · 夢");

    await expect(page.getByRole("button", { name: "今すぐ夢を見る" })).toBeDisabled();
    const why = page.getByRole("link", { name: "夢のモデルが決まっていない。設定の夢で決める" });
    await expect(why).toHaveAttribute("href", "/settings/roles/dream");
  });

  test("says the dream started, and the night it dreamt shows once it is over", async ({ page, request }) => {
    const models = await call<Models>(request, "ModelService", "GetModels");
    const dream = models.roles.find((r) => r.role === "ROLE_DREAM");
    expect(dream?.choices.length).toBeGreaterThan(0);
    await call(request, "ModelService", "AssignRole", { role: "ROLE_DREAM", model: dream?.choices[0] });
    await visit(page, "/memory", "記憶 · 夢");

    const button = page.getByRole("button", { name: "今すぐ夢を見る" });
    await expect(button).toBeEnabled();
    const nights = async () => (await call<Dreams>(request, "MemoryService", "ListDreams")).dreams?.length ?? 0;
    const before = await nights();
    await button.click();
    await expect(toast(page)).toHaveText("夢を見はじめた");
    await expect.poll(nights, { timeout: 30_000 }).toBeGreaterThan(before);
    await expect(page.getByRole("heading", { name: /^最後の夢 · / })).toBeVisible();
    await expect(page.getByText("まだ夢を見ていない")).toHaveCount(0);
    await expect(button).toBeEnabled();

    await call(request, "ModelService", "AssignRole", { role: "ROLE_DREAM", model: "" });
  });

  test("a night that forgot an item shows its commit, where the item went and why", async ({ page, request }) => {
    for (const [name, id, text] of [
      ["e2e-call", "e2e-done1", "来週電話する"],
      ["e2e-papers", "e2e-keep1", "資料を送る"],
      ["e2e-estimate", "e2e-keep2", "見積を出す"],
    ]) {
      await call(request, "MemoryService", "WriteFile", {
        path: `long/prospective/${name}.md`,
        text: `---\nid: ${id}\nsummary: ${text}\nsrc: inferred\n---\n\n${text}\n`,
      });
    }
    const model = await call<{ id: string }>(request, "ModelService", "RegisterModel", {
      name: "e2e の夢",
      kind: "MODEL_KIND_AGENT_CLI",
      endpoint: resolve("e2e/fake-claude.pl"),
    });
    await call(request, "ModelService", "AssignRole", { role: "ROLE_DREAM", model: model.id });
    const before = (await call<Dreams>(request, "MemoryService", "ListDreams")).dreams?.length ?? 0;
    await call(request, "MemoryService", "DreamNow");
    await expect
      .poll(async () => (await call<Dreams>(request, "MemoryService", "ListDreams")).dreams?.length ?? 0, {
        timeout: 30_000,
      })
      .toBeGreaterThan(before);

    await visit(page, "/memory", "記憶 · 夢");
    await expect(page.getByText(/^commit [0-9a-f]{7}$/)).toBeVisible();
    const change = page.locator(".item").filter({ hasText: "来週電話する" });
    await expect(change.locator(".tag")).toHaveText("忘れた");
    await expect(change.locator(".sub")).toHaveText(/^long\/prospective\/e2e-call\.md → forgotten\/.+\.md · e2eで済んだ$/);

    await call(request, "ModelService", "AssignRole", { role: "ROLE_DREAM", model: "" });
    await call(request, "ModelService", "UnregisterModel", { id: model.id });
  });
});
