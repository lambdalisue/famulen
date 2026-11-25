import { createAsync, useNavigate } from "@solidjs/router";
import { createMemo, createSignal, For, Show } from "solid-js";
import LogSheet from "~/components/logs/LogSheet";
import Markdown from "~/components/Markdown";
import { RunResult } from "~/components/records/Delegated";
import RoleModel from "~/components/RoleModel";
import { Role } from "~/gen/famulen/v1/settings_pb";
import type { WorkerFinish, WorkerSettings } from "~/gen/famulen/v1/workers_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { useParam, withParams } from "~/lib/params";
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
  LinkButton,
  List,
  ListRow,
  NumberField,
  PageHeader,
  Section,
  Switch,
  Tag,
  TextArea,
  TextField,
  whenConfirmed,
} from "~/components/ui";

type Change = Partial<{ slots: number; paused: boolean; finishes: { repository: string; steps: string }[] }>;

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

/** One finish: `own` as the owner wrote it, or as Famulen brings it along; `bundled` when there is one to go back to. */
type Shown = { repository: string; steps: string; own: boolean; bundled: boolean };

const sameRepository = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** The owner's own and what Famulen brings along, those for any work first. */
function shown(s: WorkerSettings): Shown[] {
  const has = (all: WorkerFinish[], f: WorkerFinish) => all.some((o) => sameRepository(o.repository, f.repository));
  return [
    ...s.finishes.map((f) => ({ repository: f.repository, steps: f.steps, own: true, bundled: has(s.bundledFinishes, f) })),
    ...s.bundledFinishes
      .filter((f) => !has(s.finishes, f))
      .map((f) => ({ repository: f.repository, steps: f.steps, own: false, bundled: true })),
  ].sort((a, b) => (a.repository < b.repository ? -1 : 1));
}

/** Writes one finish; `was` is the one it takes the place of. */
function FinishDialog(props: {
  was?: Shown;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSaved: (now: { repository: string }) => void;
}) {
  const [repository, setRepository] = createSignal(props.was?.repository ?? "");
  const [steps, setSteps] = createSignal(props.was?.steps ?? "");
  const was = props.was?.repository;
  const submit = async (e: SubmitEvent) => {
    e.preventDefault();
    const now = { repository: repository().trim(), steps: steps().trim() };
    let place = -1;
    const saved = await save((s) => {
      place = was === undefined ? -1 : s.finishes.findIndex((f) => sameRepository(f.repository, was));
      if (place < 0) place = s.finishes.length;
      return { finishes: [...s.finishes.slice(0, place), now, ...s.finishes.slice(place + 1)] };
    });
    if (!saved) return;
    // The address it was written by is kept as its one name.
    props.onSaved(saved.finishes[place]);
    props.onOpenChange(false);
  };
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange} title={props.was ? "仕上げを直す" : "仕上げを足す"} size="wide">
      <form class="stack" onSubmit={submit}>
        <Field label="リポジトリ" hint="クローンした場所によらず、このリポジトリの仕事に使う。空なら、ほかのどの仕事にも使う。">
          <TextField
            class="mono"
            value={repository()}
            readOnly={props.was?.bundled}
            placeholder="github.com/owner/name"
            onInput={(e) => setRepository(e.currentTarget.value)}
          />
        </Field>
        <Field label="手順" hint="Markdown で書ける。">
          <TextArea rows="10" required value={steps()} placeholder={EXAMPLE} onInput={(e) => setSteps(e.currentTarget.value)} />
        </Field>
        <DialogActions submit="保存" disabled={!steps().trim()} />
      </form>
    </Dialog>
  );
}

const PAGE = "/settings/roles/worker";
/** Stands in the address for the finish of any work; a repository's name has a slash, so is never this. */
const ANY = "any";

const keyOf = (f: { repository: string }) => f.repository || ANY;
const hrefOf = (f: { repository: string }) => withParams(PAGE, { finish: keyOf(f) });
const nameOf = (f: Shown) => f.repository || "どの仕事にも";

const lastPart = (repository: string) => repository.split("/").at(-1) ?? repository;

/** How work on code is finished: the owner's own steps, one set picked from the list and read beside it. */
function Finishes(props: { settings: WorkerSettings }) {
  const navigate = useNavigate();
  const asked = useParam("finish");
  const [editing, setEditing] = createSignal<{ was?: Shown }>();
  const all = createMemo(() => shown(props.settings));
  const picked = createMemo(() => all().find((f) => keyOf(f) === asked()));
  const remove = (f: Shown) =>
    whenConfirmed(`「${nameOf(f)}」の仕上げを${f.bundled ? "既定に戻す" : "消す"}`, f.bundled ? "戻す" : "消す", async () => {
      const without = (s: WorkerSettings) => ({ finishes: s.finishes.filter((o) => !sameRepository(o.repository, f.repository)) });
      const done = await save(without, f.bundled ? "既定に戻した" : "消した");
      if (done && !f.bundled) navigate(PAGE, { scroll: false, replace: true });
    });
  return (
    <Section
      title="仕上げ"
      label="仕上げ"
      note="外のエージェントに委ねた、コードを変える仕事を、どう終わらせるか。秘書が何を書いたかによらず、頼みに添えて渡す。仕事のリポジトリのものを使い、なければ「どの仕事にも」のものを使う。"
      action={
        <Button small icon={icons.plus} onClick={() => setEditing({})}>
          仕上げを足す
        </Button>
      }
    >
      <div class="finishes">
        <div class="finish-split" classList={{ open: !!picked() }}>
          <List class="finish-list" label="仕上げの一覧">
            <For each={all()} fallback={<EmptyState>まだない。頼みに書かれたとおりに終わる</EmptyState>}>
              {(f) => (
                <ListRow
                  href={hrefOf(f)}
                  current={picked()?.repository === f.repository}
                  title={f.repository ? <span class="mono">{lastPart(f.repository)}</span> : nameOf(f)}
                  sub={f.repository ? <span class="mono finish-path">{f.repository}</span> : "ほかに当てはまるものがないとき"}
                  end={f.own ? undefined : <Tag>既定</Tag>}
                  leads
                />
              )}
            </For>
          </List>
          <Show when={picked()}>
            {(f) => (
              <section class="finish-detail" aria-label="仕上げの詳細">
                <LinkButton small class="finish-back" icon={icons.chevronLeft} href={PAGE} noScroll>
                  仕上げの一覧
                </LinkButton>
                <header class="finish-head">
                  <h3 classList={{ mono: !!f().repository }}>{nameOf(f())}</h3>
                  <IconButton plain icon={icons.pencil} label={`「${nameOf(f())}」の仕上げを直す`} onClick={() => setEditing({ was: f() })} />
                  <Show when={f().own}>
                    <IconButton
                      plain
                      icon={f().bundled ? icons.refresh : icons.trash}
                      label={`「${nameOf(f())}」の仕上げを${f().bundled ? "既定に戻す" : "消す"}`}
                      onClick={() => void remove(f())}
                    />
                  </Show>
                  <IconButton
                    plain
                    class="finish-close"
                    icon={icons.x}
                    label="詳細を閉じる"
                    onClick={() => navigate(PAGE, { scroll: false })}
                  />
                </header>
                <div class="rec-prose">
                  <Markdown text={f().steps} />
                </div>
              </section>
            )}
          </Show>
        </div>
      </div>
      <Show when={editing()} keyed>
        {(e) => (
          <FinishDialog
            was={e.was}
            open
            onOpenChange={(open) => !open && setEditing(undefined)}
            onSaved={(f) => navigate(hrefOf(f), { scroll: false, replace: !!e.was })}
          />
        )}
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
            秘書やあなたがタスクとして委ねた仕事を、一つずつ片付けるエージェント。仕事ごとに生まれ、渡された道具と手帳のファイルだけを使い、済めば消える。あなたに確かめる道具は使えない。外のエージェントに委ねた仕事は、あなたの承認を得てから、そのセッションとして始まる。
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
              <Finishes settings={s()} />
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
