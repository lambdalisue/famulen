import { reread } from "./remember";
import { createSignal } from "solid-js";

const [toastText, setToastText] = createSignal<{ text: string; bad: boolean } | undefined>();
let timer: ReturnType<typeof setTimeout> | undefined;

/** The short note at the bottom of the screen after a save. */
export const toast = toastText;

export function say(text: string, bad = false) {
  setToastText({ text, bad });
  clearTimeout(timer);
  timer = setTimeout(() => setToastText(undefined), bad ? 4000 : 1800);
}

/** What went wrong, without the code the protocol puts before it. */
export function message(e: unknown): string {
  return e instanceof Error ? e.message.replace(/^\[\w+\]\s*/, "") : String(e);
}

/**
 * Run a change against the API, reload what is on screen and say how it
 * went. The page stays where it is; nothing navigates. Returns the result,
 * or undefined when it failed.
 */
export async function act<T>(run: () => Promise<T>, done = "保存した"): Promise<T | undefined> {
  try {
    const result = await run();
    await reread();
    if (done) say(done);
    return result;
  } catch (e) {
    say(message(e), true);
    return undefined;
  }
}

/** Read from the API without changing anything; a failure is said, not thrown. */
export async function look<T>(run: () => Promise<T>): Promise<T | undefined> {
  try {
    return await run();
  } catch (e) {
    say(message(e), true);
    return undefined;
  }
}
