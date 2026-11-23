import { shortTime, weekday, WEEKDAYS } from "~/lib/dates";
import { createAsync, useNavigate } from "@solidjs/router";
import { createSignal, For, Show } from "solid-js";
import type { RoutineRun } from "~/gen/famulen/v1/settings_pb";
import { act, look } from "~/lib/act";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";
import { assigneeOf, newWorkgroups, OWNER, SECRETARY, type Rec, routinesQuery, valueOf } from "~/lib/records";
import { useOwner } from "~/lib/owner";
import { workgroup } from "~/lib/workgroup";
import { Button, Callout, Dialog, DialogActions, Field, Icon, Segmented, Select, Switch, Tag, TextArea, TextField } from "~/components/ui";
import type { RowParts } from "./Row";

const useRuns = () => {
  const routines = createAsync(() => routinesQuery());
  return (id: string) => routines()?.find((r) => r.id === id);
};

const skip = (routine: string, run: RoutineRun) =>
  act(() => api.routines.skipRun({ routine, at: run.at, undo: run.skipped }), run.skipped ? "戻した" : "この回を飛ばす");

/** Whether a routine runs at all. */
function Running(props: { routine: Rec }) {
  const r = () => props.routine;
  return (
    <Switch
      hidden
      checked={isOn(r())}
      onChange={(on) =>
        void act(() => api.records.saveRecord({ kind: "routines", id: r().id, fields: [{ name: "Enabled", value: on ? "yes" : "no" }] }), on ? "動かす" : "止めた")
      }
    >
      {r().title}を動かす
    </Switch>
  );
}

const isOn = (r: Rec) => valueOf(r, "Enabled") !== "no";

/** When a routine next runs, with a way to leave that run out. */
function Next(props: { routine: Rec }) {
  const runs = useRuns();
  const owner = useOwner();
  const next = () => (isOn(props.routine) ? runs(props.routine.id)?.runs[0] : undefined);
  const aides = () => (valueOf(props.routine, "Assignee") || OWNER) === SECRETARY;
  return (
    <span class="rec-side">
      <Show when={next()}>
        {(n) => (
          <span class="rec-when">
            {n().skipped ? "飛ばす " : "次 "}
            {shortTime(n().at, owner.tz())}
          </span>
        )}
      </Show>
      <span class="row">
        <Show when={next()}>
          {(n) => (
            <button
              type="button"
              class="tag"
              aria-label={`${props.routine.title}の次の回を${n().skipped ? "飛ばさない" : "飛ばす"}`}
              onClick={() => void skip(props.routine.id, n())}
            >
              <Icon svg={n().skipped ? icons.arrowBackUp : icons.playerSkipForward} size={11} />
              {n().skipped ? "戻す" : "次を飛ばす"}
            </button>
          )}
        </Show>
        <Tag tone={aides() ? "aide" : undefined}>{assigneeOf(props.routine, owner.aide())}</Tag>
      </span>
    </span>
  );
}

function When(props: { routine: Rec }) {
  const runs = useRuns();
  const info = () => runs(props.routine.id);
  return (
    <>
      <span>{info()?.when}</span>
      <Show when={info()?.problem}>{(p) => <Tag tone="danger">{p()}</Tag>}</Show>
    </>
  );
}

/** One routine in a list. */
export function routineParts(r: () => Rec): RowParts {
  return {
    lead: () => <Running routine={r()} />,
    title: () => <span classList={{ quiet: !isOn(r()) }}>{r().title}</span>,
    sub: () => <When routine={r()} />,
    end: () => <Next routine={r()} />,
  };
}

/** The runs to come, each skipped or put back on its own. */
export function Runs(props: { routine: Rec }) {
  const runs = useRuns();
  const owner = useOwner();
  const tz = owner.tz;
  return (
    <Show when={runs(props.routine.id)}>
      {(info) => (
        <section class="rec-section">
          <h3>これからの回</h3>
          <Callout icon={icons.repeat} action={<Show when={info().problem}>{(p) => <Tag tone="danger">{p()}</Tag>}</Show>}>
            <b>{info().when || "時刻がまだない"}</b>
            <Show when={info().lastRun > 0n}>
              <span class="sub"> 前に動いたのは {shortTime(info().lastRun, tz())}</span>
            </Show>
          </Callout>
          <For each={info().runs}>
            {(r) => (
              <div class="run" classList={{ skipped: r.skipped }}>
                <span class="mono run-at">{shortTime(r.at, tz())}</span>
                <Show when={r.skipped}>
                  <Tag>飛ばす</Tag>
                </Show>
                <span class="grow" />
                <Button small onClick={() => void skip(props.routine.id, r)}>
                  {r.skipped ? "戻す" : "この回を飛ばす"}
                </Button>
              </div>
            )}
          </For>
          <p class="sub">
            ほかのタスクの状態や前の回しだいで飛ばすなら、下の「飛ばす条件」を足す。
          </p>
        </section>
      )}
    </Show>
  );
}

const REPEATS: [string, string][] = [
  ["daily", "毎日"],
  ["weekdays", "平日"],
  ["weekly", "毎週"],
  ["monthly", "毎月"],
  ["cron", "細かく"],
];

/** Add a routine: what, when in words, and what to do then. */
export function AddRoutine(props: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const navigate = useNavigate();
  const owner = useOwner();
  const [repeat, setRepeat] = createSignal("weekdays");
  const [who, setWho] = createSignal("secretary");
  const add = async (e: SubmitEvent & { currentTarget: HTMLFormElement }) => {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    const text = (k: string) => String(f.get(k) ?? "").trim();
    const when = await look(() =>
      api.routines.cronFrom({
        repeat: repeat(),
        time: text("time"),
        weekday: Number(text("weekday") || 1),
        day: Number(text("day") || 1),
        cron: text("cron"),
      }),
    );
    if (!when) return;
    const made = await act(
      () =>
        api.records.saveRecord({
          kind: "routines",
          title: text("title"),
          notes: text("notes"),
          workgroups: newWorkgroups(workgroup()),
          fields: [
            { name: "Schedule", value: when.cron },
            { name: "Assignee", value: who() },
          ],
        }),
      "足した",
    );
    if (!made) return;
    props.onOpenChange(false);
    navigate(`/tasks?tab=routines&id=${made.id}`);
  };
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange} title="定期タスクを足す">
      <form class="dialog-form" onSubmit={add}>
        <Field label="何をするか">
          <TextField name="title" required autofocus />
        </Field>
        <Field label="いつ" group>
          <Segmented label="繰り返し" items={REPEATS} value={repeat()} onChoose={setRepeat} />
          <div class="row">
            <Show when={repeat() === "weekly"}>
              <Select name="weekday" aria-label="曜日">
                <For each={WEEKDAYS}>{(d, i) => <option value={i()} selected={i() === 1}>{d}曜</option>}</For>
              </Select>
            </Show>
            <Show when={repeat() === "monthly"}>
              <TextField name="day" type="number" min="1" max="28" value="1" aria-label="日" />
            </Show>
            <Show when={repeat() !== "cron"} fallback={<TextField class="mono" name="cron" required placeholder="分 時 日 月 曜日" aria-label="cron" />}>
              <TextField name="time" type="time" value="09:00" required aria-label="時刻" />
            </Show>
          </div>
        </Field>
        <Field label="だれが" hint={who() === "owner" ? "回が来ると、あなたのタスクができる。" : "回が来ると、下に書いたことをする。"} group>
          <Segmented
            label="担当"
            items={[
              ["secretary", owner.aide()],
              ["owner", "あなた"],
            ]}
            value={who()}
            onChoose={setWho}
          />
        </Field>
        <Field label="詳しく">
          <TextArea name="notes" placeholder="Markdown で書ける" />
        </Field>
        <DialogActions submit="足す" />
      </form>
    </Dialog>
  );
}
