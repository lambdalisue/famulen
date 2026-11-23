import { createAsync, query } from "@solidjs/router";
import { createEffect, createSignal, For, on, Show } from "solid-js";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";
import { KIND_NAMES, kindsQuery, matches, type Rec } from "~/lib/records";
import { Button, Dialog, SearchBox } from "~/components/ui";

/** Every record of these kinds, or of every kind there is. */
const pickQuery = query(async (kinds: string) => {
  const names = kinds ? kinds.split(",") : (await kindsQuery()).map((k) => k.name);
  const lists = await Promise.all(names.map((kind) => api.records.listRecords({ kind })));
  return lists.flatMap((l) => l.records);
}, "pick");

const SHOWN = 50;

function Choices(props: { kinds?: string[]; onPick: (r: Rec) => void }) {
  const all = createAsync(() => pickQuery(props.kinds?.join(",") ?? ""));
  const [words, setWords] = createSignal("");
  const [at, setAt] = createSignal(0);
  const found = () => (all() ?? []).filter((r) => matches(r, words())).slice(0, SHOWN);
  createEffect(on(words, () => setAt(0)));
  const move = (to: number) => {
    const last = found().length - 1;
    setAt(Math.max(0, Math.min(last, to)));
    list?.children[at()]?.scrollIntoView({ block: "nearest" });
  };
  let list: HTMLUListElement | undefined;
  return (
    <div class="service-add">
      <SearchBox label="探す" placeholder="題名や中身の言葉" autofocus value={words()} onInput={(e) => setWords(e.currentTarget.value)} onKeyDown={(e) => {
            if (e.isComposing) return;
            if (e.key === "ArrowDown") move(at() + 1);
            else if (e.key === "ArrowUp") move(at() - 1);
            else if (e.key === "Enter") {
              const r = found()[at()];
              if (r) props.onPick(r);
            } else return;
            e.preventDefault();
          }} />
      <ul class="pick-list" ref={list}>
        <For each={found()} fallback={<li class="empty">見つからない</li>}>
          {(r, n) => (
            <li>
              <button type="button" class="folder-item" aria-current={n() === at() ? "true" : undefined} onClick={() => props.onPick(r)}>
                <span class="grow">{r.title}</span>
                <span class="sub">
                  {KIND_NAMES[r.kind] ?? r.kind} <span class="mono">{r.code}</span>
                </span>
              </button>
            </li>
          )}
        </For>
      </ul>
    </div>
  );
}

/** The dialog that picks one of the notebook's records. */
export function PickDialog(props: {
  label: string;
  kinds?: string[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onPick: (r: Rec) => void;
  /** What has the focus once it closes; what opened it otherwise. */
  focusAfter?: () => HTMLElement | null;
}) {
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange} title={props.label} size="wide" focusAfter={props.focusAfter}>
      <Show when={props.open}>
        <Choices
          kinds={props.kinds}
          onPick={(r) => {
            props.onPick(r);
            props.onOpenChange(false);
          }}
        />
      </Show>
    </Dialog>
  );
}

/** A button to pick one of the notebook's records. */
export default function RecordPicker(props: { label: string; kinds?: string[]; onPick: (r: Rec) => void }) {
  const [open, setOpen] = createSignal(false);
  return (
    <>
      <Button small icon={icons.plus} onClick={() => setOpen(true)}>
{props.label}
      </Button>
      <PickDialog label={props.label} kinds={props.kinds} open={open()} onOpenChange={setOpen} onPick={props.onPick} />
    </>
  );
}
