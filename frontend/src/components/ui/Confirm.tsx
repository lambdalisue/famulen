import { createSignal, Show } from "solid-js";
import { Dialog, DialogActions } from "./Dialog";

type Asked = { title: string; note?: string; verb: string; answer: (yes: boolean) => void };

const [asked, setAsked] = createSignal<Asked | undefined>();
const [open, setOpen] = createSignal(false);

/**
 * Ask before something that cannot be put back by hand: a record, a login, a
 * key, work under way. What is put back by typing or choosing one value again,
 * such as a label or a tile, is not asked about.
 */
export function confirmed(title: string, verb: string, note?: string): Promise<boolean> {
  asked()?.answer(false);
  return new Promise((answer) => {
    setAsked({ title, verb, note, answer });
    setOpen(true);
  });
}

/** Ask, and do it when the owner says so. */
export async function whenConfirmed<T>(title: string, verb: string, run: () => Promise<T>, note?: string): Promise<T | undefined> {
  return (await confirmed(title, verb, note)) ? run() : undefined;
}

/** The one dialog every question of {@link confirmed} is asked in. */
export function ConfirmHost() {
  const answer = (yes: boolean) => {
    const now = asked();
    setOpen(false);
    setAsked((a) => (a ? { ...a, answer: () => {} } : a));
    now?.answer(yes);
  };
  return (
    <Show when={asked()}>
      {(a) => (
        <Dialog open={open()} onOpenChange={(o) => !o && answer(false)} title={a().title} description={a().note} alert>
          <DialogActions submit={a().verb} variant="danger" onSubmit={() => answer(true)} />
        </Dialog>
      )}
    </Show>
  );
}
