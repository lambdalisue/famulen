import { createEffect, createMemo, createSignal } from "solid-js";

/**
 * What is being typed over a saved value. It goes back to the saved value
 * only when that value itself changes: every save reloads the page's data,
 * and the same value arriving again must not wipe what is being typed.
 */
export function createDraft(saved: () => string) {
  const kept = createMemo(saved);
  const [draft, setDraft] = createSignal(kept());
  createEffect(() => setDraft(kept()));
  return [draft, setDraft] as const;
}
