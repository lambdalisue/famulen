import { createAsync, useNavigate } from "@solidjs/router";
import { createSignal, For, Match, Show, Switch } from "solid-js";
import { DelegatedPage, DelegatedSummary, Delegation } from "~/components/records/Delegated";
import RecordDetail from "~/components/records/detail/RecordDetail";
import RecordsPage, { type Group } from "~/components/records/RecordsPage";
import { AddRoutine, routineParts, Runs } from "~/components/records/RoutineParts";
import { finish, taskParts } from "~/components/records/TaskRow";
import { Button, Callout, IconButton, List, ListRow, Segmented, whenConfirmed } from "~/components/ui";
import { shortTime } from "~/lib/dates";
import * as icons from "~/lib/icons";
import { useOwner } from "~/lib/owner";
import { useParam } from "~/lib/params";
import { usePicked } from "~/lib/picked";
import { bucketOf, isOver, type Rec, valueOf } from "~/lib/records";
import { cancelWake, wakesQuery } from "~/lib/routines";

const RANK: { [p: string]: number } = { high: 0, normal: 1, low: 2 };

function order(a: Rec, b: Rec): number {
  const due = (r: Rec) => valueOf(r, "Due") || "9999";
  const rank = (r: Rec) => RANK[valueOf(r, "Priority")] ?? 1;
  return due(a).localeCompare(due(b)) || rank(a) - rank(b) || a.title.localeCompare(b.title);
}

type Tab = "once" | "routines" | "delegated";

/** The views of the page, and what each is called. */
const TABS: [Tab, string][] = [
  ["once", "一度きり"],
  ["routines", "定期"],
  ["delegated", "委託"],
];

function Tabs(props: { tab: Tab }) {
  const navigate = useNavigate();
  return <Segmented label="タスクの種類" items={TABS} value={props.tab} onChoose={(v) => navigate(v === "once" ? "/tasks" : `/tasks?tab=${v}`)} />;
}

/** Finish a task, or take it up again, from its detail. */
function Finish(props: { task: Rec }) {
  const over = () => isOver(props.task);
  return (
    <div>
      <Button variant={over() ? undefined : "go"} icon={over() ? icons.arrowBackUp : icons.check} onClick={() => void finish(props.task)}>
        {over() ? "まだに戻す" : "済にする"}
      </Button>
    </div>
  );
}

/** What stands in the way of a task, above what it says. */
function Blocked(props: { task: Rec }) {
  return (
    <Show when={props.task.blockers.length > 0}>
      <Callout tone="warn" icon={icons.lock}>
        {props.task.blockers.map((b) => `「${b.title}」`).join("")}が済むまで待っている
      </Callout>
    </Show>
  );
}

/** What the secretary put off until a time of its own choosing. */
function Wakes() {
  const owner = useOwner();
  const wakes = createAsync(() => wakesQuery());
  return (
    <Show when={(wakes() ?? []).length > 0}>
      <section class="stack">
        <h2>{owner.aide()} が待っている</h2>
        <List>
          <For each={wakes()}>
            {(w) => (
              <ListRow
                icon={icons.clock}
                title={w.why || w.note}
                sub={w.why ? w.note : undefined}
                end={
                  <>
                    <span class="rec-when">{shortTime(w.atUnix, owner.tz())}</span>
                    <IconButton
                      plain
                      icon={icons.x}
                      label={`起床予約「${w.note}」を取り消す`}
                      onClick={() => void whenConfirmed(`起床予約「${w.note}」を取り消す`, "取り消す", () => cancelWake(w.id))}
                    />
                  </>
                }
              />
            )}
          </For>
        </List>
      </section>
    </Show>
  );
}

export default function Tasks() {
  const owner = useOwner();
  const asked = useParam("tab");
  const tab = (): Tab => TABS.find(([t]) => t === asked())?.[0] ?? "once";
  usePicked({
    kind: () => (tab() === "routines" ? "routines" : "tasks"),
    section: () => "タスク",
    sub: () => TABS.find(([t]) => t === tab() && t !== "once")?.[1] ?? "",
  });
  const [adding, setAdding] = createSignal(false);

  const groups = (records: Rec[]): Group[] => {
    const sorted = [...records].sort(order);
    const of = (b: string) => sorted.filter((r) => bucketOf(r, owner.today()) === b);
    const done = of("done").sort((a, b) => valueOf(b, "Finished").localeCompare(valueOf(a, "Finished")));
    return [
      { name: "今日", records: of("today") },
      { name: "今週", records: of("week") },
      { name: "いつか", records: of("later") },
      { name: "済", records: done, folded: records.some((r) => !isOver(r)) },
    ];
  };
  const task = (base: string) => (id: string) => (
    <RecordDetail
      kind="tasks"
      id={id}
      back={base}
      top={(r) => (
        <>
          <Finish task={r} />
          <Blocked task={r} />
          <Delegation task={r} />
        </>
      )}
    />
  );

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
          row={taskParts}
          detail={task("/tasks")}
          add={{ label: "タスクを足す", titleLabel: "何をするか", fields: ["Due", "Priority", "Assignee"] }}
          head={<DelegatedSummary />}
          foot={<Wakes />}
          empty="タスクはまだない。上のボタンか、会話で頼んで足す"
        />
      </Match>
      <Match when={tab() === "routines"}>
        <RecordsPage
          kind="routines"
          base="/tasks?tab=routines"
          title="定期タスク"
          tools={<Tabs tab="routines" />}
          narrow={["Assignee"]}
          row={routineParts}
          detail={(id) => <RecordDetail kind="routines" id={id} back="/tasks?tab=routines" top={(r) => <Runs routine={r} />} />}
          add={{ label: "定期タスクを足す", onAdd: () => setAdding(true) }}
          empty="定期タスクはまだない"
        />
        <AddRoutine open={adding()} onOpenChange={setAdding} />
      </Match>
      <Match when={tab() === "delegated"}>
        <DelegatedPage tools={<Tabs tab="delegated" />} detail={task("/tasks?tab=delegated")} />
      </Match>
    </Switch>
  );
}
