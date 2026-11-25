import { expect, test } from "@playwright/test";
import { call, expectView, toast, visit } from "./support";

type Workspaces = { attentionMinutes?: number };

test("the owner sets on the triage page how long a channel is attended after a mention", async ({ page, request }) => {
  try {
    await visit(page, "/settings/channels", "設定 · チャネル");
    await expect(page.getByLabel("注視する分")).toHaveCount(0);
    await page.locator(".settings-page").getByRole("link", { name: "トリアージ" }).click();
    await expectView(page, "設定 · トリアージ");
    const minutes = page.getByLabel("注視する分");
    await expect(minutes).toHaveValue("60");

    await minutes.fill("0");
    await minutes.blur();
    await expect(toast(page)).toHaveText("保存した");
    await expect
      .poll(async () => (await call<Workspaces>(request, "ChannelService", "ListWorkspaces")).attentionMinutes ?? 0)
      .toBe(0);

    await page.reload();
    await expect(page.getByLabel("注視する分")).toHaveValue("0");
  } finally {
    await call(request, "ChannelService", "SetAttention", { minutes: 60 });
  }
});
