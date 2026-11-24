import { readFileSync, utimesSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { expect, test } from "@playwright/test";
import { call, detail, record, row, toast, visit } from "./support";

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
  await expect(row.locator(".item-title")).toHaveText("e2e: 朝の連絡はメールがいい");
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
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await expect(toast(page)).toHaveText("保存した");
  await page.reload();
  await expect(editor).toHaveValue("# 心得\n\n- 返事は短く、結論から");

  await page.getByRole("link", { name: "読む", exact: true }).click();
  await expect(page.locator("#pane article.markdown-body").getByRole("listitem")).toHaveText("返事は短く、結論から");

  await page.getByRole("link", { name: "経緯", exact: true }).click();
  await expect(page.locator(".list .item").first()).toContainText("あなた");

  await visit(page, "/memory/core", "記憶 · 心得");
  await expect(page.getByText("返事は短く、結論から")).toBeVisible();
});

test("what is being written stays when something else changes meanwhile", async ({ page, request }) => {
  await visit(page, "/memory/file?path=core.md&mode=write", "記憶 · core.md");
  const editor = page.getByLabel("core.md を書く");
  await editor.fill("# 心得\n\n- 書きかけ");
  const loaded = page.waitForResponse((r) => r.url().includes("MemoryService/ReadFile"));
  await record(request, "memos", "ほかで足した");
  await loaded;
  await expect(editor).toHaveValue("# 心得\n\n- 書きかけ");
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await expect(toast(page)).toHaveText("保存した");
});

test("what is being written over a memory file is kept, and said, when the file is written elsewhere", async ({ page, request }) => {
  const path = "long/semantic/e2e-two-writers.md";
  const write = (text: string) =>
    call(request, "MemoryService", "WriteFile", { path, text: `---\nid: e2e-w1\nsummary: 二人で書く\nsrc: said\n---\n\n${text}\n` });
  await write("はじめ");
  await visit(page, `/memory/file?path=${path}&mode=write`, /^記憶/);
  const editor = page.getByLabel(`${path} を書く`);
  const again = page.getByRole("button", { name: "書きかけを捨てて読み直す" });

  await write("ほかで直した");
  await expect(editor).toHaveValue(/ほかで直した/);

  await editor.fill("書きかけ");
  await write("もう一度直した");
  await expect(again).toBeVisible();
  await expect(editor).toHaveValue("書きかけ");
  await again.click();
  await expect(editor).toHaveValue(/もう一度直した/);
  await expect(page.getByRole("button", { name: "保存", exact: true })).toBeDisabled();
});

test.describe("memory drawn as Markdown", () => {
  test("a memory file is drawn with the attributes of its items set apart, and written as plain text", async ({
    page,
    request,
  }) => {
    const said = '- 返事は **短く** 、結論から {id: e2e-md1, src: said, workgroup: [], reason: "a {b} **c**"}';
    const others =
      '- 引用符 {"id": e2e-md11, src: said}\n- 角括弧 {id: e2e-md12, workgroup: ["team[1]"]}\n- YAML でない {id: e2e-md13, reason: a: b}';
    const text = `# 心得\n\n${said}\n- 波括弧 {x} はそのまま\n${others}\n\n\`\`\`\n- 例 {id: e2e-md2, src: said}\n\`\`\`\n`;
    await call(request, "MemoryService", "WriteFile", { path: "core.md", text });
    await visit(page, "/memory/file?path=core.md", "記憶 · core.md");

    const body = page.locator("#pane article.markdown-body");
    await expect(body.getByRole("heading", { name: "心得" })).toBeVisible();
    const item = body.getByRole("listitem").first();
    await expect(item.locator("strong")).toHaveText("短く");
    await expect(item.locator(".attrs")).toHaveText('id: e2e-md1, src: said, workgroup: [], reason: "a {b} **c**"');
    await expect(body.getByRole("listitem").nth(1)).toHaveText("波括弧 {x} はそのまま");
    await expect(body.getByRole("listitem").nth(2).locator(".attrs")).toHaveText('"id": e2e-md11, src: said');
    await expect(body.getByRole("listitem").nth(3).locator(".attrs")).toHaveText('id: e2e-md12, workgroup: ["team[1]"]');
    await expect(body.getByRole("listitem").nth(4)).toHaveText("YAML でない {id: e2e-md13, reason: a: b}");
    await expect(body.locator("pre")).toContainText("- 例 {id: e2e-md2, src: said}");

    await page.getByRole("link", { name: "書く", exact: true }).click();
    await expect(page.getByLabel("core.md を書く")).toHaveValue(text);
  });

  test("a long-term memory file keeps its frontmatter folded apart from the memory", async ({ page, request }) => {
    await call(request, "MemoryService", "WriteFile", {
      path: "long/semantic/e2e-folded.md",
      text: "---\nid: e2e-md3\nsummary: 朝は \`お茶\`\nsrc: said\n---\n\n朝は *お茶* 。\n",
    });
    await visit(page, "/memory/file?path=long/semantic/e2e-folded.md", /^記憶/);
    const body = page.locator("#pane article.markdown-body");
    await expect(body.locator("em")).toHaveText("お茶");
    await expect(body.getByText("e2e-md3")).toBeHidden();
    await body.getByText("属性", { exact: true }).click();
    await expect(body.getByText("e2e-md3")).toBeVisible();

    await visit(page, "/memory/long", "記憶 · 長期記憶");
    const card = page.locator("section.pref").filter({ hasText: "朝は お茶" }).first();
    await expect(card.locator("h2 code")).toHaveText("お茶");
    await expect(card.locator(".item code")).toHaveText("お茶");
  });

  test("an item and a file open the record and the memory they link to, and the item is not picked", async ({
    page,
    request,
  }) => {
    const party = await record(request, "parties", "田中さん");
    await call(request, "MemoryService", "WriteFile", {
      path: "long/semantic/e2e-linked.md",
      text: `---\nid: e2e-md4\nsummary: 甘いもの\nsrc: said\n---\n\n[[records/parties/${party.id}|田中さん]] は甘いものが好き。\n`,
    });
    await call(request, "MemoryService", "WriteFile", {
      path: "core.md",
      text: `# 心得\n\n- [[records/parties/${party.id}|田中さん]] には [[memory/long/semantic/e2e-linked|好み]] を見て選ぶ {id: e2e-md10, src: said}\n`,
    });
    await visit(page, "/memory/core", "記憶 · 心得");
    const item = page.locator(".item").filter({ hasText: "を見て選ぶ" });
    await item.locator(".md-link", { hasText: "好み" }).click();
    await expect(page).toHaveURL(/\/memory\/file\?path=long%2Fsemantic%2Fe2e-linked\.md$/);
    await expect(page.locator("#composer")).not.toContainText("を見て選ぶ");
    await page.locator("article.markdown-body .md-link", { hasText: "田中さん" }).click();
    await expect(page).toHaveURL(new RegExp(`/parties\\?id=${party.id}`));

    await visit(page, "/memory/core", "記憶 · 心得");
    await item.locator(".md-link", { hasText: "田中さん" }).click();
    await expect(page).toHaveURL(new RegExp(`/parties\\?id=${party.id}`));
  });

  test("a wiki link names its file as it is, and a Markdown link as a URL", async ({ page, request }) => {
    const target = "---\nsummary: 割合\n---\n";
    for (const path of ["long/semantic/e2e 100%.md", "long/semantic/e2e%20link.md", "long/semantic/e2e link.md"])
      await call(request, "MemoryService", "WriteFile", { path, text: target });
    await call(request, "MemoryService", "WriteFile", {
      path: "core.md",
      text: "# 心得\n\n- [[memory/long/semantic/e2e 100%|割合]]\n- [[memory/long/semantic/e2e%20link|綴り]]\n- [名前](memory/long/semantic/e2e%20link.md)\n",
    });
    for (const at of ["/memory/core", "/memory/file?path=core.md"])
      for (const [name, path] of [
        ["割合", "long/semantic/e2e 100%.md"],
        ["綴り", "long/semantic/e2e%20link.md"],
        ["名前", "long/semantic/e2e link.md"],
      ]) {
        await visit(page, at, /^記憶 · /);
        await page.locator(".md-link", { hasText: name }).click();
        await expect(page).toHaveURL((url) => url.searchParams.get("path") === path);
      }
  });

  test("a one-line item sits on one line and runs nothing", async ({ page, request }) => {
    // Short enough for one line whatever the fonts.
    const line = "- **太字** と <script>x()</script> <img src=x onerror=x()>";
    await call(request, "MemoryService", "WriteFile", {
      path: "core.md",
      text: `# 心得\n\n- ふつうの行 {id: e2e-md5, src: said}\n${line} {id: e2e-md6, src: said}\n`,
    });
    await visit(page, "/memory/core", "記憶 · 心得");
    const plain = page.locator(".item").filter({ hasText: "ふつうの行" });
    const marked = page.locator(".item").filter({ hasText: "太字" });
    await expect(marked.locator("strong")).toHaveText("太字");
    await expect(marked.locator("script, img")).toHaveCount(0);
    await expect(marked).toContainText("<script>");
    await expect(marked).not.toContainText("e2e-md6");
    await expect(marked.locator("p")).toHaveCount(0);
    const heights = await Promise.all([plain, marked].map(async (l) => (await l.boundingBox())?.height));
    expect(heights[0]).toBe(heights[1]);
  });

  test("pending, episodic, working and forgotten memories are drawn too", async ({ page, request }) => {
    const write = (path: string, text: string) => call(request, "MemoryService", "WriteFile", { path, text });
    await write("pending.md", "- 朝が **好き** らしい {id: e2e-md7, src: inferred, ev: 1}\n");
    await write("episodic/2026-01-02.md", "- 09:15 朝に **話した** {id: e2e-md8, src: said, state: waiting}\n");
    await write(
      "forgotten/2026-01.md",
      "- 昔の **癖** {id: e2e-md9, src: said, forgot: 2026-01-03, reason: \"`新しい` もの\"}\n",
    );
    await write("working/e2e-drawn.md", "# 宿の `下調べ`\n\n## ここまで\n\n- **三つ** 比べた\n- 一つ消した\n");

    await visit(page, "/memory/pending", "記憶 · 保留中の判断");
    await expect(page.locator(".item strong")).toHaveText("好き");
    await visit(page, "/memory/episodic?at=2026-01-02", "記憶 · エピソード記憶");
    await expect(page.locator(".item strong")).toHaveText("話した");
    await visit(page, "/memory/forgotten?at=2026-01", "記憶 · 忘却");
    await expect(page.locator(".item strong")).toHaveText("癖");
    await expect(page.locator(".item .item-sub code")).toHaveText("新しい");
    await visit(page, "/memory/working", "記憶 · 作業記憶");
    const card = page.locator("article.card").filter({ has: page.getByRole("heading", { name: "宿の 下調べ" }) });
    await expect(card.locator("h2 code")).toHaveText("下調べ");
    await expect(card.getByRole("listitem")).toHaveText(["三つ 比べた", "一つ消した"]);

    await write("pending.md", "");
    await write("core.md", "# 心得\n");
  });
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
      ["e2e-call", "e2e-done1", "来週 **電話** する"],
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
    const change = page.locator(".item").filter({ hasText: "来週 電話 する" });
    await expect(change.locator("strong")).toHaveText("電話");
    await expect(change.locator(".tag")).toHaveText("忘れた");
    await expect(change.locator(".sub")).toHaveText(/^long\/prospective\/e2e-call\.md → forgotten\/.+\.md · e2eで済んだ$/);

    await call(request, "ModelService", "AssignRole", { role: "ROLE_DREAM", model: "" });
    await call(request, "ModelService", "UnregisterModel", { id: model.id });
  });

  test("a night makes a task of a memo and asks about a doubtful record, which the owner clears", async ({ page, request }) => {
    const { path: notebook } = await call<{ path: string }>(request, "SettingsService", "GetNotebookPlace");
    const todo = await record(request, "memos", "e2e-todo 牛乳を買う", { notes: "低脂肪のもの" });
    const stale = await record(request, "memos", "e2e-stale 店の営業時間", { notes: "20 時まで" });
    for (const { id } of [todo, stale]) {
      const file = join(notebook, "records", "memos", `${id}.md`);
      const old = readFileSync(file, "utf8").replace(/^(created|updated): .*$/gm, "$1: 2020-01-01T00:00:00+09:00");
      writeFileSync(file, old);
      utimesSync(file, new Date("2020-01-01"), new Date("2020-01-01"));
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
      .poll(
        async () => {
          const now = await call<Dreams & { dreaming?: boolean }>(request, "MemoryService", "ListDreams");
          return !now.dreaming && (now.dreams?.length ?? 0) > before;
        },
        { timeout: 30_000 },
      )
      .toBe(true);

    await visit(page, "/memory", "記憶 · 夢");
    const made = page.locator(".item").filter({ hasText: "e2e-todo 牛乳を買う" });
    await expect(made.locator(".tag")).toHaveText("タスクにした");
    await expect(made.locator(".sub")).toContainText("e2e-why");
    const doubted = page.locator(".item").filter({ hasText: "e2e-stale 店の営業時間" });
    await expect(doubted.locator(".tag")).toHaveText("古いかも");

    await visit(page, "/memos", "メモ");
    await expect(row(page, "e2e-todo 牛乳を買う")).toHaveCount(0);
    await page.getByLabel("タスクに移したメモも").check();
    await row(page, "e2e-todo 牛乳を買う").click();
    await detail(page).locator(".callout").getByRole("link", { name: "e2e-todo 牛乳を買う" }).click();
    await expect(page).toHaveURL(/\/tasks\?id=/);
    await expect(detail(page).getByText("低脂肪のもの")).toBeVisible();

    await visit(page, "/memos", "メモ");
    await page.getByLabel("古いかもしれないものだけ").check();
    await expect(page.locator("[data-record]")).toHaveCount(1);
    await expect(row(page, "e2e-stale 店の営業時間").locator(".tag.warn")).toHaveText("古いかも");
    await row(page, "e2e-stale 店の営業時間").click();
    await expect(detail(page).getByText("20 時まで")).toBeVisible();
    await expect(detail(page).locator(".callout.warn")).toContainText("e2e-why");
    await detail(page).getByRole("button", { name: "確かめたので印を外す" }).click();
    await expect(toast(page)).toHaveText("印を外した");
    await expect(detail(page).locator(".callout.warn")).toHaveCount(0);
    type Listed = { records?: { title: string }[] };
    const open = await call<Listed>(request, "RecordService", "ListRecords", { kind: "tasks", openOnly: true });
    expect(open.records?.map((t) => t.title).filter((t) => t.includes("e2e-stale"))).toEqual([]);

    await call(request, "ModelService", "AssignRole", { role: "ROLE_DREAM", model: "" });
    await call(request, "ModelService", "UnregisterModel", { id: model.id });
  });
});
