import { shortTime } from "~/lib/dates";
import { createAsync, useNavigate } from "@solidjs/router";
import { createEffect, createSignal, For, Match, Show, Switch } from "solid-js";
import { DelegatedQueue, Delegation } from "~/components/records/Delegated";
import RecordDetail from "~/components/records/RecordDetail";
import RecordsPage, { type Group } from "~/components/records/RecordsPage";
import { AddRoutine, RoutineRow, Runs } from "~/components/records/RoutineParts";
import TaskRow from "~/components/records/TaskRow";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";
import { bucketOf, isOver, type Rec, valueOf } from "~/lib/records";
import { cancelWake, wakesQuery } from "~/lib/routines";
import { useOwner } from "~/lib/owner";
import { useParam } from "~/lib/params";
import { usePicked } from "~/lib/picked";
import { Button, Icon, IconButton, Segmented } from "~/components/ui";

const RANK: { [p: string]: number } = { high: 0, normal: 1, low: 2 };

function order(a: Rec, b: Rec): number {
  const due = (r: Rec) => valueOf(r, "Due") || "9999";
  const rank = (r: Rec) => RANK[valueOf(r, "Priority")] ?? 1;
  return due(a).localeCompare(due(b)) || rank(a) - rank(b) || a.title.localeCompare(b.title);
}

function Tabs(props: { tab: string }) {
  const navigate = useNavigate();
  return (
    <Segmented
      label="タスクの種類"
      items={[
        ["once", "一度きり"],
        ["routines", "定期"],
      ]}
      value={props.tab}
      onChoose={(v) => navigate(v === "routines" ? "/tasks?tab=routines" : "/tasks")}
    />
  );
}

/** Finish a task, or take it up again, from its own page. */
function Finish(props: { task: Rec }) {
  const over = () => isOver(props.task);
  return (
    <div>
      <button
        type="button"
        class={`btn${over() ? "" : " go"}`}
        onClick={() =>
          void act(
            () => api.records.saveRecord({ kind: "tasks", id: props.task.id, fields: [{ name: "Status", value: over() ? "pending" : "done" }] }),
            over() ? "戻した" : "済にした",
          )
        }
      >
        <Icon svg={over() ? icons.arrowBackUp : icons.check} size={14} />
        {over() ? "まだに戻す" : "済にする"}
      </button>
    </div>
  );
}

/** What stands in the way of a task, above what it says. */
function Blocked(props: { task: Rec }) {
  return (
    <Show when={props.task.blockers.length > 0}>
      <div class="callout warn">
        <Icon svg={icons.lock} size={14} />
        {props.task.blockers.map((b) => `「${b.title}」`).join("")}が済むまで待っている
      </div>
    </Show>
  );
}

/** What the secretary put off until a time of its own choosing. */
function Wakes() {
  const owner = useOwner();
  const wakes = createAsync(() => wakesQuery());
  return (
    <Show when={(wakes() ?? []).length > 0}>
      <section class="rec-group">
        <h2>{owner.aide()} が待っている</h2>
        <div class="rec-rows">
          <For each={wakes()}>
            {(w) => (
              <div class="rec-row" style={{ cursor: "default" }}>
                <Icon svg={icons.clock} size={16} />
                <span class="rec-main">
                  <b>{w.why || w.note}</b>
                  <Show when={w.why}>
                    <span class="rec-sub">{w.note}</span>
                  </Show>
                </span>
                <span class="rec-when">{shortTime(w.atUnix, owner.tz())}</span>
                <IconButton plain icon={icons.trash} label={`起床予約「${w.note}」を取り消す`} onClick={() => void cancelWake(w.id)} />
              </div>
            )}
          </For>
        </div>
      </section>
    </Show>
  );
}

export default function Tasks() {
  const owner = useOwner();
  const asked = useParam("tab");
  const tab = () => (asked() === "routines" ? "routines" : "once");
  const [adding, setAdding] = createSignal(false);
  usePicked({ kind: () => (tab() === "routines" ? "routines" : "tasks"), section: () => "タスク", sub: () => (tab() === "routines" ? "定期" : "") });

  const groups = (records: Rec[]): Group[] => {
    const day = owner.today();
    const sorted = [...records].sort(order);
    const of = (b: string) => sorted.filter((r) => bucketOf(r, day) === b);
    const done = of("done").sort((a, b) => valueOf(b, "Finished").localeCompare(valueOf(a, "Finished")));
    return [
      { name: "今日", records: of("today") },
      { name: "今週", records: of("week") },
      { name: "いつか", records: of("later") },
      { name: "済", records: done, folded: records.some((r) => !isOver(r)) },
    ];
  };

  return (
    <Switch>
      <Match when={tab() === "once"}>
        <RecordsPage
          kind="tasks"
          base="/tasks"
          title="タスク"
          tools={<Tabs tab="once" />}
          narrow={["Assignee", "Status", "Priority"]}
          groups={groups}
          row={(r) => <TaskRow task={r} />}
          detail={(id) => <RecordDetail kind="tasks" id={id} back="/tasks" top={(r) => (
            <>
              <Finish task={r} />
              <Blocked task={r} />
              <Delegation task={r} />
            </>
          )} />}
          add={{ label: "タスクを足す", titleLabel: "何をするか", fields: ["Due", "Priority", "Assignee"] }}
          foot={
            <>
              <DelegatedQueue />
              <Wakes />
            </>
          }
          empty="タスクはまだない。上のボタンか、会話で頼んで足す。"
        />
      </Match>
      <Match when={tab() === "routines"}>
        <RecordsPage
          kind="routines"
          base="/tasks?tab=routines"
          title="定期タスク"
          tools={
            <>
              <Tabs tab="routines" />
              <Button variant="primary" icon={icons.plus} onClick={() => setAdding(true)}>
定期タスクを足す
              </Button>
            </>
          }
          narrow={["Assignee"]}
          row={(r) => <RoutineRow routine={r} />}
          detail={(id) => <RecordDetail kind="routines" id={id} back="/tasks?tab=routines" top={(r) => <Runs routine={r} />} />}
          empty="定期タスクはまだない。"
        />
        <AddRoutine open={adding()} onOpenChange={setAdding} />
      </Match>
    </Switch>
  );
}
