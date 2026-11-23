import { expect, test } from "@playwright/test";
import { call, toast, visit } from "./support";

const BACKEND = "http://127.0.0.1:17790";

type Detail = { tools?: { name: string; effective?: string; alwaysAsk?: boolean }[] };
type Text = { toml: string };

const REMOVE = `
[[tools]]
name = "remove_note"
description = "Remove one note."
method = "DELETE"
path = "/notes/{/id}"
`;

test("the secretary's change to a definition is read as a diff, let in on the approval board, and taken back", async ({
  page,
  request,
}) => {
  const added = await call<{ connection: { id: string } }>(request, "ConnectionService", "AddConnection", {
    target: "e2e-notes",
    how: "definition",
  });
  const id = added.connection.id;
  try {
    const before = await call<Text>(request, "ConnectionService", "GetDefinitionText", { id: "e2e-notes" });
    const proposed = await request.post(`${BACKEND}/famulen.v1.ConnectionService/ProposeDefinitionChange`, {
      headers: { "famulen-caller": "secretary" },
      data: { definition: "e2e-notes", toml: before.toml + REMOVE, why: "メモを消す道具がない" },
    });
    expect(await proposed.text()).toContain("Held for the owner's approval");

    await visit(page, "/", /^ホーム/);
    const card = page.locator('[data-testid="approval"][data-kind="definition"]');
    await expect(card).toContainText("e2e-notes");
    await expect(card).toContainText("メモを消す道具がない");
    const diff = card.getByTestId("definition-diff");
    await expect(diff.locator('[data-kind="add"]').filter({ hasText: 'name = "remove_note"' })).toBeVisible();
    await expect(diff.locator('[data-kind="del"]')).toHaveCount(0);
    expect((await call<Text>(request, "ConnectionService", "GetDefinitionText", { id: "e2e-notes" })).toml).toBe(
      before.toml,
    );

    await card.getByRole("button", { name: "許す" }).click();
    await expect(card).toHaveCount(0);
    await expect
      .poll(async () => (await call<Detail>(request, "ConnectionService", "GetConnection", { id })).tools ?? [])
      .toContainEqual(expect.objectContaining({ name: "remove_note", effective: "PERMISSION_AUTO", alwaysAsk: true }));

    await visit(page, "/settings/services", "設定 · サービス");
    const changes = page.getByTestId("definition-changes").filter({ hasText: "e2e-notes" });
    await changes.getByRole("button", { name: /メモを消す道具がない/ }).click();
    await expect(changes.locator('[data-kind="add"]').filter({ hasText: 'name = "remove_note"' })).toBeVisible();
    await changes.getByRole("button", { name: "e2e-notesを一つ前に戻す" }).click();
    await expect(toast(page)).toHaveText("一つ前に戻した");
    await expect(changes).toContainText("戻した");
    expect((await call<Text>(request, "ConnectionService", "GetDefinitionText", { id: "e2e-notes" })).toml).toBe(
      before.toml,
    );
  } finally {
    await call(request, "ConnectionService", "DeleteConnection", { id });
  }
});
