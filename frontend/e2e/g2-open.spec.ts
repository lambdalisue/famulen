import { expect, type Locator, type Page, test } from "@playwright/test";
import jsQR from "jsqr";
import { reachesOnlyThisMachine } from "~/lib/phone";
import { visit } from "./support";

// A name a phone could be given, which this browser finds on this machine.
const AWAY = "http://famulen.test:13000";

test.use({ launchOptions: { args: ["--host-resolver-rules=MAP famulen.test 127.0.0.1"] } });

async function open(page: Page, origin = "") {
  await visit(page, `${origin}/settings/channels`, "設定 · チャネル");
  await page.getByRole("button", { name: "QR コード" }).click();
  return page.getByRole("dialog", { name: "Even G2 で開く" });
}

/** What a camera pointed at the picture on screen reads. */
async function scan(page: Page, picture: Locator): Promise<string | undefined> {
  const png = (await picture.screenshot()).toString("base64");
  const { data, width, height } = await page.evaluate(async (src) => {
    const image = new Image();
    image.src = src;
    await image.decode();
    const canvas = document.createElement("canvas");
    canvas.width = image.width;
    canvas.height = image.height;
    const context = canvas.getContext("2d") as CanvasRenderingContext2D;
    context.drawImage(image, 0, 0);
    const pixels = context.getImageData(0, 0, image.width, image.height);
    return { data: Array.from(pixels.data), width: pixels.width, height: pixels.height };
  }, `data:image/png;base64,${png}`);
  return jsQR(Uint8ClampedArray.from(data), width, height)?.data;
}

for (const colorScheme of ["light", "dark"] as const) {
  test(`the QR code is the address of the page for the glasses (${colorScheme})`, async ({ page }) => {
    await page.emulateMedia({ colorScheme });
    const asked: string[] = [];
    page.on("request", (r) => asked.push(r.url()));

    const dialog = await open(page, AWAY);
    const picture = dialog.getByRole("img", { name: `${AWAY}/g2 の QR コード` });

    expect(await scan(page, picture)).toBe(`${AWAY}/g2`);
    // No service outside is told the address.
    expect(asked.filter((url) => url.startsWith("http") && !url.startsWith(`${AWAY}/`))).toEqual([]);
    await expect(dialog.getByRole("link", { name: "hub.evenrealities.com/login" })).toHaveAttribute(
      "href",
      "https://hub.evenrealities.com/login",
    );
    await expect(dialog).toContainText("Scan QR");
  });
}

test("a console opened at localhost shows no QR code, but what to do", async ({ page }) => {
  await page.goto("/settings/channels");
  await page.evaluate(() => location.replace(location.href.replace("127.0.0.1", "localhost")));
  await page.waitForURL(/localhost/);
  const dialog = await open(page, "http://localhost:13000");

  await expect(dialog).toContainText("スマートフォンからは届きません");
  await expect(dialog).toContainText("FAMULEN_CONSOLE_URL");
  await expect(dialog.getByRole("img")).toHaveCount(0);
  await expect(dialog).toContainText("Scan QR");
});

test("which host names reach only the machine they are opened on", () => {
  for (const host of ["localhost", "LOCALHOST", "localhost.", "famulen.localhost", "127.0.0.1", "127.1.2.3", "0.0.0.0", "[::1]", "[::]"]) {
    expect(reachesOnlyThisMachine(host), host).toBe(true);
  }
  for (const host of ["famulen.test", "mac.tail1234.ts.net", "192.168.1.20", "100.64.0.1", "localhost.example.com", "127.example.com", "[fd7a:115c:a1e0::1]"]) {
    expect(reachesOnlyThisMachine(host), host).toBe(false);
  }
});
