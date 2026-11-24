import { For, Show, createSignal } from "solid-js";
import type { Folder } from "~/gen/famulen/v1/settings_pb";
import { look } from "~/lib/act";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";
import { coveringPlace, isUnder } from "~/lib/places";
import { Checkbox, Dialog, DialogActions, DialogTrigger, Icon, IconButton } from "~/components/ui";

type Mark = { kind: "added" | "chosen" | "covered" | "within" | "disk" | "none"; by?: string };

const NOTES: Record<Mark["kind"], (by: string) => string> = {
  added: () => "足してある",
  covered: (by) => `${by} で書ける`,
  within: (by) => `${by} を選んである`,
  disk: () => "ディスク全体は選べない",
  chosen: () => "",
  none: () => "",
};

/**
 * Walk the folders of the machine Famulen runs on and pick several at once.
 * The browser's own folder dialog cannot be used: it shows the viewer's
 * machine, which need not be Famulen's, and never hands over a whole path.
 */
export default function FolderPicker(props: {
  start: () => string;
  places: () => readonly string[];
  onAdd: (paths: string[]) => Promise<boolean>;
}) {
  const [open, setOpen] = createSignal(false);
  const [folder, setFolder] = createSignal<Folder>();
  const [chosen, setChosen] = createSignal<string[]>([]);
  const go = async (path: string) => {
    const f = await look(() => api.settings.getFolder({ path }));
    if (f) setFolder(f);
    return f;
  };
  const opened = async (next: boolean) => {
    setOpen(next);
    if (!next) return;
    setFolder(undefined);
    setChosen([]);
    const start = props.start().trim();
    if (!(start && (await go(start)))) await go("");
  };
  const child = (name: string) => `${folder()?.path.replace(/\/$/, "") ?? ""}/${name}`;
  const mark = (path: string): Mark => {
    if (path === "/") return { kind: "disk" };
    if (props.places().includes(path)) return { kind: "added" };
    const place = coveringPlace(path, props.places());
    if (place) return { kind: "covered", by: place };
    if (chosen().includes(path)) return { kind: "chosen" };
    const above = coveringPlace(path, chosen());
    if (above) return { kind: "within", by: above };
    return { kind: "none" };
  };
  const toggle = (path: string, on: boolean) =>
    setChosen((c) => (on ? [...c.filter((p) => !isUnder(p, path)), path] : c.filter((p) => p !== path)));
  const add = async () => {
    if (await props.onAdd(chosen())) setOpen(false);
  };

  const Row = (row: { path: string; label: string; enter?: () => void }) => {
    const m = () => mark(row.path);
    return (
      <li class="folder-row" data-mark={m().kind}>
        <Checkbox
          class="folder-check"
          checked={["added", "chosen", "covered", "within"].includes(m().kind)}
          disabled={["added", "covered", "within", "disk"].includes(m().kind)}
          onChange={(e) => toggle(row.path, e.currentTarget.checked)}
        >
          <span class="sr">{`${row.path} を選ぶ`}</span>
        </Checkbox>
        <Show
          when={row.enter}
          fallback={
            <span class="folder-item here">
              <Icon svg={icons.folderOpen} size={14} />
              <span>{row.label}</span>
            </span>
          }
        >
          {(enter) => (
            <button type="button" class="folder-item" onClick={() => enter()()}>
              <Icon svg={icons.folder} size={14} />
              <span>{row.label}</span>
            </button>
          )}
        </Show>
        <span class="folder-note sub">{NOTES[m().kind](m().by ?? "")}</span>
      </li>
    );
  };

  return (
    <Dialog
      open={open()}
      onOpenChange={(o) => void opened(o)}
      size="wide"
      title="フォルダを選ぶ"
      description="Famulen が動いているマシンのフォルダ。名前を押すと中へ進み、チェックで選ぶ。ほかのフォルダへ移っても、選んだものは残る。書ける場所の下は、選ばなくても書ける。"
      trigger={
        <DialogTrigger small icon={icons.folder}>
          選ぶ
        </DialogTrigger>
      }
    >
      <div class="folder-picker">
        <div class="folder-bar">
          <IconButton icon={icons.home} label="ホーム" variant="ghost" onClick={() => void go("")} />
          <IconButton
            icon={icons.folderUp}
            label="ひとつ上へ"
            variant="ghost"
            disabled={!folder()?.parent}
            onClick={() => void go(folder()?.parent ?? "")}
          />
          <span class="mono folder-path" aria-label="いまのフォルダ">
            {folder()?.path ?? "…"}
          </span>
        </div>
        <Show when={folder()}>
          {(f) => (
            <ul class="folder-list" aria-label="中のフォルダ">
              <Row path={f().path} label="このフォルダ" />
              <For each={f().children} fallback={<li class="sub folder-empty">中にフォルダはない。</li>}>
                {(name) => <Row path={child(name)} label={name} enter={() => void go(child(name))} />}
              </For>
            </ul>
          )}
        </Show>
        <div class="folder-chosen">
          <span class="sub">{chosen().length > 0 ? "選んだフォルダ" : "まだ選んでいない。"}</span>
          <Show when={chosen().length > 0}>
            <ul class="chips" aria-label="選んだフォルダ">
              <For each={chosen()}>
                {(p) => (
                  <li class="chip mono">
                    <span>{p}</span>
                    <button type="button" class="chip-x" aria-label={`${p} を選ばない`} onClick={() => toggle(p, false)}>
                      <Icon svg={icons.x} size={12} />
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </div>
      </div>
      <DialogActions
        submit={chosen().length > 0 ? `${chosen().length} 件を足す` : "足す"}
        disabled={chosen().length === 0}
        onSubmit={() => void add()}
      />
    </Dialog>
  );
}
