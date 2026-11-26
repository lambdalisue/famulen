import { Title } from "@solidjs/meta";
import { createSignal, onCleanup, onMount, Show } from "solid-js";

/**
 * The console for the Even G2. The Even App on the owner's phone opens this
 * page, which draws on the glasses; the phone shows only how it goes.
 */
export default function G2Page() {
  const [state, setState] = createSignal("眼鏡につないでいる");
  const [problem, setProblem] = createSignal("");
  let stop = () => {};
  let gone = false;
  onCleanup(() => {
    gone = true;
    stop();
  });
  onMount(async () => {
    // Loaded only in the Even App's browser: the SDK reaches for the phone's bridge as it loads.
    const [{ glasses }, { G2 }] = await Promise.all([import("~/g2/even"), import("~/g2/app")]);
    const late = setTimeout(() => setProblem("Even App の中で開くと、眼鏡に出る。"), 5_000);
    try {
      const found = await glasses();
      clearTimeout(late);
      // The page may have been left while the glasses were being reached.
      if (gone) return;
      const app = new G2(found);
      setProblem("");
      app.onState = setProblem;
      stop = () => app.stop();
      await app.start();
      setState("眼鏡に出している");
    } catch (e) {
      clearTimeout(late);
      setProblem(e instanceof Error ? e.message : String(e));
    }
  });
  return (
    <main class="g2-phone">
      <Title>Famulen · Even G2</Title>
      <h1>Famulen</h1>
      <p data-testid="g2-state">{state()}</p>
      <Show when={problem()}>
        <p class="sub" role="status">
          {problem()}
        </p>
      </Show>
    </main>
  );
}
