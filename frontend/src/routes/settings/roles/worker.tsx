import { createAsync } from "@solidjs/router";
import { createSignal, For, Show } from "solid-js";
import LogSheet from "~/components/logs/LogSheet";
import { RunResult } from "~/components/records/Delegated";
import RoleModel from "~/components/RoleModel";
import { Role } from "~/gen/famulen/v1/settings_pb";
import type { WorkerFinish, WorkerSettings } from "~/gen/famulen/v1/workers_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { setView } from "~/lib/view";
import { delegatedQuery, workerSettingsQuery } from "~/lib/workers";
import * as icons from "~/lib/icons";
import {
  Button,
  Dialog,
  DialogActions,
  EmptyState,
  Field,
  IconButton,
  List,
  ListRow,
  NumberField,
  PageHeader,
  Section,
  Switch,
  TextArea,
  TextField,
  whenConfirmed,
} from "~/components/ui";

type Change = Partial<{ slots: number; paused: boolean; finishes: { directory: string; steps: string }[] }>;

let saving: Promise<unknown> = Promise.resolve();

/** One after another over what is saved by then: what is on screen may be older. */
function save(change: (now: WorkerSettings) => Change, done?: string) {
  const saved = saving.then(() =>
    act(async () => {
      const now = await api.workers.getWorkerSettings({});
      const { slots, paused, finishes } = now;
      return api.workers.setWorkerSettings({ slots, paused, finishes, ...change(now) });
    }, done),
  );
  saving = saved;
  return saved;
}

/** How many delegated tasks run at once, and whether any is started. */
function Pool(props: { settings: WorkerSettings }) {
  const s = () => props.settings;
  return (
    <Section
      title="同時に動く数"
      note={
        <>
          委ねたタスクは、優先度、期日、古さの順に、ここで決めた数までを同時に動かす。あなたの承認や答えを待っているものは数に入らない。並びは
          <a href="/tasks">タスク</a>の頁で見られる。
        </>
      }
    >
      <NumberField label="同時に動く数" min={1} max={20} value={s().slots} unit="件" onSave={(slots) => void save(() => ({ slots }))} />
      <Switch checked={s().paused} onChange={(paused) => void save(() => ({ paused }))}>
        一時停止する（動いているものは続く）
      </Switch>
    </Section>
  );
}

const EXAMPLE = `1. 差分をレビューし、指摘を直す
2. PR を作る
3. CI が通るまで直し、通ったらマージする
4. 動いているものに反映する`;

/** Writes one finish; `was` is the one it takes the place of. */
function FinishDialog(props: { was?: WorkerFinish; open: boolean; onOpenChange: (open: boolean) => void }) {
  const [directory, setDirectory] = createSignal(props.was?.directory ?? "");
  const [steps, setSteps] = createSignal(props.was?.steps ?? "");
  const was = props.was?.directory;
  const submit = async (e: SubmitEvent) => {
    e.preventDefault();
    const now = { directory: directory().trim(), steps: steps().trim() };
    const saved = await save((s) => ({
      finishes:
        was !== undefined && s.finishes.some((f) => f.directory === was)
          ? s.finishes.map((f) => (f.directory === was ? now : f))
          : [...s.finishes, now],
    }));
    if (saved) props.onOpenChange(false);
  };
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange} title={props.was ? "仕上げを直す" : "仕上げを足す"} size="wide">
      <form class="stack" onSubmit={submit}>
        <Field label="ディレクトリ" hint="この中の仕事に使う。空ならどの仕事にも使う。">
          <TextField class="mono" value={directory()} placeholder="/home/me/src/project" onInput={(e) => setDirectory(e.currentTarget.value)} />
        </Field>
        <Field label="手順">
          <TextArea rows="10" required value={steps()} placeholder={EXAMPLE} onInput={(e) => setSteps(e.currentTarget.value)} />
        </Field>
        <DialogActions submit="保存" disabled={!steps().trim()} />
      </form>
    </Dialog>
  );
}

/** How work on code is finished: the owner's own steps, handed to every Adroit session that may change code. */
function Finishes(props: { finishes: WorkerFinish[] }) {
  const [editing, setEditing] = createSignal<{ was?: WorkerFinish }>();
  const name = (f: WorkerFinish) => f.directory || "どの仕事にも";
  const remove = (f: WorkerFinish) =>
    whenConfirmed(
      `「${name(f)}」の仕上げを消す`,
      "消す",
      () => save((s) => ({ finishes: s.finishes.filter((o) => o.directory !== f.directory) }), "消した"),
    );
  return (
    <Section
      title="仕上げ"
      label="仕上げ"
      note="Adroit に委ねた実装を、どう終わらせるか。秘書が何を書いたかによらず、頼みに添えて渡す。仕事にいちばん近いディレクトリのもの一つを使う。"
      action={
        <Button small icon={icons.plus} onClick={() => setEditing({})}>
          仕上げを足す
        </Button>
      }
    >
      <List>
        <For each={props.finishes} fallback={<EmptyState>まだない。頼みに書かれたとおりに終わる</EmptyState>}>
          {(f) => (
            <ListRow
              title={<span classList={{ mono: !!f.directory }}>{name(f)}</span>}
              sub={<span class="finish-steps">{f.steps}</span>}
              end={
                <>
                  <IconButton plain icon={icons.pencil} label={`「${name(f)}」の仕上げを直す`} onClick={() => setEditing({ was: f })} />
                  <IconButton plain icon={icons.x} label={`「${name(f)}」の仕上げを消す`} onClick={() => void remove(f)} />
                </>
              }
            />
          )}
        </For>
      </List>
      <Show when={editing()} keyed>
        {(e) => <FinishDialog was={e.was} open onOpenChange={(open) => !open && setEditing(undefined)} />}
      </Show>
    </Section>
  );
}

function Ended() {
  const delegated = createAsync(() => delegatedQuery());
  return (
    <List>
      <For each={delegated()?.ended} fallback={<EmptyState>まだない</EmptyState>}>
        {(r) => <RunResult run={r} />}
      </For>
    </List>
  );
}

/** Workers: what does the tasks that were delegated. */
export default function Worker() {
  setView("設定", "手代");
  const settings = createAsync(() => workerSettingsQuery());
  return (
    <>
      <PageHeader
        title="手代"
        note={
          <>
            秘書やあなたがタスクとして委ねた仕事を、一つずつ片付けるエージェント。仕事ごとに生まれ、渡された道具と手帳のファイルだけを使い、済めば消える。あなたに確かめる道具は使えない。Adroit
            に委ねた仕事は、あなたの承認を得てから Adroit のセッションとして始まる。
          </>
        }
      />
      <div class="prefs">
        <RoleModel
          role={Role.WORKER}
          cap="使わないにすると、秘書と同じモデルで、一日に決まった数まで動く。上限に達した日は、手代に委ねた仕事は次の日まで待つ。値段のあるモデルは円で、ないモデルは回数で数える。"
        />
        <Show when={settings()}>
          {(s) => (
            <>
              <Pool settings={s()} />
              <Finishes finishes={s().finishes} />
            </>
          )}
        </Show>
      </div>
      <LogSheet title="済んだ仕事" label="済んだ仕事を見る">
        <Ended />
      </LogSheet>
    </>
  );
}
