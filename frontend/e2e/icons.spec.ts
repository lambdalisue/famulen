import { readFileSync } from "node:fs";
import { expect, type Page, test } from "@playwright/test";
import { call, toast, visit } from "./support";

const TYPES: Record<string, string> = {
  "app-icon.svg": "image/svg+xml",
  "app-icon-192.png": "image/png",
  "app-icon-512.png": "image/png",
  "app-icon-maskable-512.png": "image/png",
  "apple-touch-icon.png": "image/png",
  "avatar.svg": "image/svg+xml",
  "favicon-16.png": "image/png",
  "favicon-32.png": "image/png",
  "favicon.ico": "image/vnd.microsoft.icon",
};

/** Every icon of the page: the links of its head and the icon in the bar. */
async function expectPortrait(page: Page, portrait: "male" | "female") {
  const links = page.locator('head link:is([rel="icon"], [rel="apple-touch-icon"], [rel="manifest"])');
  await expect
    .poll(async () => [...new Set(await links.evaluateAll((all) => all.map((l) => l.getAttribute("href")?.split("/")[2])))])
    .toEqual([portrait]);
  for (const [rel, count] of [["icon", 4], ["apple-touch-icon", 1], ["manifest", 1]] as const) {
    await expect(page.locator(`head link[rel="${rel}"]`)).toHaveCount(count);
  }
  await expect(page.locator(".logo img")).toHaveAttribute("src", `/icons/${portrait}/app-icon.svg`);
}

test.describe("the secretary's portrait", () => {
  test.afterEach(async ({ request }) => {
    await call(request, "SettingsService", "UpdatePersona", { gender: "PERSONA_GENDER_MALE" });
  });

  test("the portrait chosen changes every icon at once, in other tabs too, and is kept", async ({ page, context }) => {
    const other = await context.newPage();
    await visit(other, "/", /^ホーム/);
    await visit(page, "/settings", "設定 · 秘書");
    await expectPortrait(page, "male");
    await expectPortrait(other, "male");

    const choices = page.getByRole("radiogroup", { name: "秘書の性別" });
    await expect(choices.getByRole("radio", { name: /男性/ })).toBeChecked();
    await choices.getByText("女性", { exact: true }).click();
    await expect(toast(page)).toHaveText("保存した");

    await expectPortrait(page, "female");
    await expectPortrait(other, "female");
    await page.reload();
    await expect(choices.getByRole("radio", { name: /女性/ })).toBeChecked();
    await expectPortrait(page, "female");
  });

  test("the keyboard chooses too", async ({ page }) => {
    await visit(page, "/settings", "設定 · 秘書");
    await page.getByRole("radio", { name: /男性/ }).focus();
    await page.keyboard.press("ArrowRight");
    await expect(toast(page)).toHaveText("保存した");
    await expectPortrait(page, "female");
  });

  test("the page as the server sends it already names the chosen icons in its head", async ({ request }) => {
    await call(request, "SettingsService", "UpdatePersona", { gender: "PERSONA_GENDER_FEMALE" });
    for (const path of ["/", "/settings", "/tasks"]) {
      const head = (await (await request.get(path)).text()).split("</head>")[0];
      for (const file of ["manifest.webmanifest", "apple-touch-icon.png", "favicon.ico", "favicon-16.png", "favicon-32.png", "app-icon.svg"]) {
        expect(head.split(`/icons/female/${file}"`).length - 1, `${path} ${file}`).toBe(1);
      }
      expect(head).not.toContain("/icons/male/");
    }
  });

  test("an icon stands by what the secretary says, in the chat and in the bar", async ({ page, request }) => {
    await call(request, "ConsoleService", "PostMessage", { text: "e2e: 秘書の一言" });
    await visit(page, "/", /^ホーム/);
    const said = page.getByTestId("message").filter({ hasText: "e2e: 秘書の一言" });
    await expect(said.locator("img")).toHaveAttribute("src", "/icons/male/app-icon.svg");
  });
});

test("the icons served are the brand's, each with its own type", async ({ request }) => {
  for (const portrait of ["male", "female"]) {
    for (const [file, type] of Object.entries(TYPES)) {
      const answer = await request.get(`/icons/${portrait}/${file}`);
      expect(answer.ok(), file).toBe(true);
      expect(answer.headers()["content-type"], file).toBe(type);
      expect((await answer.body()).equals(readFileSync(`../brand/${portrait}/${file}`)), file).toBe(true);
    }
  }
});
