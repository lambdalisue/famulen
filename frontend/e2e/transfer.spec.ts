import { expect, test } from "@playwright/test";
import { call, visit } from "./support";

// A phone reaches the console over a slow line, so what it is sent is
// compressed, and a stream still arrives as it is said.

const GZIP = { "accept-encoding": "gzip" };

test("a page and what the browser reads through the console come compressed", async ({ request, baseURL }) => {
  for (let i = 0; i < 5; i++) await call(request, "RecordService", "SaveRecord", { kind: "memos", title: `e2e-圧縮 ${i}`, notes: "長い本文。".repeat(200) });

  const page = await request.get("/memos", { headers: GZIP });
  expect(page.headers()["content-encoding"]).toBe("gzip");
  expect(await page.text()).toContain("e2e-圧縮 4");

  const read = await request.post("/famulen.v1.RecordService/ListRecords", { headers: { ...GZIP, "content-type": "application/json", origin: baseURL ?? "" }, data: { kind: "memos" } });
  expect(read.headers()["content-encoding"]).toBe("gzip");
  expect(await read.text()).toContain("e2e-圧縮 4");

  const script = (await page.text()).match(/\/_build\/assets\/[^"]+\.js/)?.[0];
  expect(script).toBeTruthy();
  const asset = await request.get(script ?? "", { headers: GZIP });
  expect(asset.headers()["content-encoding"]).toBe("gzip");
});

test("a page that is not there answers the same compressed or not", async ({ request }) => {
  const plain = await request.get("/e2e-無い頁", { headers: { "accept-encoding": "identity" } });
  const packed = await request.get("/e2e-無い頁", { headers: GZIP });

  expect(packed.status()).toBe(plain.status());
  expect(packed.headers()["content-type"]).toBe(plain.headers()["content-type"]);
  expect(packed.headers()["content-type"]).toContain("text/html");
  expect(await packed.text()).toContain("<html");
});

test("a message said while the page is open still arrives at once", async ({ page, request }) => {
  await visit(page, "/memos", /^メモ/);
  await call(request, "ConsoleService", "PostMessage", { text: "e2e-圧縮 届く" });
  await expect(page.getByTestId("message").filter({ hasText: "e2e-圧縮 届く" })).toBeVisible({ timeout: 5000 });
});
