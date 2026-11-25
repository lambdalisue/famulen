import { type APIRequestContext, expect, test } from "@playwright/test";
import { agree, call, dayFromToday, expectView, record, toast, visit } from "./support";

type Detail = { tools: { name: string }[] };
type Connected = { connection: { id: string } };

/** A connection whose server never starts: enough to list, edit and lookout through. */
async function addStandIn(request: APIRequestContext, name: string, kind: string): Promise<string> {
  const r = await call<Connected>(request, "ConnectionService", "AddConnection", {
    target: "famulen-e2e-no-such-server",
  });
  const id = r.connection.id;
  await call(request, "ConnectionService", "EditConnection", { id, name, kind });
  return id;
}

test.describe("チャネル", () => {
  test("the browser is always usable, and the page sends the owner to the services to add more", async ({ page }) => {
    await visit(page, "/settings/channels", "設定 · チャネル");
    const browser = page.locator(".item").filter({ hasText: "このブラウザ" });
    await expect(browser.getByText("使える")).toBeVisible();
    await expect(browser).toContainText("http://127.0.0.1:13000");

    await expect(page.getByRole("button", { name: /ワークスペースを足す/ })).toHaveCount(0);
    await expect(page.locator(".detail").getByRole("link", { name: "サービス" }).first()).toHaveAttribute(
      "href",
      "/settings/services",
    );
  });

  test("a service whose receiver is for talking is listed once added, and turned off and on there and on its page", async ({
    page,
    request,
  }) => {
    const r = await call<Connected>(request, "ConnectionService", "AddConnection", {
      target: "e2e-glasses",
      how: "definition",
      env: "token=e2e",
    });
    const id = r.connection.id;
    await call(request, "ConnectionService", "EditConnection", { id, account: "居間の眼鏡" });
    try {
      await visit(page, "/settings/channels", "設定 · チャネル");
      const row = page.locator(".list > .item").filter({ hasText: "居間の眼鏡" });
      await expect(row).toContainText("E2E Glasses");
      await expect(row.getByRole("checkbox")).toBeChecked();

      await row.locator(".wg-switch").click();
      await expect(toast(page)).toHaveText("ここでは話しかけを受けない");
      await page.reload();
      await expect(row.getByRole("checkbox")).not.toBeChecked();
      const detail = await call<{ receivers: { destination?: string }[] }>(request, "ConnectionService", "GetConnection", {
        id,
      });
      expect(detail.receivers[0]?.destination ?? "").toBe("");

      await row.getByRole("link", { name: /居間の眼鏡/ }).click();
      await expect(page).toHaveURL(new RegExp(`/settings/services\\?id=${id}$`));
      const section = page.locator("section.pref").filter({ has: page.getByRole("heading", { name: "チャネル" }) });
      await expect(page.getByLabel("渡す先")).toHaveCount(0);
      await section.locator(".wg-switch").click();
      await expect(toast(page)).toHaveText("ここで話しかけを受ける");
      await expect(section.getByRole("checkbox")).toBeChecked();
    } finally {
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });
});

test.describe("サービス", () => {
  test("the services to add are searched in a dialog, and one whose app is missing says what to set up", async ({
    page,
  }) => {
    await visit(page, "/settings/services", "設定 · サービス");
    await expect(page.getByText("Notion")).toHaveCount(0);
    await page.getByRole("button", { name: "サービスを足す" }).click();
    const dialog = page.getByRole("dialog", { name: "サービスを足す" });
    const choices = dialog.getByRole("list", { name: "足せるサービス" });
    for (const name of [/GitHub/, /Google/, /Notion/, /Slack/])
      await expect(choices.getByRole("button", { name })).toBeVisible();

    const search = dialog.getByLabel("サービスを探す");
    await expect(search).toBeFocused();
    for (const [words, name] of [
      ["ノーション", /Notion/],
      ["gmail", /Google/],
      ["プルリクエスト", /GitHub/],
    ] as const) {
      await search.fill(words);
      await expect(choices.getByRole("button")).toHaveCount(1);
      await expect(choices.getByRole("button", { name })).toBeVisible();
    }

    for (const [name, id] of [
      ["GitHub", "github"],
      ["Google", "google"],
      ["Slack", "slack"],
    ] as const) {
      await search.fill(id);
      await search.press("Enter");
      await expect(dialog.getByText(new RegExp(`${name} のアプリが設定されていない`))).toBeVisible();
      await expect(dialog.getByRole("link", { name: `crates/gateway/services/${id}/README.md` })).toHaveAttribute(
        "href",
        new RegExp(`crates/gateway/services/${id}/README\\.md$`),
      );
      await expect(dialog.getByRole("button", { name: "足す", exact: true })).toHaveCount(0);
      await dialog.getByRole("button", { name: "一覧へ" }).click();
    }

    for (const [words, name] of [
      ["スライト", /Slite/],
      ["ドックベース", /DocBase/],
      ["フリー", /freee/],
      ["キントーン", /kintone/],
      ["アドロイト", /Adroit/],
      ["ディスコード", /Discord/],
      ["bsky", /Bluesky/],
      ["kot", /KING OF TIME/],
    ] as const) {
      await search.fill(words);
      await expect(choices.getByRole("button")).toHaveCount(1);
      await expect(choices.getByRole("button", { name })).toBeVisible();
    }
    await search.press("Enter");
    await expect(dialog.getByLabel("アクセストークン")).toHaveAttribute("type", "password");
    await dialog.getByRole("button", { name: "一覧へ" }).click();

    await search.fill("キベラ");
    await search.press("Enter");
    await dialog.getByLabel(/チームのサブドメイン/).fill("evil.example");
    await dialog.getByRole("button", { name: "足す", exact: true }).click();
    await expect(toast(page)).toContainText("チームのサブドメイン");
    await dialog.getByRole("button", { name: "一覧へ" }).click();

    await search.fill("そんなサービスはない");
    await expect(choices.getByRole("button")).toHaveCount(0);
    await expect(dialog.getByRole("button", { name: /一覧にないサービスをつなぐ/ })).toBeVisible();
  });

  test("what a service's instructions say to paste is read into how to connect it", async ({ page }) => {
    await visit(page, "/settings/services", "設定 · サービス");
    await page.getByRole("button", { name: "サービスを足す" }).click();
    const dialog = page.getByRole("dialog", { name: "サービスを足す" });
    await dialog.getByRole("button", { name: /一覧にないサービスをつなぐ/ }).click();
    const paste = dialog.getByLabel("URL・コマンド・設定を貼る");
    const connect = dialog.getByRole("button", { name: "つなぐ", exact: true });
    await expect(connect).toBeDisabled();

    await paste.fill("https://mcp.example.com/mcp");
    await expect(dialog.getByText("リモートの MCP サーバー", { exact: true })).toBeVisible();
    await expect(dialog.getByRole("radio", { name: "MCP" })).toBeChecked();
    await expect(dialog.getByLabel("鍵を入れるヘッダー")).toBeHidden();
    await expect(dialog.getByText("環境変数を足す")).toHaveCount(0);

    await paste.fill("https://api.example.com/v1/openapi.json");
    await expect(dialog.getByRole("radio", { name: "API" })).toBeChecked();
    await expect(dialog.getByLabel(/OpenAPI の URL/)).toHaveValue("https://api.example.com/v1/openapi.json");

    await paste.fill(
      '{"mcpServers":{"notes":{"command":"npx","args":["-y","notes-mcp","--root","/my notes"],"env":{"NOTES_TOKEN":"<your token>","NOTES_MODE":"read"}}}}',
    );
    await expect(dialog.getByText("このマシンで動かす MCP サーバー", { exact: true })).toBeVisible();
    await expect(dialog.getByText('npx -y notes-mcp --root "/my notes"', { exact: true })).toBeVisible();
    await expect(dialog.getByLabel("名前（なくてもよい）")).toHaveValue("notes");
    await expect(dialog.getByLabel("NOTES_TOKEN の値")).toHaveValue("");
    await expect(dialog.getByLabel("NOTES_MODE の値")).toHaveValue("read");
    await dialog.getByLabel("NOTES_TOKEN の値").pressSequentially("abc");
    await expect(dialog.getByLabel("NOTES_TOKEN の値")).toHaveValue("abc");

    await paste.fill('claude mcp add --transport http board https://mcp.board.example/mcp --header "X-Api-Key: k-1"');
    await expect(dialog.getByText("https://mcp.board.example/mcp", { exact: true })).toBeVisible();
    await expect(dialog.getByLabel("名前（なくてもよい）")).toHaveValue("board");
    await expect(dialog.getByLabel("鍵を入れるヘッダー")).toHaveValue("X-Api-Key");

    await paste.fill('{"mcpServers":{"one":{"url":"https://one.example/mcp"},"two":{"command":"npx","args":["mcp-remote","https://two.example/sse"]}}}');
    await dialog.getByText("two", { exact: true }).click();
    await expect(dialog.getByText("https://two.example/sse", { exact: true })).toBeVisible();
    await expect(dialog.getByText("リモートの MCP サーバー", { exact: true })).toBeVisible();

    await paste.fill('{"mcpServers":');
    await expect(dialog.getByText(/つなぎ方を読み取れない/)).toBeVisible();
    await expect(connect).toBeDisabled();
  });

  test("the connected services are searched by name and account", async ({ page, request }) => {
    const ids = [await addStandIn(request, "帳簿", "ledger"), await addStandIn(request, "図書館", "library")];
    await call(request, "ConnectionService", "EditConnection", { id: ids[1], account: "reader@example.com" });
    try {
      await visit(page, "/settings/services", "設定 · サービス");
      await page.getByLabel("つないだサービスを探す").fill("reader@");
      const connected = page.locator("section").filter({ has: page.getByRole("heading", { name: "つないだサービス" }) });
      await expect(connected.getByRole("link", { name: /図書館/ })).toBeVisible();
      await expect(connected.getByRole("link", { name: /帳簿/ })).toHaveCount(0);
    } finally {
      for (const id of ids) await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("a Google account's page lists its calendars, which are added and removed there", async ({ page, request }) => {
    const id = await addStandIn(request, "Google · 仕事", "google");
    await call(request, "CalendarService", "AddCalendar", { name: "祝日", googleConnection: id, googleCalendar: "holidays" });
    await call(request, "CalendarService", "AddCalendar", { name: "よその", googleConnection: "google-elsewhere" });
    try {
      await visit(page, `/settings/services?id=${id}`, `設定 · サービス · ${id}`);
      const section = page.locator("section.pref").filter({ has: page.getByRole("heading", { name: "カレンダー" }) });
      await expect(section.getByText("祝日", { exact: true })).toBeVisible();
      await expect(section.getByText("よその", { exact: true })).toHaveCount(0);
      await expect(page.getByLabel("購読 URL")).toHaveCount(0);

      await section.getByLabel("カレンダーの名前").fill("わたし");
      await section.getByRole("button", { name: "足す" }).click();
      await expect(toast(page)).toHaveText("足した");
      await expect(section.locator(".item").filter({ hasText: "わたし" })).toContainText("メインのカレンダー");

      await section.getByRole("button", { name: "祝日を外す" }).click();

      await agree(page, "外す");
      await expect(toast(page)).toHaveText("外した");
      await expect(section.getByText("祝日", { exact: true })).toHaveCount(0);
    } finally {
      const left = await call<{ calendars?: { id: string; name: string }[] }>(request, "CalendarService", "ListCalendars", {});
      for (const c of (left.calendars ?? []).filter((c) => ["祝日", "よその", "わたし"].includes(c.name)))
        await call(request, "CalendarService", "RemoveCalendar", { id: c.id });
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("the secretary's own tools are listed with how far each is trusted", async ({ page, request }) => {
    const detail = await call<Detail>(request, "ConnectionService", "GetConnection", { id: "famulen" });
    const tool = detail.tools[0]?.name ?? "";
    expect(tool).not.toBe("");
    await visit(page, "/settings/services", "設定 · サービス");

    await page.getByRole("link", { name: /自身の道具/ }).click();
    await expect(page).toHaveURL(/\/settings\/services\?id=famulen$/);
    await expect(page.getByText("Famulen が自分で持つ道具", { exact: false })).toBeVisible();

    const row = page.locator(".list > .item").filter({ has: page.getByText(tool, { exact: true }) });
    await expect(row.getByRole("radio", { name: "既定（まかせる）" })).toBeChecked();
    await expect(row.getByRole("radio", { name: "拒否する" })).toHaveCount(0);

    await row.getByText("上書き", { exact: true }).click();
    await expect(toast(page)).toHaveText("既定を上書きした");
    await expect(row.getByRole("radio", { name: "上書き" })).toBeChecked();
    for (const name of ["まかせる", "許可する", "拒否する", "確認する"]) {
      await expect(row.getByRole("radio", { name, exact: true })).toBeVisible();
    }
    await expect(row.getByRole("radio", { name: "まかせる", exact: true })).toBeChecked();
    await row.getByText("拒否する", { exact: true }).click();
    await expect(toast(page)).toHaveText("保存した");
    await expect(row.getByRole("radio", { name: "拒否する" })).toBeChecked();
    await page.reload();
    await expect(row.getByRole("radio", { name: "拒否する" })).toBeChecked();
    await expectView(page, "設定 · サービス · famulen");

    await row.getByText("既定（まかせる）", { exact: true }).click();
    await expect(toast(page)).toHaveText("既定に戻した");
    await expect(row.getByRole("radio", { name: "既定（まかせる）" })).toBeChecked();
    await expect(row.getByRole("radio", { name: "拒否する" })).toHaveCount(0);
  });

  test("the bulk buttons set every read-only tool at once and 既定に戻す clears them", async ({ page, request }) => {
    await visit(page, "/settings/services?id=famulen", "設定 · サービス · famulen");
    const bulk = page.locator(".row").filter({ hasText: "読むだけの道具を" });
    const first = await page.locator(".tool-row").first().elementHandle();
    await bulk.getByRole("button", { name: "拒否する" }).click();
    await expect(toast(page)).toHaveText("保存した");
    await expect(page.getByRole("radio", { name: "拒否する" }).first()).toBeChecked();
    expect(await first?.evaluate((el) => el.isConnected), "the row is not drawn anew").toBe(true);

    await bulk.getByRole("button", { name: "既定に戻す" }).click();
    await expect(toast(page)).toHaveText("保存した");
    const detail = await call<{ tools: { permission?: string }[] }>(request, "ConnectionService", "GetConnection", { id: "famulen" });
    expect(detail.tools.filter((t) => t.permission === "PERMISSION_FORBID")).toEqual([]);
  });

  test("a server added by its command is listed, renamed on its page and removed", async ({ page }) => {
    await visit(page, "/settings/services", "設定 · サービス");
    await page.getByRole("button", { name: "サービスを足す" }).click();
    const dialog = page.getByRole("dialog", { name: "サービスを足す" });
    await dialog.getByLabel("サービスを探す").fill("famulen-e2e-missing-server --stdio");
    await dialog.getByLabel("サービスを探す").press("Enter");
    await dialog.getByLabel("名前（なくてもよい）").fill("手元の道具箱");
    await dialog.getByRole("button", { name: "環境変数を足す" }).click();
    await dialog.getByLabel("環境変数 1 の名前").pressSequentially("BOX_TOKEN");
    await dialog.getByLabel("BOX_TOKEN の値").fill("e2e-box-secret");
    await dialog.getByRole("button", { name: "つなぐ", exact: true }).click();
    await expect(toast(page)).toHaveText("つないだ");
    await expect(dialog).toHaveCount(0);
    await expect(page).toHaveURL(/\/settings\/services\?id=/);
    await expect(page.getByRole("heading", { name: "手元の道具箱" })).toBeVisible();
    await expect(page.locator("body")).not.toContainText("e2e-box-secret");
    await expect(page.getByText("famulen-e2e-missing-server", { exact: false }).first()).toBeVisible();

    await expect(page.getByLabel("サービス種")).toBeHidden();
    await page.getByRole("button", { name: "詳しい設定" }).click();
    await expect(page.getByLabel("サービス種")).toBeVisible();
    await expect(page.getByLabel("アカウントの目印")).toBeEditable();
    await page.getByLabel("名前").fill("手元のサーバー");
    await page.getByLabel("名前").press("Tab");
    await expect(toast(page)).toHaveText("保存した");
    await expect(page.getByRole("heading", { name: "手元のサーバー" })).toBeVisible();

    await page.getByRole("button", { name: "外す", exact: true }).click();

    await agree(page, "外す");
    await expect(page).toHaveURL(/\/settings\/services$/);
    await expect(toast(page)).toHaveText("外した");
    await expect(page.getByRole("link", { name: /手元のサーバー/ })).toHaveCount(0);
  });
});

test.describe("見張り", () => {
  test("a preset is added with one press, can be stopped and started, and removed", async ({ page, request }) => {
    const r = await call<Connected>(request, "ConnectionService", "AddConnection", {
      target: "e2e-hook",
      how: "definition",
      env: "token=e2e",
    });
    const id = r.connection.id;
    await call(request, "ConnectionService", "EditConnection", { id, name: "仕事のフック" });
    try {
      await visit(page, "/settings/lookout", "設定 · 見張り");
      await page.getByRole("button", { name: /仕事のフック · 届いたもの.*受け口に届いたもの/ }).click();
      await expect(toast(page)).toHaveText("見張りに足した");

      const sources = page.locator("section").filter({ has: page.getByRole("heading", { name: "見張るもの" }) });
      const source = sources.locator(".list > .item").filter({ hasText: "仕事のフック" });
      await expect(source.getByRole("radio", { name: "既定（ほか・15 分）" })).toBeChecked();
      await expect(source.getByRole("checkbox", { name: /見張っている/ })).toBeChecked();

      await source.locator(".switch").click();
      await expect(toast(page)).toHaveText("止めた");
      await expect(source.getByRole("checkbox", { name: /止めている/ })).not.toBeChecked();
      await page.reload();
      await expect(source.getByText("止めている")).toBeVisible();
      await source.locator(".switch").click();
      await expect(toast(page)).toHaveText("見張りを始めた");

      await source.getByRole("button", { name: "外す" }).click();

      await agree(page, "外す");
      await expect(toast(page)).toHaveText("外した");
      await expect(source).toHaveCount(0);
    } finally {
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("a lookout of one's own is added in a dialog with its connection, tool, arguments and interval", async ({ page, request }) => {
    const id = await addApiStandIn(request, "社内の課題", "e2e-tasks");
    try {
      await visit(page, "/settings/lookout", "設定 · 見張り");
      await page.getByRole("button", { name: "見張りを足す" }).click();
      const dialog = page.getByRole("dialog", { name: "見張りを足す" });
      await dialog.getByLabel("名前").fill("担当の課題");
      await dialog.getByRole("combobox", { name: /^つなぎ先/ }).selectOption({ label: "社内の課題" });
      await dialog.getByLabel("読む道具").selectOption("http_get");
      await dialog.getByLabel("path").fill("/issues");
      await dialog.getByLabel("query").fill('{"assignee":"me"}');
      await expect(dialog.getByRole("radio", { name: "既定", exact: true })).toBeChecked();
      await expect(dialog.getByText("既定は、メール 30 分・チャット 5 分・ほか 15 分。")).toBeVisible();
      await expect(dialog.getByLabel("間隔（分）")).toHaveCount(0);
      await dialog.getByRole("radiogroup", { name: "間隔の決め方" }).getByText("上書き", { exact: true }).click();
      await expect(dialog.getByLabel("間隔（分）")).toBeFocused();
      await dialog.getByRole("button", { name: "足す", exact: true }).click();
      await expect(dialog).toBeVisible();
      await dialog.getByLabel("間隔（分）").fill("30");
      await dialog.getByRole("button", { name: "足す", exact: true }).click();

      await expect(toast(page)).toHaveText("見張りに足した");
      await expect(dialog).toHaveCount(0);
      const source = page.locator(".list > .item").filter({ hasText: "担当の課題" });
      await expect(source.getByLabel("担当の課題を見る間隔（分）")).toHaveValue("30");
      await expect(source).toContainText("分ごとに読みに行く");

      await source.getByRole("button", { name: "外す" }).click();

      await agree(page, "外す");
      await expect(source).toHaveCount(0);
    } finally {
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("what a connection brings is read without naming a tool, which stays folded away", async ({ page, request }) => {
    const r = await call<Connected>(request, "ConnectionService", "AddConnection", {
      target: "e2e-hook",
      how: "definition",
      env: "token=e2e",
    });
    const id = r.connection.id;
    await call(request, "ConnectionService", "EditConnection", { id, name: "畳んだフック" });
    try {
      await visit(page, "/settings/lookout", "設定 · 見張り");
      await page.getByRole("button", { name: "見張りを足す" }).click();
      const dialog = page.getByRole("dialog", { name: "見張りを足す" });
      await expect(dialog.getByLabel("読む道具")).toBeHidden();
      await dialog.getByLabel("名前").fill("畳んだ見張り");
      await dialog.getByRole("combobox", { name: /^つなぎ先/ }).selectOption({ label: "畳んだフック" });
      await expect(dialog.getByText("このつなぎ先に届いたものを読む。")).toBeVisible();
      await expect(dialog.getByLabel("読む道具")).toBeHidden();
      await dialog.getByRole("button", { name: "詳しい設定" }).click();
      await expect(dialog.getByLabel("読む道具")).toBeVisible();
      await expect(dialog.getByLabel("path")).toHaveCount(0);
      await dialog.getByRole("button", { name: "足す", exact: true }).click();
      await expect(toast(page)).toHaveText("見張りに足した");
      const source = page.locator(".list > .item").filter({ hasText: "畳んだ見張り" });
      await expect(source).toBeVisible();
      await source.getByRole("button", { name: "外す" }).click();
      await agree(page, "外す");
      await expect(source).toHaveCount(0);
    } finally {
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("a source added on its own is edited in the dialog: its name, its interval, where it goes and its workgroup", async ({ page, request }) => {
    const r = await call<Connected>(request, "ConnectionService", "AddConnection", {
      target: "e2e-hook",
      how: "definition",
      env: "token=e2e",
    });
    const id = r.connection.id;
    const added = await call<{ id: string }>(request, "LookoutService", "AddLookout", { name: "直す見張り", connection: id });
    const group = await call<{ id: string }>(request, "WorkgroupService", "AddWorkgroup", { name: "見張りの組", color: "#0B5650" });
    try {
      await visit(page, "/settings/lookout", "設定 · 見張り");
      await page.getByRole("button", { name: "直す見張りを直す" }).click();
      const dialog = page.getByRole("dialog", { name: "見張りを直す" });
      await expect(dialog.getByRole("radiogroup", { name: "どこへの決め方" }).getByRole("radio", { name: "見張りが決める" })).toBeChecked();
      await expect(dialog.getByRole("radio", { name: "朝のまとめ" })).toHaveCount(0);
      await dialog.getByLabel("名前").fill("直した見張り");
      await dialog.getByRole("radiogroup", { name: "どこへの決め方" }).getByText("上書き", { exact: true }).click();
      await dialog.getByText("朝のまとめ", { exact: true }).click();
      await dialog.getByRole("radiogroup", { name: "間隔の決め方" }).getByText("上書き", { exact: true }).click();
      await dialog.getByLabel("間隔（分）").fill("25");
      await dialog.getByLabel("ワークグループ").selectOption({ label: "見張りの組" });
      await dialog.getByRole("button", { name: "保存" }).click();
      await expect(toast(page)).toHaveText("見張りを直した");
      await expect(dialog).toHaveCount(0);

      const row = page.locator(".list > .item").filter({ hasText: "直した見張り" });
      await expect(row).toContainText("朝のまとめへ");
      await expect(row.getByLabel("直した見張りを見る間隔（分）")).toHaveValue("25");
      await page.reload();
      await expectView(page, "設定 · 見張り");
      await page.getByRole("button", { name: "直した見張りを直す" }).click();
      await expect(dialog.getByRole("radio", { name: "朝のまとめ" })).toBeChecked();
      await expect(dialog.getByLabel("ワークグループ")).toHaveValue(group.id);
      await dialog.getByText("見張りが決める", { exact: true }).click();
      await dialog.getByLabel("ワークグループ").selectOption({ label: "全体" });
      await dialog.getByRole("button", { name: "保存" }).click();
      await expect(toast(page)).toHaveText("見張りを直した");
      await expect(row).not.toContainText("朝のまとめへ");

      await visit(page, `/settings/services?id=${id}`, `設定 · サービス · ${id}`);
      await page.getByRole("button", { name: "直した見張りを直す" }).click();
      await expect(dialog.getByLabel("ワークグループ")).toHaveValue("all");
    } finally {
      await call(request, "LookoutService", "RemoveLookout", { id: added.id });
      await call(request, "WorkgroupService", "DeleteWorkgroup", { id: group.id });
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("what the owner chose in the dialog stays when the choices arrive late", async ({ page, request }) => {
    const r = await call<Connected>(request, "ConnectionService", "AddConnection", { target: "e2e-hook", how: "definition", env: "token=e2e" });
    const id = r.connection.id;
    const added = await call<{ id: string }>(request, "LookoutService", "AddLookout", { name: "遅い見張り", connection: id });
    let arrive = () => {};
    const late = new Promise<void>((done) => {
      arrive = done;
    });
    await page.route("**/famulen.v1.LookoutService/GetLookoutChoices*", async (route) => {
      await late;
      await route.continue();
    });
    try {
      await visit(page, "/settings/lookout", "設定 · 見張り");
      await page.getByRole("button", { name: "遅い見張りを直す" }).click();
      const dialog = page.getByRole("dialog", { name: "見張りを直す" });
      await dialog.getByLabel("名前").fill("変えた見張り");
      await dialog.getByRole("radiogroup", { name: "どこへの決め方" }).getByText("上書き", { exact: true }).click();
      const arrived = page.waitForResponse(/GetLookoutChoices/);
      arrive();
      await arrived;
      await dialog.getByRole("button", { name: "保存" }).click();
      await expect(toast(page)).toHaveText("見張りを直した");
      await expect(page.locator(".list > .item").filter({ hasText: "変えた見張り" })).toContainText("秘書へ");
    } finally {
      arrive();
      await call(request, "LookoutService", "RemoveLookout", { id: added.id });
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("a source reads only the places chosen for it, and the rest stays with the one choosing none", async ({ page, request }) => {
    const r = await call<Connected>(request, "ConnectionService", "AddConnection", {
      target: "e2e-hook",
      how: "definition",
      env: "token=e2e",
    });
    const id = r.connection.id;
    await call(request, "ConnectionService", "EditConnection", { id, name: "場所のフック" });
    const rest = await call<{ id: string }>(request, "LookoutService", "AddLookout", { name: "残りの見張り", connection: id });
    await page.route("**/famulen.v1.LookoutService/GetLookoutChoices*", (route) =>
      route.fulfill({
        json: {
          receives: true,
          placeLabel: "ラベル",
          places: [
            { id: "L1", name: "仕事" },
            { id: "L2", name: "家" },
          ],
        },
      }),
    );
    try {
      await visit(page, "/settings/lookout", "設定 · 見張り");
      await page.getByRole("button", { name: "見張りを足す" }).click();
      const dialog = page.getByRole("dialog", { name: "見張りを足す" });
      await dialog.getByLabel("名前").fill("仕事だけの見張り");
      await dialog.getByRole("combobox", { name: /^つなぎ先/ }).selectOption({ label: "場所のフック" });
      await expect(dialog.getByText("ラベルを選ばないものは「残りの見張り」が見張っている。", { exact: false })).toBeVisible();
      await expect(dialog.getByRole("checkbox", { name: "仕事" })).toBeVisible();
      await dialog.getByRole("button", { name: "足す", exact: true }).click();
      await expect(toast(page)).toHaveText("ラベルを一つは選ぶ");
      await dialog.getByRole("checkbox", { name: "仕事" }).check();
      await dialog.getByRole("radiogroup", { name: "どこへの決め方" }).getByText("上書き", { exact: true }).click();
      await dialog.getByText("ログだけ", { exact: true }).click();
      await dialog.getByRole("button", { name: "足す", exact: true }).click();
      await expect(toast(page)).toHaveText("見張りに足した");
      const row = page.locator(".list > .item").filter({ hasText: "仕事だけの見張り" });
      await expect(row).toContainText("仕事 だけ");
      await expect(row).toContainText("ログだけ");

      await page.getByRole("button", { name: "仕事だけの見張りを直す" }).click();
      const edit = page.getByRole("dialog", { name: "見張りを直す" });
      await expect(edit.getByRole("checkbox", { name: "仕事" })).toBeChecked();
      await edit.getByRole("checkbox", { name: "家" }).check();
      await edit.getByRole("button", { name: "保存" }).click();
      await expect(toast(page)).toHaveText("見張りを直した");
      await expect(row).toContainText("仕事・家 だけ");
    } finally {
      const { sources } = await call<{ sources: { id: string; connection: string }[] }>(request, "LookoutService", "GetLookout");
      for (const s of sources.filter((s) => s.connection === id)) await call(request, "LookoutService", "RemoveLookout", { id: s.id });
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
    expect(rest.id).not.toBe("");
  });

  test("the tool's arguments are changed after it was added, keeping what its fields do not ask for", async ({ page, request }) => {
    const id = await addApiStandIn(request, "直す API", "e2e-reading");
    const added = await call<{ id: string }>(request, "LookoutService", "AddLookout", {
      name: "条件の見張り",
      connection: id,
      tool: "http_get",
      args: '{"path":"/a","extra":1}',
    });
    try {
      await visit(page, "/settings/lookout", "設定 · 見張り");
      await page.getByRole("button", { name: "条件の見張りを直す" }).click();
      const dialog = page.getByRole("dialog", { name: "見張りを直す" });
      await expect(dialog.getByLabel("読む道具")).toHaveValue("http_get");
      await expect(dialog.getByLabel("path")).toHaveValue("/a");
      await dialog.getByLabel("path").fill("/b");
      await dialog.getByRole("button", { name: "保存" }).click();
      await expect(toast(page)).toHaveText("見張りを直した");
      const { sources } = await call<{ sources: { id: string; args: string }[] }>(request, "LookoutService", "GetLookout");
      expect(JSON.parse(sources.find((s) => s.id === added.id)?.args ?? "")).toEqual({ path: "/b", extra: 1 });
    } finally {
      await call(request, "LookoutService", "RemoveLookout", { id: added.id });
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("an argument is named as the tool titles it, or else by its own name", async ({ page, request }) => {
    const id = await addApiStandIn(request, "題のある API", "e2e-titled");
    const properties = { query: { type: "string", title: "検索の言葉", description: "探す言葉" }, max: { type: "integer" } };
    await page.route("**/famulen.v1.LookoutService/GetLookoutChoices*", (route) =>
      route.fulfill({
        json: { tools: [{ name: "search", description: "探す", inputSchema: JSON.stringify({ type: "object", properties, required: ["query"] }) }] },
      }),
    );
    try {
      await visit(page, "/settings/lookout", "設定 · 見張り");
      await page.getByRole("button", { name: "見張りを足す" }).click();
      const dialog = page.getByRole("dialog", { name: "見張りを足す" });
      await dialog.getByRole("combobox", { name: /^つなぎ先/ }).selectOption({ label: "題のある API" });
      await dialog.getByLabel("読む道具").selectOption("search");
      await expect(dialog.getByRole("textbox", { name: /^検索の言葉/ })).toHaveAttribute("required", "");
      await expect(dialog.getByText("探す言葉", { exact: false })).toBeVisible();
      await expect(dialog.getByRole("spinbutton", { name: /^max/ })).toBeVisible();
    } finally {
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("a connection already watched for what arrives asks for a tool instead of watching it twice", async ({ page, request }) => {
    const r = await call<Connected>(request, "ConnectionService", "AddConnection", {
      target: "e2e-hook",
      how: "definition",
      env: "token=e2e",
    });
    const id = r.connection.id;
    await call(request, "ConnectionService", "EditConnection", { id, name: "見張り済みのフック" });
    const added = await call<{ id: string }>(request, "LookoutService", "AddLookout", { name: "先の見張り", connection: id });
    try {
      await visit(page, "/settings/lookout", "設定 · 見張り");
      await page.getByRole("button", { name: "見張りを足す" }).click();
      const dialog = page.getByRole("dialog", { name: "見張りを足す" });
      await dialog.getByRole("combobox", { name: /^つなぎ先/ }).selectOption({ label: "見張り済みのフック" });
      await expect(dialog.getByText("このつなぎ先に届いたものは「先の見張り」が見張っている。", { exact: false })).toBeVisible();
      await expect(dialog.getByLabel("読む道具")).toBeVisible();
    } finally {
      await call(request, "LookoutService", "RemoveLookout", { id: added.id });
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("a connection that brings nothing asks for a tool, offering the ones that only read", async ({ page, request }) => {
    const r = await call<Connected>(request, "ConnectionService", "AddConnection", { target: "http://127.0.0.1:9/", how: "api" });
    const id = r.connection.id;
    await call(request, "ConnectionService", "EditConnection", { id, name: "届かない API" });
    try {
      await visit(page, "/settings/lookout", "設定 · 見張り");
      await page.getByRole("button", { name: "見張りを足す" }).click();
      const dialog = page.getByRole("dialog", { name: "見張りを足す" });
      await dialog.getByLabel("名前").fill("届かない見張り");
      await dialog.getByRole("combobox", { name: /^つなぎ先/ }).selectOption({ label: "届かない API" });
      await expect(dialog.getByText("このつなぎ先からは何も届かない。", { exact: false })).toBeVisible();
      const tool = dialog.getByLabel("読む道具");
      await expect(tool).toBeVisible();
      await expect(tool.locator("option")).toHaveText(["選ぶ", "http_get"]);
      await dialog.getByRole("button", { name: "足す", exact: true }).click();
      await expect(dialog).toBeVisible();
      await tool.selectOption("http_get");
      await expect(dialog.locator(".sub", { hasText: /^GET/ })).toBeVisible();
      await expect(dialog.getByLabel("path")).toHaveAttribute("required", "");
    } finally {
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("a lookout added without overriding follows the default", async ({ page, request }) => {
    const id = await addApiStandIn(request, "既定の課題", "e2e-default");
    try {
      await visit(page, "/settings/lookout", "設定 · 見張り");
      await page.getByRole("button", { name: "見張りを足す" }).click();
      const dialog = page.getByRole("dialog", { name: "見張りを足す" });
      await dialog.getByLabel("名前").fill("既定で見る課題");
      await dialog.getByRole("combobox", { name: /^つなぎ先/ }).selectOption({ label: "既定の課題" });
      await dialog.getByLabel("読む道具").selectOption("http_get");
      await dialog.getByLabel("path").fill("/");
      const ways = dialog.getByRole("radiogroup", { name: "間隔の決め方" });
      await ways.getByText("上書き", { exact: true }).click();
      await dialog.getByLabel("間隔（分）").fill("40");
      await ways.getByText("既定", { exact: true }).click();
      await dialog.getByRole("button", { name: "足す", exact: true }).click();
      await expect(toast(page)).toHaveText("見張りに足した");
      const source = page.locator(".list > .item").filter({ hasText: "既定で見る課題" });
      await expect(source.getByRole("radio", { name: "既定（ほか・15 分）" })).toBeChecked();
      await source.getByRole("button", { name: "外す" }).click();
      await agree(page, "外す");
      await expect(source).toHaveCount(0);
    } finally {
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("a Slack workspace is offered once, as the connection its events arrive on", async ({ page, request }) => {
    const id = await addStandIn(request, "ワークスペース", "e2e-workspace");
    try {
      await page.route("**/famulen.v1.ChannelService/ListWorkspaces*", (route) =>
        route.fulfill({ json: { workspaces: [{ id, name: "ワークスペース" }] } }),
      );
      await visit(page, "/settings/lookout", "設定 · 見張り");
      await page.getByRole("button", { name: "見張りを足す" }).click();
      const options = page.getByRole("dialog", { name: "見張りを足す" }).locator("option", { hasText: "ワークスペース" });
      await expect(options).toHaveText(["Slack のチャネルに届く出来事 · ワークスペース"]);
      await expect(options).toHaveAttribute("value", id);
    } finally {
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("past mail is gone back over from the lookout's page, with how far it got, and stopped", async ({ page, request }) => {
    const mailbox = (more: object) => ({ connection: "g", name: "Google · me@example.com", ...more });
    let now: object = mailbox({});
    const asked: unknown[] = [];
    await page.route("**/famulen.v1.LookoutService/ListPastPurchases*", (route) => route.fulfill({ json: { mailboxes: [now] } }));
    await page.route("**/famulen.v1.LookoutService/StartPastPurchases", (route) => {
      asked.push(route.request().postDataJSON());
      now = mailbox({ state: "PAST_MAIL_STATE_GOING", since: "2021-01-01", month: "2022-05-01", found: "12", left: "3", added: "4" });
      return route.fulfill({ json: now });
    });
    await page.route("**/famulen.v1.LookoutService/StopPastPurchases", (route) => {
      now = { ...now, state: "PAST_MAIL_STATE_STOPPED" };
      return route.fulfill({ json: now });
    });
    await visit(page, "/settings/lookout", "設定 · 見張り");
    // The page came from the server, which has no such account; it is read anew.
    await record(request, "memos", "さかのぼりの頁を読み直させる");
    const row = page.locator(".past-mail");
    await expect(row).toContainText("まださかのぼっていない");

    await row.getByLabel("さかのぼる年数").selectOption({ label: "5 年" });
    await row.getByRole("button", { name: "さかのぼる", exact: false }).click();
    await agree(page, "さかのぼる");
    await expect(row).toContainText("2022-05 を探している");
    await expect(row).toContainText("見つけたメール 12 通 · 残り 3 通 · 足した物 4 件");
    const year = Number(dayFromToday(0).slice(0, 4)) - 5;
    expect(asked).toEqual([{ connection: "g", since: expect.stringMatching(new RegExp(`^${year}-`)) }]);

    await row.getByRole("button", { name: /止める/ }).click();
    await expect(row).toContainText("2022-05 で止めている");
    await row.getByRole("button", { name: /続ける/ }).click();
    await expect(row).toContainText("2022-05 を探している");
    expect(asked[1]).toEqual({ connection: "g", since: "2021-01-01" });
  });

  test("past mail is gone back over for contracts apart from what was bought", async ({ page, request }) => {
    const mailbox = { connection: "g", name: "Google · me@example.com" };
    let now: object = mailbox;
    await page.route("**/famulen.v1.LookoutService/ListPastPurchases*", (route) => route.fulfill({ json: { mailboxes: [mailbox] } }));
    await page.route("**/famulen.v1.LookoutService/ListPastContracts*", (route) => route.fulfill({ json: { mailboxes: [now] } }));
    await page.route("**/famulen.v1.LookoutService/StartPastContracts", (route) => {
      now = { ...mailbox, state: "PAST_MAIL_STATE_GOING", since: "2021-01-01", month: "2022-05-01", found: "12", left: "3", added: "4" };
      return route.fulfill({ json: now });
    });
    await visit(page, "/settings/lookout", "設定 · 見張り");
    await record(request, "memos", "契約のさかのぼりの頁を読み直させる");
    const [bought, held] = [page.locator(".past-mail").first(), page.locator(".past-mail").last()];
    await expect(page.locator(".past-mail")).toHaveCount(2);
    await held.getByRole("button", { name: "さかのぼる", exact: false }).click();
    await agree(page, "さかのぼる");
    await expect(held).toContainText("見つけたメール 12 通 · 残り 3 通 · 足した契約 4 件");
    await expect(bought).toContainText("まださかのぼっていない");
  });

  test("broken JSON arguments are refused with the reason and the dialog stays open", async ({ page, request }) => {
    const id = await addApiStandIn(request, "壊れた引数の先", "e2e-broken");
    try {
      await visit(page, "/settings/lookout", "設定 · 見張り");
      await page.getByRole("button", { name: "見張りを足す" }).click();
      const dialog = page.getByRole("dialog", { name: "見張りを足す" });
      await dialog.getByLabel("名前").fill("壊れた見張り");
      await dialog.getByRole("combobox", { name: /^つなぎ先/ }).selectOption({ label: "壊れた引数の先" });
      await dialog.getByLabel("読む道具").selectOption("http_get");
      await dialog.getByLabel("path").fill("/");
      await dialog.getByLabel("query").fill("{not json");
      await dialog.getByRole("button", { name: "足す", exact: true }).click();
      await expect(toast(page)).toHaveClass(/bad/);
      await expect(dialog).toBeVisible();
      await expect(page.locator(".list > .item").filter({ hasText: "壊れた見張り" })).toHaveCount(0);
    } finally {
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("a source uses the default interval or an overriding one, and a number is asked only for the override", async ({ page, request }) => {
    const r = await call<Connected>(request, "ConnectionService", "AddConnection", {
      target: "e2e-hook",
      how: "definition",
      env: "token=e2e",
    });
    const id = r.connection.id;
    await call(request, "ConnectionService", "EditConnection", { id, name: "間隔のフック" });
    try {
      await visit(page, "/settings/lookout", "設定 · 見張り");
      await page.getByRole("button", { name: /間隔のフック · 届いたもの/ }).click();
      await expect(toast(page)).toHaveText("見張りに足した");
      const source = page.locator(".list > .item").filter({ hasText: "間隔のフック" });
      const fallback = source.getByRole("radio", { name: "既定（ほか・15 分）" });
      const override = source.getByRole("radio", { name: "上書き" });
      const interval = source.getByLabel("間隔のフック · 届いたものを見る間隔（分）");
      await expect(fallback).toBeChecked();
      await expect(interval).toHaveCount(0);

      await source.getByText("上書き", { exact: true }).click();
      await expect(toast(page)).toHaveText("既定を上書きした");
      await expect(override).toBeChecked();
      await expect(interval).toHaveValue("15");
      await expect(interval).toBeFocused();
      await expect(source).toContainText("分ごとにまとめて見る");
      await interval.fill("40");
      await interval.press("Tab");
      await expect(toast(page)).toHaveText("間隔を変えた");
      await page.reload();
      await expect(override).toBeChecked();
      await expect(interval).toHaveValue("40");
      const own = page.getByRole("list", { name: "間隔を決めている見張り" });
      await expect(own).toContainText("間隔のフック · 届いたもの");
      await expect(own).toContainText("既定（ほか・15 分）");

      await expectView(page, "設定 · 見張り");
      await source.getByText("既定（ほか・15 分）", { exact: true }).click();
      await expect(toast(page)).toHaveText("既定に戻した");
      await expect(fallback).toBeChecked();
      await expect(interval).toHaveCount(0);
      await expect(own).toHaveCount(0);

      await visit(page, `/settings/services?id=${id}`, `設定 · サービス · ${id}`);
      const section = page.locator(".pref").filter({ has: page.getByRole("heading", { name: "見張り", exact: true }) });
      await expect(section.getByRole("radio", { name: "既定（ほか・15 分）" })).toBeChecked();
      await section.getByText("上書き", { exact: true }).click();
      await expect(section.getByLabel("間隔のフック · 届いたものをまとめて見る間隔（分）")).toHaveValue("15");

      await visit(page, "/settings/lookout", "設定 · 見張り");
      await source.getByRole("button", { name: "外す" }).click();
      await agree(page, "外す");
      await expect(source).toHaveCount(0);
    } finally {
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("a number being typed outlives the reload after another save", async ({ page, request }) => {
    const r = await call<Connected>(request, "ConnectionService", "AddConnection", {
      target: "e2e-hook",
      how: "definition",
      env: "token=e2e",
    });
    const id = r.connection.id;
    await call(request, "ConnectionService", "EditConnection", { id, name: "打ちかけのフック" });
    const added = await call<{ id: string }>(request, "LookoutService", "AddLookout", {
      name: "打ちかけの見張り",
      connection: id,
      intervalMinutes: 40,
    });
    let hold: Promise<void> | undefined;
    await page.route(/\/famulen\.v1\.[A-Za-z]+\/(Get|List)/, async (route) => {
      await hold;
      await route.continue();
    });
    /** Type into `field` while the reload after `save` is still on its way. */
    const typeDuring = async (save: () => Promise<void>, field: ReturnType<typeof page.getByLabel>, text: string) => {
      let release = () => {};
      hold = new Promise((done) => {
        release = done;
      });
      await save();
      await field.fill(text);
      release();
      await expect(toast(page)).toBeVisible();
      await expect(field).toHaveValue(text);
      await expect(field).toBeFocused();
      await page.reload();
    };
    try {
      await visit(page, "/settings/lookout", "設定 · 見張り");
      const source = page.locator(".list > .item").filter({ hasText: "打ちかけの見張り" });
      const toggle = () => source.locator(".switch").click();
      await typeDuring(toggle, source.getByLabel("打ちかけの見張りを見る間隔（分）"), "7");
      await expectView(page, "設定 · 見張り");
      await typeDuring(toggle, page.getByLabel("ほかの既定の間隔（分）"), "8");
      await expectView(page, "設定 · 見張り");
      await typeDuring(toggle, page.getByLabel("打ちかけの見張りだけの間隔（分）"), "9");

      await visit(page, `/settings/services?id=${id}`, `設定 · サービス · ${id}`);
      const section = page.locator(".pref").filter({ has: page.getByRole("heading", { name: "見張り", exact: true }) });
      await typeDuring(
        () => section.locator(".switch").click(),
        section.getByLabel("打ちかけの見張りをまとめて見る間隔（分）"),
        "6",
      );
    } finally {
      hold = undefined;
      await page.unrouteAll({ behavior: "ignoreErrors" });
      await call(request, "LookoutService", "RemoveLookout", { id: added.id });
      await call(request, "LookoutService", "SetLookoutDefault", { feed: "other", intervalMinutes: 15 });
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });

  test("mail is read every 30 minutes and chat every 5 by default, and a default is changed on the page", async ({ page, request }) => {
    try {
      await visit(page, "/settings/lookout", "設定 · 見張り");
      await expect(page.getByRole("heading", { name: "既定の間隔" })).toBeVisible();
      await expect(page.getByLabel("メールの既定の間隔（分）")).toHaveValue("30");
      await expect(page.getByLabel("チャットの既定の間隔（分）")).toHaveValue("5");
      const other = page.getByLabel("ほかの既定の間隔（分）");
      await expect(other).toHaveValue("15");
      await other.fill("20");
      await other.press("Tab");
      await expect(toast(page)).toHaveText("既定の間隔を変えた");
      await page.reload();
      await expect(page.getByLabel("ほかの既定の間隔（分）")).toHaveValue("20");
    } finally {
      await call(request, "LookoutService", "SetLookoutDefault", { feed: "other", intervalMinutes: 15 });
    }
  });

  test("the page has no hand-written rules or interests, only the profile made from records and memory", async ({ page }) => {
    await visit(page, "/settings/lookout", "設定 · 見張り");
    await expect(page.getByRole("heading", { name: "判断の手がかり" })).toHaveCount(0);
    await page.getByText("いま判断に使っている手がかりを見る").click();
    await expect(page.locator(".lookout-profile")).toBeVisible();
    await expect(page.getByRole("heading", { name: "ふるいの決まり" })).toHaveCount(0);
    await expect(page.getByLabel(/見ずに落とす/)).toHaveCount(0);
    await expect(page.getByLabel(/関心/)).toHaveCount(0);
    await expect(page.locator("#pane textarea")).toHaveCount(0);
  });
});

/** An HTTP API connection of a kind: its tools are known without reaching it. */
async function addApiStandIn(request: APIRequestContext, name: string, kind: string): Promise<string> {
  const r = await call<Connected>(request, "ConnectionService", "AddConnection", {
    target: "http://127.0.0.1:9/",
    how: "api",
  });
  const id = r.connection.id;
  await call(request, "ConnectionService", "EditConnection", { id, name, kind });
  return id;
}

test.describe("サービス種ごとの道具の既定", () => {
  test("a permission set for a kind reaches every account of it unless the account has its own", async ({ page, request }) => {
    const work = await addApiStandIn(request, "仕事の API", "e2e-shared");
    const home = await addApiStandIn(request, "家の API", "e2e-shared");
    try {
      await visit(page, "/settings/services", "設定 · サービス");
      await page.locator("section").filter({ has: page.getByRole("heading", { name: "道具の権限の既定" }) }).getByRole("link", { name: /e2e-shared/ }).click();
      await expect(page).toHaveURL(/\/settings\/services\?kind=e2e-shared$/);
      await expectView(page, "設定 · サービス種 · e2e-shared");
      await expect(page.getByRole("heading", { name: "サービス種「e2e-shared」の既定" })).toBeVisible();
      await expect(page.getByRole("link", { name: "仕事の API" })).toBeVisible();
      await expect(page.getByRole("link", { name: "家の API" })).toBeVisible();

      const kindRow = page.locator(".list > .item").filter({ has: page.getByText("http_post", { exact: true }) });
      await expect(kindRow).not.toContainText("まだ決めていない");
      await kindRow.getByText("上書き", { exact: true }).click();
      await kindRow.getByText("拒否する", { exact: true }).click();
      await expect(toast(page)).toHaveText("保存した");
      await expect(kindRow.getByRole("radio", { name: "拒否する" })).toBeChecked();

      await call(request, "ConnectionService", "SetPermission", { scope: `conn:${home}`, tool: "http_post", permission: "PERMISSION_ALLOW" });

      await page.getByRole("link", { name: "仕事の API" }).click();
      await expectView(page, `設定 · サービス · ${work}`);
      const atWork = page.locator(".list > .item").filter({ has: page.getByText("http_post", { exact: true }) });
      await expect(atWork.getByRole("link", { name: "種類の既定" })).toBeVisible();
      await expect(atWork.getByRole("radio", { name: "既定（拒否する）" })).toBeChecked();

      await visit(page, `/settings/services?id=${home}`, `設定 · サービス · ${home}`);
      const atHome = page.locator(".list > .item").filter({ has: page.getByText("http_post", { exact: true }) });
      await expect(atHome).not.toContainText("このアカウント");
      await expect(atHome.getByRole("link", { name: "種類の既定" })).toHaveCount(0);
      await expect(atHome.getByRole("radio", { name: "許可する" })).toBeChecked();
      await expect(atHome.getByRole("radio", { name: "既定（拒否する）" })).not.toBeChecked();

      await page.getByRole("link", { name: "サービス種「e2e-shared」の既定を見る" }).click();
      await expect(page).toHaveURL(/\/settings\/services\?kind=e2e-shared$/);
      await expectView(page, "設定 · サービス種 · e2e-shared");
    } finally {
      await call(request, "ConnectionService", "SetPermission", { scope: "kind:e2e-shared", tool: "http_post", permission: "PERMISSION_UNSPECIFIED" });
      await call(request, "ConnectionService", "DeleteConnection", { id: work });
      await call(request, "ConnectionService", "DeleteConnection", { id: home });
    }
  });

  test("the bulk buttons of a kind set its writing tools at once and 既定に戻す clears them", async ({ page, request }) => {
    const id = await addApiStandIn(request, "まとめる API", "e2e-bulk");
    try {
      await visit(page, "/settings/services?kind=e2e-bulk", "設定 · サービス種 · e2e-bulk");
      const bulk = page.locator(".row").filter({ hasText: "書く・送る道具を" });
      await bulk.getByRole("button", { name: "確認する" }).click();
      await expect(toast(page)).toHaveText("保存した");
      for (const tool of ["http_post", "http_delete"]) {
        const row = page.locator(".list > .item").filter({ has: page.getByText(tool, { exact: true }) });
        await expect(row.getByRole("radio", { name: "確認する" })).toBeChecked();
      }
      const get = page.locator(".list > .item").filter({ has: page.getByText("http_get", { exact: true }) });
      await expect(get.getByRole("radio", { name: "既定（まかせる）" })).toBeChecked();

      await bulk.getByRole("button", { name: "既定に戻す" }).click();
      await expect(toast(page)).toHaveText("保存した");
      const kind = await call<{ tools: { permission?: string }[] }>(request, "ConnectionService", "GetKind", { id: "e2e-bulk" });
      expect(kind.tools.filter((t) => t.permission)).toEqual([]);
    } finally {
      await call(request, "ConnectionService", "DeleteConnection", { id });
    }
  });
});
