import { Switch } from "@ark-ui/solid/switch";
import { createAsync, useNavigate } from "@solidjs/router";
import { createSignal, For, Show } from "solid-js";
import Icon from "~/components/Icon";
import Modal from "~/components/Modal";
import Segs from "~/components/Segs";
import type { RoutineRun } from "~/gen/famulen/v1/settings_pb";
import { act, look } from "~/lib/act";
import { api } from "~/lib/api";
import { personaQuery } from "~/lib/data";
import * as icons from "~/lib/icons";
import { assigneeOf, newWorkgroups, type Rec, routinesQuery, valueOf } from "~/lib/records";
import { shortTime } from "~/lib/time";
import { workgroup } from "~/lib/workgroup";

const useRuns = () => {
  const routines = createAsync(() => routinesQuery());
  return (id: string) => routines()?.find((r) => r.id === id);
};

const skip = (routine: string, run: RoutineRun) =>
  act(() => api.routines.skipRun({ routine, at: run.at, undo: run.skipped }), run.skipped ? "戻した" : "この回を飛ばす");

/** One routine in a list. */
export function RoutineRow(props: { routine: Rec }) {
  const runs = useRuns();
  const persona = createAsync(() => personaQuery());
  const info = () => runs(props.routine.id);
  const next = () => info()?.runs[0];
  const on = () => valueOf(props.routine, "Enabled") !== "no";
  return (
    <>
      <Switch.Root
        checked={on()}
        onCheckedChange={(d) =>
          void act(
            () => api.records.saveRecord({ kind: "routines", id: props.routine.id, fields: [{ name: "Enabled", value: d.checked ? "yes" : "no" }] }),
            d.checked ? "動かす" : "止めた",
          )
        }
      >
        <Switch.Control class="switch">
          <Switch.Thumb class="switch-thumb" />
        </Switch.Control>
        <Switch.Label class="sr">{props.routine.title}を動かす</Switch.Label>
        <Switch.HiddenInput />
      </Switch.Root>
      <span class="rec-main">
        <b style={on() ? {} : { color: "#8C8A82" }}>{props.routine.title}</b>
        <span class="rec-sub">
          <span>{info()?.when}</span>
          <Show when={info()?.problem}>{(p) => <span class="tag bad">{p()}</span>}</Show>
          <For each={props.routine.labels}>{(l) => <span class="tag label">{l}</span>}</For>
        </span>
      </span>
      <span class="rec-side">
        <Show when={on() && next()}>
          {(n) => (
            <span class="rec-when">
              {n().skipped ? "飛ばす " : "次 "}
              {shortTime(n().at, persona()?.timezone ?? "UTC")}
            </span>
          )}
        </Show>
        <span class="row">
          <Show when={on() && next()}>
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
          <span class={`tag${assigneeOf(props.routine, "秘書") === "秘書" ? " aide" : ""}`}>{assigneeOf(props.routine, persona()?.name || "秘書")}</span>
        </span>
      </span>
    </>
  );
}

/** The runs to come, each skipped or put back on its own. */
export function Runs(props: { routine: Rec }) {
  const runs = useRuns();
  const persona = createAsync(() => personaQuery());
  const tz = () => persona()?.timezone ?? "UTC";
  return (
    <Show when={runs(props.routine.id)}>
      {(info) => (
        <section class="rec-section">
          <h3>これからの回</h3>
          <div class="callout">
            <b>{info().when || "時刻がまだない"}</b>
            <Show when={info().lastRun > 0n}>
              <span class="sub">前に動いたのは {shortTime(info().lastRun, tz())}</span>
            </Show>
            <Show when={info().problem}>{(p) => <span class="tag bad">{p()}</span>}</Show>
          </div>
          <For each={info().runs}>
            {(r) => (
              <div class="run" classList={{ skipped: r.skipped }}>
                <span class="mono run-at">{shortTime(r.at, tz())}</span>
                <Show when={r.skipped}>
                  <span class="tag">飛ばす</span>
                </Show>
                <span class="grow" />
                <button type="button" class="btn small" onClick={() => void skip(props.routine.id, r)}>
                  {r.skipped ? "戻す" : "この回を飛ばす"}
                </button>
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
const DAYS = ["日", "月", "火", "水", "木", "金", "土"];

/** Add a routine: what, when in words, and what to do then. */
export function AddRoutine(props: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const navigate = useNavigate();
  const persona = createAsync(() => personaQuery());
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
    <Modal open={props.open} onOpenChange={props.onOpenChange} title="定期タスクを足す">
      <form class="dialog-form" onSubmit={add}>
        <label class="dialog-field">
          <span>何をするか</span>
          <input class="input" name="title" required autofocus />
        </label>
        <div class="dialog-field">
          <span>いつ</span>
          <Segs label="繰り返し" items={REPEATS} value={repeat()} onChoose={setRepeat} />
          <div class="row">
            <Show when={repeat() === "weekly"}>
              <select class="input" name="weekday" aria-label="曜日">
                <For each={DAYS}>{(d, i) => <option value={i()} selected={i() === 1}>{d}曜</option>}</For>
              </select>
            </Show>
            <Show when={repeat() === "monthly"}>
              <input class="input" name="day" type="number" min="1" max="28" value="1" aria-label="日" />
            </Show>
            <Show when={repeat() !== "cron"} fallback={<input class="input mono" name="cron" required placeholder="分 時 日 月 曜日" aria-label="cron" />}>
              <input class="input" name="time" type="time" value="09:00" required aria-label="時刻" />
            </Show>
          </div>
        </div>
        <div class="dialog-field">
          <span>だれが</span>
          <Segs
            label="担当"
            items={[
              ["secretary", persona()?.name || "秘書"],
              ["owner", "あなた"],
            ]}
            value={who()}
            onChoose={setWho}
          />
          <span class="sub">{who() === "owner" ? "回が来ると、あなたのタスクができる。" : "回が来ると、下に書いたことをする。"}</span>
        </div>
        <label class="dialog-field">
          <span>詳しく</span>
          <textarea class="input" name="notes" placeholder="Markdown で書ける" />
        </label>
        <div class="dialog-actions">
          <button type="button" class="btn" onClick={() => props.onOpenChange(false)}>
            やめる
          </button>
          <button type="submit" class="btn primary">
            足す
          </button>
        </div>
      </form>
    </Modal>
  );
}
