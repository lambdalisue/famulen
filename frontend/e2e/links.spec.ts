import { expect, type Locator, type Page, test } from "@playwright/test";
import { agree, call, detail, record, toast, visit } from "./support";

const back = (page: Parameters<typeof detail>[0], text: string) => detail(page).locator(".rec-links").filter({ hasText: text });

test("typing [[ in a memo picks a record with the keyboard, and the record shows the memo", async ({ page, request }) => {
  const party = await record(request, "parties", "リンク先の田中さん", { fields: { Kind: "person" } });
  const memo = await record(request, "memos", "手土産の覚え");
  await visit(page, `/memos?id=${memo.id}`, "メモ · 手土産の覚え");
  await detail(page).getByRole("button", { name: "本文を書く" }).click();
  const body = detail(page).getByLabel("本文", { exact: true });
  await body.pressSequentially("和菓子は[[");
  const picker = page.getByRole("dialog", { name: "記録へのリンク" });
  await picker.getByLabel("探す").fill("リンク先の");
  await expect(picker.getByRole("button", { name: /リンク先の田中さん/ })).toHaveAttribute("aria-current", "true");
  await picker.getByLabel("探す").press("Enter");
  await expect(body).toBeFocused();
  await body.pressSequentially("に");
  await expect(body).toHaveValue(`和菓子は[[records/parties/${party.id}|リンク先の田中さん]]に`);
  await body.press("Control+Enter");
  await expect(toast(page)).toHaveText("保存した");
  await detail(page).locator(".md-link", { hasText: "リンク先の田中さん" }).click();
  await expect(page).toHaveURL(new RegExp(`/parties\\?id=${party.id}`));
  await expect(back(page, "触れているメモ")).toContainText("手土産の覚え");
});

test("related records of any kind are added and removed, and show on both", async ({ page, request }) => {
  const memo = await record(request, "memos", "関わるメモ");
  const party = await record(request, "parties", "関わる相手さん", { fields: { Kind: "person" } });
  await visit(page, `/parties?id=${party.id}`, "相手 · 関わる相手さん");
  await detail(page).getByRole("button", { name: "関わるものを足す" }).click();
  await page.getByRole("dialog").getByLabel("探す").fill("関わるメモ");
  await page.getByRole("dialog").getByRole("button", { name: /関わるメモ/ }).click();
  await expect(toast(page)).toHaveText("保存した");
  await detail(page).getByRole("link", { name: "関わるメモ" }).click();
  await expect(page).toHaveURL(new RegExp(`/memos\\?id=${memo.id}`));
  await expect(back(page, "関わる相手")).toContainText("関わる相手さん");
  await page.goto(`/parties?id=${party.id}`);
  await detail(page).getByRole("button", { name: "関わるメモを関わるものから外す" }).click();
  await expect(detail(page).getByRole("link", { name: "関わるメモ" })).toHaveCount(0);
});

test("a link follows a rename, and one whose record is gone says so", async ({ page, request }) => {
  const a = await record(request, "memos", "前の名前");
  const b = await record(request, "tasks", "消える仕事");
  const memo = await record(request, "memos", "つなぐメモ", {
    notes: `[[${a.code}]] と [[${b.code}]]`,
    fields: { Related: b.code },
  });
  await call(request, "RecordService", "SaveRecord", { kind: "memos", id: a.id, title: "今の名前" });
  await call(request, "RecordService", "DeleteRecord", { kind: "tasks", id: b.id });
  await visit(page, `/memos?id=${memo.id}`, "メモ · つなぐメモ");
  await expect(detail(page).locator(".md-link", { hasText: "今の名前" })).toBeVisible();
  await expect(detail(page).locator(".md-link-missing", { hasText: "消える仕事" })).toBeVisible();
  await expect(detail(page).locator(".tag.missing")).toContainText("消える仕事（もうない）");
});

test("a link shown as code in a line of text leads nowhere", async ({ page, request }) => {
  const memo = await record(request, "memos", "見本の先");
  const link = `[[records/memos/${memo.id}|見本の先]]`;
  const party = await record(request, "parties", "見本の相手さん", { fields: { Kind: "person" } });
  await call(request, "RecordService", "SaveRecord", {
    kind: "parties",
    id: party.id,
    log: [`\`${link}\` と ${link}`],
  });
  await visit(page, `/parties?id=${party.id}`, "相手 · 見本の相手さん");
  await expect(detail(page).getByText(`\`${link}\` と`)).toBeVisible();
  await expect(detail(page).getByRole("link", { name: "見本の先" })).toHaveCount(1);
});

test("a memory that links to a record shows on it, and leads back", async ({ page, request }) => {
  const memo = await record(request, "memos", "記憶が触れるメモ");
  await call(request, "MemoryService", "WriteFile", {
    path: "long/semantic/e2e-links.md",
    text: `---\nid: e2e-l1\nsummary: メモの覚え\nsrc: said\n---\n\n[[${memo.code}]] を見る。\n`,
  });
  await visit(page, `/memos?id=${memo.id}`, "メモ · 記憶が触れるメモ");
  await back(page, "触れている記憶").getByRole("link", { name: "メモの覚え" }).click();
  await expect(page).toHaveURL(/\/memory\/file/);
  await page.locator("#pane .rec-prose").getByRole("link", { name: "記憶が触れるメモ" }).click();
  await expect(page).toHaveURL(new RegExp(`id=${memo.id}`));
});

test("what links to a record stays as it is on screen when the record is read again", async ({ page, request }) => {
  const party = await record(request, "parties", "読み直す相手さん", { fields: { Kind: "person" } });
  await record(request, "memos", "前からのメモ", { notes: `[[${party.code}]]` });
  await visit(page, `/parties?id=${party.id}`, /^相手/);
  const links = back(page, "触れているメモ");
  const row = await links.getByRole("listitem").filter({ hasText: "前からのメモ" }).elementHandle();
  await record(request, "memos", "後からのメモ", { notes: `[[${party.code}]]` });
  await expect(links).toContainText("後からのメモ");
  expect(await row?.evaluate((e) => e.isConnected)).toBe(true);
});

const SENTENCE = "キャサリン（Katherine Montgomery）（Slack・アットマーク、U09ABCDEFGH）：地区の区分（関東・関西・東海・福岡）の定義を確認している。";

const roundness = (of: Locator) => of.evaluate((el) => Number.parseFloat(getComputedStyle(el).borderTopLeftRadius));

/** The links of one kind are in square rows below their name, as wide as the section they are in. */
async function expectRows(page: Page, name: string) {
  const links = back(page, name);
  const row = links.getByRole("listitem");
  await expect(row).toHaveText(new RegExp(SENTENCE.slice(0, 20)));
  const [section, label, rowBox] = [await links.boundingBox(), await links.getByText(name, { exact: true }).boundingBox(), await row.boundingBox()];
  if (!section || !label || !rowBox) throw new Error(`no ${name}`);
  expect(rowBox.y).toBeGreaterThanOrEqual(label.y + label.height);
  expect(rowBox.width).toBeGreaterThanOrEqual(section.width - 4);
  expect(await links.getByRole("list").evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
  // What opens it is the whole row, and no part of it is a chip.
  await expect(links.locator(".tag")).toHaveCount(0);
  expect(await roundness(links.getByRole("list"))).toBeLessThanOrEqual(20);
}

for (const width of [390, 1280]) {
  test(`at ${width}, long memories and titles that link here are read in rows of the full width`, async ({ page, request }) => {
    // High enough for the whole record to be in the pictures.
    await page.setViewportSize({ width, height: 1400 });
    const party = await record(request, "parties", `長い文でつながる相手 ${width}`, { fields: { Kind: "person" } });
    const other = await record(request, "memos", `${SENTENCE.repeat(2)}（関わる ${width}）`);
    await record(request, "memos", `${SENTENCE}（触れる ${width}）`, { notes: `[[${party.code}]]` });
    await call(request, "RecordService", "SaveRecord", {
      kind: "parties",
      id: party.id,
      fields: [
        { name: "Kind", value: "person" },
        { name: "Related", value: `[[${other.code}]]` },
      ],
    });
    await call(request, "MemoryService", "WriteFile", {
      path: "long/semantic/e2e-long-link.md",
      text: `---\nid: e2e-long-link\nsummary: ${SENTENCE}\nsrc: said\n---\n\n[[${other.code}]] を見る。\n`,
    });

    await visit(page, `/parties?id=${party.id}`, /^相手/);
    await expectRows(page, "触れているメモ");
    // A chip keeps to its column with the button that takes it off, and is no ellipse.
    const chip = detail(page).locator(".tag.removable").filter({ hasText: `関わる ${width}` });
    const [tag, off] = [await chip.boundingBox(), await chip.getByRole("button").boundingBox()];
    if (!tag || !off) throw new Error("no chip");
    expect(tag.height).toBeGreaterThan(40);
    expect(off.x + off.width).toBeLessThanOrEqual(tag.x + tag.width);
    expect(await roundness(chip)).toBeLessThanOrEqual(20);

    await detail(page).screenshot({ path: test.info().outputPath("related.png") });
    await visit(page, `/memos?id=${other.id}`, /^メモ/);
    await expectRows(page, "触れている記憶");
    await detail(page).screenshot({ path: test.info().outputPath("memories.png") });
  });
}
