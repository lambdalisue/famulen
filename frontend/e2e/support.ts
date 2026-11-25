import { type APIRequestContext, expect, type Locator, type Page } from "@playwright/test";

const BACKEND = "http://127.0.0.1:17790";

/** Call the backend directly, as Connect JSON, to set up what a page has no control for. */
export async function call<T = Record<string, unknown>>(
  request: APIRequestContext,
  service: string,
  method: string,
  data: Record<string, unknown> = {},
): Promise<T> {
  const res = await request.post(`${BACKEND}/famulen.v1.${service}/${method}`, { data });
  expect(res.ok(), `${service}/${method}: ${await res.text()}`).toBe(true);
  return (await res.json()) as T;
}

/**
 * Open a page and wait until the browser has taken it over. Each page names
 * itself in the composer's view line only once its code runs in the browser,
 * so the line is the sign that its controls answer.
 */
export async function visit(page: Page, path: string, view: string | RegExp) {
  await page.goto(path);
  await expectView(page, view);
}

export async function expectView(page: Page, view: string | RegExp) {
  await expect(page.locator("#composer .view")).toHaveText(view);
}

/**
 * Come to a page by its tab, so that the browser and not the server reads what
 * it shows: what a test answers in the backend's place reaches it only so. The
 * tabs are there on a wide screen.
 */
export async function throughTab(page: Page, path: string) {
  const size = page.viewportSize();
  await page.setViewportSize({ width: 1440, height: 900 });
  await visit(page, "/memos", /^メモ/);
  await page.locator(`a[href="${path}"]:visible`).first().click();
  await page.waitForURL(`**${path}`);
  if (size) await page.setViewportSize(size);
}

/** Open an agent's log from the button at the foot of its settings page. */
export async function openLog(page: Page, path: string, view: string | RegExp, title: string, button = "ログを見る") {
  await visit(page, path, view);
  await page.getByRole("button", { name: button }).click();
  const sheet = page.getByRole("dialog", { name: title });
  await expect(sheet).toBeVisible();
  return sheet;
}

/** The note at the bottom of the screen after a change. */
export function toast(page: Page) {
  return page.locator("#toast");
}

/** Answer yes to what the page asks before it removes something. */
export async function agree(page: Page, verb: string, about?: string | RegExp) {
  const asked = page.getByRole("alertdialog", about ? { name: about } : {});
  await expect(asked).toBeVisible();
  await asked.getByRole("button", { name: verb, exact: true }).click();
  await expect(asked).toHaveCount(0);
}

/** The pane beside the list, or beside the calendar. */
export function pane(page: Page, name: string | RegExp) {
  return page.getByRole("region", { name });
}

/** YYYY-MM-DD, `days` from today in Asia/Tokyo, the zone the test backend runs in. */
export function dayFromToday(days: number): string {
  const today = new Intl.DateTimeFormat("sv-SE", { timeZone: "Asia/Tokyo" }).format(new Date());
  const d = new Date(`${today}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

type Given = {
  notes?: string;
  labels?: string[];
  fields?: { [name: string]: string };
  sections?: { name: string; fields?: { name: string; value: string }[]; items?: { text: string; fields?: { name: string; value: string }[] }[] }[];
};

/** Add a record straight through the API. */
export function record(request: APIRequestContext, kind: string, title: string, given: Given = {}) {
  const fields = Object.entries(given.fields ?? {}).map(([name, value]) => ({ name, value }));
  return call<{ id: string; code: string }>(request, "RecordService", "SaveRecord", { kind, title, ...given, fields });
}

/** A record in the list. */
export function row(page: Page, title: string) {
  return page.locator("[data-record]").filter({ hasText: title });
}

/** What is open beside the list. */
export function detail(page: Page) {
  return page.locator("article.rec");
}

/** A field of what is open, as it is read. */
export function field(page: Page, name: string) {
  return detail(page).locator(".rec-field").filter({ has: page.locator("dt", { hasText: new RegExp(`^${name}$`) }) });
}

/** Say something along with an image of this size, drawn in a page. */
export async function sayImage(page: Page, request: APIRequestContext, text: string, name: string, size: { width: number; height: number }) {
  const drawn = await page.evaluate(({ width, height }) => {
    const canvas = document.createElement("canvas");
    Object.assign(canvas, { width, height });
    const pen = canvas.getContext("2d");
    if (pen) {
      pen.fillStyle = "#2a78d6";
      pen.fillRect(0, 0, width, height);
    }
    return canvas.toDataURL("image/png").split(",")[1];
  }, size);
  const res = await request.post(`${BACKEND}/attachments?name=${encodeURIComponent(name)}`, {
    data: Buffer.from(drawn, "base64"),
    headers: { "content-type": "application/octet-stream" },
  });
  expect(res.ok(), await res.text()).toBe(true);
  const { id } = (await res.json()) as { id: string };
  await call(request, "ConsoleService", "Send", { text, workgroup: "all", attachments: [id] });
  await expect(page.getByTestId("message").filter({ hasText: text })).toBeAttached();
  // What the secretary is doing takes room at the end of the conversation until it is done.
  await expect.poll(async () => (await call<{ state: string }>(request, "ConsoleService", "GetStatus")).state).not.toBe("running");
  await expect(page.getByTestId("working")).toHaveCount(0);
}

/** Whether an element has a size and all of it is on the screen. */
export async function whollyOnScreen(page: Page, element: Locator) {
  const box = await element.boundingBox();
  const screen = page.viewportSize();
  if (!box || !screen) return false;
  return box.width > 0 && box.height > 0 && box.x >= 0 && box.y >= 0 && box.x + box.width <= screen.width && box.y + box.height <= screen.height;
}
