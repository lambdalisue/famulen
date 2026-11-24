import { type BrowserContext, expect, type Page, test, type Worker } from "@playwright/test";
import { call, toast, visit } from "./support";

type Notices = { devices?: { id: string; name: string; here?: boolean }[]; detail?: string; key?: string };
type Inbox = { approvals?: { id: string; status: string }[] };

const notices = (request: Parameters<typeof call>[0]) => call<Notices>(request, "NoticeService", "GetNotices");

/** A headless browser refuses notices and has no push service behind it, so both are stood in for. */
async function standInPushService(page: Page) {
  await page.addInitScript(() => {
    let made: PushSubscription | null = null;
    const keys = {
      p256dh: "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
      auth: "BTBZMqHH6r4Tts7J_aSIgg",
    };
    Object.defineProperty(Notification, "permission", { get: () => "granted" });
    Notification.requestPermission = async () => "granted";
    const endpoint = "https://push.example.invalid/e2e";
    PushManager.prototype.getSubscription = async () => made;
    PushManager.prototype.subscribe = async (options) => {
      const key = String(options?.applicationServerKey);
      if (key.length !== 87) throw new Error(`not a key: ${key}`);
      made = Object.create(PushSubscription.prototype, {
        endpoint: { value: endpoint },
        options: { value: {} },
        toJSON: { value: () => ({ endpoint, keys }) },
        unsubscribe: {
          value: async () => {
            made = null;
            return true;
          },
        },
      }) as PushSubscription;
      return made;
    };
  });
}

async function worker(context: BrowserContext): Promise<Worker> {
  return context.serviceWorkers()[0] ?? context.waitForEvent("serviceworker");
}

/** Press the notice of a request, as the worker is told of it. */
async function press(context: BrowserContext, confirm: string) {
  await (await worker(context)).evaluate(async (id) => {
    const scope: object = self;
    // Only a real press lets the worker bring the window to the front.
    await Reflect.get(scope, "open")(id).catch(() => {});
  }, confirm);
}

async function ask(page: Page, request: Parameters<typeof call>[0]): Promise<string> {
  await page.getByPlaceholder(/に頼む$/).fill("e2e-bash で ls して");
  await page.getByRole("button", { name: "送る", exact: true }).click();
  const pending = async () =>
    (await call<Inbox>(request, "ConsoleService", "GetInbox")).approvals?.find((a) => a.status === "pending")?.id;
  await expect.poll(pending).toBeTruthy();
  return (await pending()) ?? "";
}

const card = (page: Page, id: string) => page.locator(`#approval-${id}`);

test.afterEach(async ({ request }) => {
  for (const d of (await notices(request)).devices ?? []) {
    await call(request, "NoticeService", "RemoveNoticeDevice", { id: d.id });
  }
  await call(request, "NoticeService", "SetNoticeDetail", { detail: "NOTICE_DETAIL_DEED" });
  for (const a of (await call<Inbox>(request, "ConsoleService", "GetInbox")).approvals ?? []) {
    if (a.status === "pending") await call(request, "ConsoleService", "AnswerApproval", { id: a.id, answer: "APPROVAL_ANSWER_DENY" });
  }
});

test("the owner takes notices on this browser, and stops", async ({ page, request }) => {
  await standInPushService(page);
  await visit(page, "/settings/notices", "設定 · 通知");
  const here = page.getByRole("region", { name: "この端末" });
  await here.getByRole("button", { name: "この端末で受け取る" }).click();
  await expect(toast(page)).toHaveText("受け取るようにした");
  const listed = page.getByRole("list", { name: "受け取る端末" }).getByRole("listitem");
  await expect(listed).toHaveCount(1);
  await expect(listed).toContainText("Chrome");
  await expect(listed).toContainText("この端末");
  expect((await notices(request)).devices).toHaveLength(1);

  await here.getByRole("button", { name: "受け取るのをやめる" }).click();
  await expect(here.getByRole("button", { name: "この端末で受け取る" })).toBeVisible();
  expect((await notices(request)).devices ?? []).toHaveLength(0);
});

test("a browser whose subscription was dropped is told once", async ({ page, request }) => {
  await standInPushService(page);
  await visit(page, "/settings/notices", "設定 · 通知");
  await page.getByRole("button", { name: "この端末で受け取る" }).click();
  await expect(toast(page)).toHaveText("受け取るようにした");
  for (const d of (await notices(request)).devices ?? []) await call(request, "NoticeService", "RemoveNoticeDevice", { id: d.id });
  await visit(page, "/", /^ホーム/);
  await expect(toast(page)).toContainText("通知が届かなくなった");
  await visit(page, "/tasks", /^タスク/);
  await expect(page.locator("#composer .view")).toBeVisible();
  await expect(toast(page)).toHaveCount(0);
});

test("a browser whose notices are refused is told so", async ({ page }) => {
  await page.addInitScript(() => Object.defineProperty(Notification, "permission", { get: () => "denied" }));
  await visit(page, "/settings/notices", "設定 · 通知");
  await expect(page.getByText(/通知が拒否されている/)).toBeVisible();
  await expect(page.getByRole("button", { name: "この端末で受け取る" })).toHaveCount(0);
});

test("the owner chooses how much a notice tells", async ({ page, request }) => {
  await visit(page, "/settings/notices", "設定 · 通知");
  const choices = page.getByRole("radiogroup", { name: "通知に載せること" });
  await expect(choices.getByRole("radio", { name: /何をするか/ })).toBeChecked();
  await choices.getByText("番号だけ", { exact: true }).click();
  await expect(toast(page)).toHaveText("保存した");
  await page.reload();
  await expect(choices.getByRole("radio", { name: /番号だけ/ })).toBeChecked();
  expect((await notices(request)).detail).toBe("NOTICE_DETAIL_NUMBER");
});

test("a push is shown as a notice, even one that does not read", async ({ page, context }) => {
  await standInPushService(page);
  await visit(page, "/settings/notices", "設定 · 通知");
  await page.getByRole("button", { name: "この端末で受け取る" }).click();
  await expect(toast(page)).toHaveText("受け取るようにした");
  const sw = await worker(context);
  const shown = (data: string) =>
    sw.evaluate(async (text) => {
      const scope: object = self;
      const seen: unknown[] = [];
      const registration = Reflect.get(scope, "registration");
      registration.showNotification = async (title: string, options: object) => void seen.push({ title, ...options });
      const event: Event = Reflect.construct(Reflect.get(scope, "PushEvent"), ["push", { data: text }]);
      self.dispatchEvent(event);
      await new Promise((done) => setTimeout(done, 50));
      return seen;
    }, data);
  expect(await shown(JSON.stringify({ title: "確認 #7", body: "読む", tag: "confirm-7", confirm: 7, icon: "/i.png" }))).toEqual([
    { title: "確認 #7", body: "読む", tag: "confirm-7", icon: "/i.png", lang: "ja", data: { confirm: 7 } },
  ]);
  expect(await shown("not json")).toHaveLength(1);
});

test("a notice pressed while the console is open shows its request there, to be answered", async ({ page, context, request }) => {
  await standInPushService(page);
  await visit(page, "/settings/notices", "設定 · 通知");
  await page.getByRole("button", { name: "この端末で受け取る" }).click();
  await expect(toast(page)).toHaveText("受け取るようにした");
  const id = await ask(page, request);
  await press(context, id);
  await expect(card(page, id)).toBeFocused();
  await expect(page).toHaveURL(/\/settings\/notices$/);
  await card(page, id).getByRole("button", { name: "却下" }).click();
  await expect(card(page, id)).toHaveCount(0);

  await press(context, id);
  await expect(toast(page)).toHaveText(`確認 #${id} は、答えを待つものの中にない`);
});

test("the address a notice opens shows its request once", async ({ page, request }) => {
  await visit(page, "/", /^ホーム/);
  const id = await ask(page, request);
  await page.goto(`/?confirm=${id}`);
  await expect(card(page, id)).toBeFocused();
  await expect(page).toHaveURL(/\/$/);
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test("the address a notice opens pulls the conversation up to its request", async ({ page, request }) => {
    await visit(page, "/", /^ホーム/);
    const id = await ask(page, request);
    await page.goto(`/?confirm=${id}`);
    await expect(page.locator("body")).toHaveClass(/chat-open/);
    await expect(card(page, id).getByRole("button", { name: "承認" })).toBeVisible();
  });
});
