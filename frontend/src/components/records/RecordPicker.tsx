import { createAsync, query } from "@solidjs/router";
import { createSignal, For, Show } from "solid-js";
import Icon from "~/components/Icon";
import Modal from "~/components/Modal";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";
import { KIND_NAMES, matches, type Rec } from "~/lib/records";

const pickQuery = query(async (kinds: string) => {
  const lists = await Promise.all(kinds.split(",").map((kind) => api.records.listRecords({ kind })));
  return lists.flatMap((l) => l.records);
}, "pick");

const SHOWN = 50;

function Choices(props: { kinds: string[]; onPick: (r: Rec) => void }) {
  const all = createAsync(() => pickQuery(props.kinds.join(",")));
  const [words, setWords] = createSignal("");
  const found = () => (all() ?? []).filter((r) => matches(r, words())).slice(0, SHOWN);
  return (
    <div class="service-add">
      <label class="service-search">
        <Icon svg={icons.search} size={16} />
        <input class="input" aria-label="探す" placeholder="題名や中身の言葉" autofocus value={words()} onInput={(e) => setWords(e.currentTarget.value)} />
      </label>
      <ul class="pick-list">
        <For each={found()} fallback={<li class="empty">見つからない</li>}>
          {(r) => (
            <li>
              <button type="button" class="folder-item" onClick={() => props.onPick(r)}>
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

/** Pick one of the notebook's records. */
export default function RecordPicker(props: { label: string; kinds: string[]; onPick: (r: Rec) => void }) {
  const [open, setOpen] = createSignal(false);
  return (
    <>
      <button type="button" class="btn small" onClick={() => setOpen(true)}>
        <Icon svg={icons.plus} size={12} />
        {props.label}
      </button>
      <Modal open={open()} onOpenChange={setOpen} title={props.label} size="wide">
        <Show when={open()}>
          <Choices
            kinds={props.kinds}
            onPick={(r) => {
              setOpen(false);
              props.onPick(r);
            }}
          />
        </Show>
      </Modal>
    </>
  );
}
