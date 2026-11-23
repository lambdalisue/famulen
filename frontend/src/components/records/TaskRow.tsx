import { Show } from "solid-js";
import { Icon, ListRow, Tag } from "~/components/ui";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { dateText, daysLeft, nearness } from "~/lib/dates";
import * as icons from "~/lib/icons";
import { useOwner } from "~/lib/owner";
import { assigneeOf, choiceOf, hrefOf, isOver, OWNER, plainText, type Rec, SECRETARY, valueOf, WORKER } from "~/lib/records";
import { type RowParts, RowTags } from "./Row";

const put = (t: Rec, name: string, value: string, done: string) =>
  act(() => api.records.saveRecord({ kind: "tasks", id: t.id, fields: [{ name, value }] }), done);

/** Finish a task, or take it up again. */
export function finish(t: Rec) {
  return put(t, "Status", isOver(t) ? "pending" : "done", isOver(t) ? "戻した" : "済にした");
}

/** The box that finishes a task. */
function Check(props: { task: Rec }) {
  const over = () => isOver(props.task);
  return (
    <button
      type="button"
      class={`rec-check${over() ? " on" : ""}`}
      aria-label={`${props.task.title}${over() ? "を戻す" : "を済にする"}`}
      onClick={() => void finish(props.task)}
    >
      <Icon svg={icons.check} size={13} />
    </button>
  );
}

/** When it is due: the days left, or the date once it is over. */
function Due(props: { task: Rec }) {
  const owner = useOwner();
  const due = () => valueOf(props.task, "Due");
  const over = () => isOver(props.task);
  return (
    <Show when={due()}>
      <span class={`rec-when ${over() ? "" : nearness(due(), owner.today())}`} title={dateText(due())}>
        {over() ? dateText(due().slice(0, 10)) : daysLeft(due(), owner.today())}
        {due().slice(11) ? ` ${due().slice(11)}` : ""}
      </span>
    </Show>
  );
}

/** Who does it, handed over and taken back with a press. */
function Assignee(props: { task: Rec }) {
  const owner = useOwner();
  const t = () => props.task;
  const aides = () => valueOf(t(), "Assignee") === SECRETARY;
  return (
    <Show when={valueOf(t(), "Assignee") !== WORKER} fallback={<Tag>{assigneeOf(t(), owner.aide())}</Tag>}>
      <button
        type="button"
        class={`tag${aides() ? " aide" : ""}`}
        aria-label={aides() ? "自分でやる" : "任せる"}
        title={aides() ? "自分でやる" : `${owner.aide()}に任せる`}
        onClick={() => void put(t(), "Assignee", aides() ? OWNER : SECRETARY, aides() ? "自分でやる" : `${owner.aide()}に任せた`)}
      >
        {assigneeOf(t(), owner.aide())}
      </button>
    </Show>
  );
}

/** One task in a list: finish it, hand it over, or open it. */
export function taskParts(t: () => Rec): RowParts {
  const status = () => valueOf(t(), "Status");
  return {
    lead: () => <Check task={t()} />,
    title: () => <span classList={{ struck: isOver(t()) }}>{t().title}</span>,
    sub: () => (
      <>
        <Show when={valueOf(t(), "Priority") === "high" && !isOver(t())}>
          <Tag tone="warn">優先</Tag>
        </Show>
        <Show when={t().blockers.length > 0}>
          <Tag tone="danger" title={t().blockers.map((b) => b.title).join("、")}>
            <Icon svg={icons.lock} size={11} /> {t().blockers[0].title} 待ち
          </Tag>
        </Show>
        <Show when={["in_progress", "waiting", "cancelled"].includes(status())}>
          <Tag>{choiceOf(status())}</Tag>
        </Show>
        <Show when={valueOf(t(), "Waiting for")}>{(w) => <span>{plainText(w())}</span>}</Show>
      </>
    ),
    end: () => (
      <span class="rec-side">
        <Due task={t()} />
        <Assignee task={t()} />
      </span>
    ),
  };
}

/** A task in a list of another page, which opens it on its own. */
export default function TaskRow(props: { task: Rec }) {
  const parts = taskParts(() => props.task);
  return (
    <ListRow
      class="rec-row"
      record={props.task.id}
      href={hrefOf("tasks", props.task.id)}
      lead={parts.lead?.()}
      title={parts.title?.()}
      sub={
        <>
          {parts.sub?.()}
          <RowTags record={props.task} />
        </>
      }
      end={parts.end?.()}
    />
  );
}
