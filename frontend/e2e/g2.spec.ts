import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { type APIRequestContext, expect, type Page, test } from "@playwright/test";
import type { Input } from "~/g2/even";
import { call, record } from "./support";

type Shown = { name: string; level: number; text: string };

/** Glasses that remember what they were told to show, and take what the test does on them. */
type FakeGlasses = {
  boxes: Shown[];
  rebuilds: number;
  microphone: boolean;
  left: number;
  emit(input: Input): void;
};

declare global {
  interface Window {
    fake: FakeGlasses;
  }
}

/** What the voice's model hears next; it hears nothing else. */
const voice = { next: "" };
let endpoint = "";

test.beforeAll(async () => {
  const server = createServer((req, res) => {
    req.resume();
    req.on("end", () => {
      res.setHeader("content-type", "application/json");
      res.end(JSON.stringify({ text: voice.next }));
    });
  });
  await new Promise<void>((done) => server.listen(0, "127.0.0.1", done));
  endpoint = `http://127.0.0.1:${(server.address() as AddressInfo).port}/v1`;
});

async function wear(page: Page) {
  await page.addInitScript(() => {
    let listener: (input: Input) => void = () => {};
    type Layout = { textObject: { containerName: string; textColor: number; content: string }[] };
    const fake: FakeGlasses = { boxes: [], rebuilds: 0, microphone: false, left: 0, emit: (input) => listener(input) };
    const put = (layout: Layout) => {
      fake.rebuilds++;
      fake.boxes = layout.textObject.map((t) => ({ name: t.containerName, level: t.textColor, text: t.content }));
      return Promise.resolve(true);
    };
    window.fake = fake;
    window.famulenGlasses = {
      create: put,
      rebuild: put,
      text: (_id: number, name: string, content: string) => {
        const box = fake.boxes.find((b) => b.name === name);
        if (box) box.text = content;
        return Promise.resolve(true);
      },
      image: () => Promise.resolve(true),
      microphone: (on: boolean) => {
        fake.microphone = on;
        return Promise.resolve(true);
      },
      leave: () => {
        fake.left++;
        return Promise.resolve();
      },
      onInput: (l: (input: Input) => void) => {
        listener = l;
      },
    };
  });
  await page.goto("/g2");
  await expect(page.getByTestId("g2-state")).toHaveText("眼鏡に出している");
}

/** Everything on the glasses' screen, box by box. */
const screen = (page: Page) => page.evaluate(() => window.fake.boxes.map((b) => b.text).join("\n"));
/** The brightest words on the glasses' screen below its top. */
const bright = (page: Page) =>
  page.evaluate(() =>
    window.fake.boxes
      .filter((b) => b.level === 4)
      .map((b) => b.text)
      .join("\n"),
  );

type Gesture = "up" | "down" | "tap" | "double" | "long";

const act = (page: Page, kind: Gesture) => page.evaluate((kind) => window.fake.emit({ kind }), kind);

/** Speak for `loud` milliseconds, then keep quiet for `quiet`, in the glasses' frames of 100 ms. */
async function speak(page: Page, loud: number, quiet: number) {
  await page.evaluate(
    ({ loud, quiet }) => {
      const frame = (on: boolean) => {
        const pcm = new Uint8Array(3200);
        const view = new DataView(pcm.buffer);
        for (let i = 0; i < 1600; i++) view.setInt16(i * 2, on ? Math.round(Math.sin(i / 4) * 9000) : 0, true);
        return pcm;
      };
      for (let t = 0; t < loud; t += 100) window.fake.emit({ kind: "sound", pcm: frame(true) });
      for (let t = 0; t < quiet; t += 100) window.fake.emit({ kind: "sound", pcm: frame(false) });
    },
    { loud, quiet },
  );
}

async function giveTheVoiceAModel(request: APIRequestContext) {
  const model = await call<{ id: string }>(request, "ModelService", "RegisterModel", {
    name: "e2e whisper",
    kind: "MODEL_KIND_TRANSCRIBE",
    model: "whisper-1",
    endpoint,
  });
  await call(request, "ModelService", "AssignRole", { role: "ROLE_VOICE", model: model.id });
}

type Message = { text: string; origin?: string; who?: string; id: string; conversation?: string };

async function messages(request: APIRequestContext): Promise<Message[]> {
  return (await call<{ messages?: Message[] }>(request, "ConsoleService", "ListMessages", { workgroup: "all", limit: 50 })).messages ?? [];
}

test.describe.configure({ mode: "serial" });

test("speaking says why nothing can be heard while the voice has no model", async ({ page }) => {
  await wear(page);
  await act(page, "tap");
  await expect.poll(() => page.evaluate(() => window.fake.microphone)).toBe(true);
  await speak(page, 800, 1600);
  await expect.poll(() => screen(page)).toContain("声の聞き取りのモデルが選ばれていない");
  expect(await page.evaluate(() => window.fake.microphone)).toBe(false);
});

test("what is said aloud reaches the secretary from the Even G2, and its answer is shown bright", async ({ page, request }) => {
  await giveTheVoiceAModel(request);
  await wear(page);
  // A new conversation, from the menu's list of them.
  await act(page, "long");
  await expect.poll(() => bright(page)).toContain("会話");
  await act(page, "tap");
  await expect.poll(() => bright(page)).toContain("新しい話");
  await act(page, "tap");

  voice.next = "e2e 眼鏡 15 分遅れるって伝えて";
  await act(page, "tap");
  await expect.poll(() => page.evaluate(() => window.fake.microphone)).toBe(true);
  await speak(page, 2500, 1600);
  await expect.poll(async () => (await messages(request)).find((m) => m.text === voice.next)?.origin).toBe("Even G2");
  expect(await page.evaluate(() => window.fake.microphone)).toBe(false);
  await expect.poll(() => screen(page)).toContain(voice.next);

  const asked = (await messages(request)).find((m) => m.text === voice.next);
  await call(request, "ConsoleService", "PostMessage", { text: "e2e 眼鏡 先方に伝えました", replyTo: asked?.id });
  await expect.poll(() => bright(page)).toContain("e2e 眼鏡 先方に伝えました");
});

test("nothing said is sent nowhere, and a double tap stops listening", async ({ page, request }) => {
  await wear(page);
  const before = (await messages(request)).length;
  await act(page, "tap");
  await expect.poll(() => page.evaluate(() => window.fake.microphone)).toBe(true);
  await act(page, "double");
  await expect.poll(() => page.evaluate(() => window.fake.microphone)).toBe(false);
  expect((await messages(request)).length).toBe(before);
  expect(await page.evaluate(() => window.fake.left)).toBe(0);
});

test("the menu shows what each item holds beside the list, and a double tap goes back and then asks to leave", async ({ page, request }) => {
  await record(request, "tasks", "e2e 眼鏡 請求書を締めて送る", { fields: { Status: "pending" } });
  await wear(page);
  await act(page, "long");
  await act(page, "down");
  await act(page, "down");
  await expect.poll(() => bright(page)).toContain("タスク");
  await expect.poll(() => screen(page)).toContain("e2e 眼鏡 請求書");
  await act(page, "double");
  await expect.poll(() => bright(page)).not.toContain("タスク");
  await act(page, "double");
  await expect.poll(() => page.evaluate(() => window.fake.left)).toBe(1);
});

test("a task is finished from the glasses", async ({ page, request }) => {
  const task = await record(request, "tasks", "e2e 眼鏡 挨拶状の宛先をまとめる", { fields: { Status: "pending" } });
  await wear(page);
  await act(page, "long");
  await act(page, "down");
  await act(page, "down");
  await act(page, "tap");
  for (let i = 0; i < 20 && !(await bright(page)).includes("e2e 眼鏡 挨拶状の宛先をまとめる"); i++) await act(page, "down");
  await act(page, "tap");
  await expect.poll(() => bright(page)).toContain("済にする");
  await act(page, "tap");
  await expect
    .poll(async () => {
      const got = await call<{ fields?: { name: string; value: string }[] }>(request, "RecordService", "GetRecord", { kind: "tasks", id: task.id });
      return got.fields?.find((f) => f.name === "Status")?.value;
    })
    .toBe("done");
});

test("what waits for an answer is shown whole and answered from the menu's 確認", async ({ page, request }) => {
  await call(request, "ConsoleService", "Send", { text: "e2e 眼鏡 e2e-asks", workgroup: "all", newConversation: true });
  type Inbox = { approvals?: { id: string; status: string }[] };
  const pending = async () => (await call<Inbox>(request, "ConsoleService", "GetInbox")).approvals?.find((a) => a.status === "pending")?.id;
  await expect.poll(pending, { timeout: 20_000 }).toBeTruthy();
  const id = await pending();
  await wear(page);
  await act(page, "long");
  for (let i = 0; i < 3; i++) await act(page, "down");
  await expect.poll(() => bright(page)).toContain("確認");
  await act(page, "tap");
  await act(page, "tap");
  await expect.poll(() => screen(page)).toContain("ls");
  await expect.poll(() => bright(page)).toContain("許す");
  // Allowed, the call would be let through once more for whichever test asks next.
  await act(page, "down");
  await expect.poll(() => bright(page)).toContain("やめる");
  await act(page, "tap");
  await expect.poll(async () => (await call<Inbox>(request, "ConsoleService", "GetInbox")).approvals?.find((a) => a.id === id)?.status ?? "answered").not.toBe("pending");
});
