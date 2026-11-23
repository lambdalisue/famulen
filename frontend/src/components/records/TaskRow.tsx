import { dateText, daysLeft } from "~/lib/dates";
import { createAsync } from "@solidjs/router";
import { For, Show } from "solid-js";
import Icon from "~/components/Icon";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { workgroupsQuery } from "~/lib/data";
import * as icons from "~/lib/icons";
import { assigneeOf, choiceOf, isOver, OWNER, plainText, type Rec, SECRETARY, valueOf, WORKER, workgroupNames } from "~/lib/records";
import { useOwner } from "~/lib/owner";

/** One task in a list: finish it, hand it over, or press it to open it. */
export default function TaskRow(props: { task: Rec }) {
  const owner = useOwner();
  const workgroups = createAsync(() => workgroupsQuery());
  const aide = owner.aide;
  const t = () => props.task;
  const over = () => isOver(t());
  const due = () => valueOf(t(), "Due");
  const day = owner.today;
  const status = () => valueOf(t(), "Status");
  const aides = () => valueOf(t(), "Assignee") === SECRETARY;
  const put = (name: string, value: string, done: string) =>
    act(() => api.records.saveRecord({ kind: "tasks", id: t().id, fields: [{ name, value }] }), done);
  return (
    <>
      <button
        type="button"
        class={`rec-check${over() ? " on" : ""}`}
        aria-label={`${t().title}${over() ? "を戻す" : "を済にする"}`}
        onClick={() => void put("Status", over() ? "pending" : "done", over() ? "戻した" : "済にした")}
      >
        <Icon svg={icons.check} size={13} />
      </button>
      <span class="rec-main">
        <b style={over() ? { "text-decoration": "line-through" } : {}}>{t().title}</b>
        <span class="rec-sub">
          <Show when={valueOf(t(), "Priority") === "high"}>
            <span class="tag warn">優先</span>
          </Show>
          <Show when={t().blockers.length > 0}>
            <span class="tag bad" title={t().blockers.map((b) => b.title).join("、")}>
              <Icon svg={icons.lock} size={11} /> {t().blockers[0].title} 待ち
            </span>
          </Show>
          <Show when={["in_progress", "waiting", "cancelled"].includes(status())}>
            <span class="tag">{choiceOf(status())}</span>
          </Show>
          <Show when={valueOf(t(), "Waiting for")}>{(w) => <span>{plainText(w())}</span>}</Show>
          <For each={t().labels}>{(l) => <span class="tag label">{l}</span>}</For>
          <span>{workgroupNames(workgroups(), t().workgroups)}</span>
        </span>
      </span>
      <span class="rec-side">
        <Show when={due()}>
          <span class="rec-when" classList={{ late: !over() && due().slice(0, 10) < day(), soon: !over() && due().slice(0, 10) === day() }} title={dateText(due())}>
            {over() ? dateText(due()) : daysLeft(due(), day())}
            {due().slice(11) ? ` ${due().slice(11)}` : ""}
          </span>
        </Show>
        <Show when={valueOf(t(), "Assignee") !== WORKER} fallback={<span class="tag">{assigneeOf(t(), aide())}</span>}>
          <button
            type="button"
            class={`tag${aides() ? " aide" : ""}`}
            aria-label={aides() ? "自分でやる" : "任せる"}
            title={aides() ? "自分でやる" : `${aide()}に任せる`}
            onClick={() => void put("Assignee", aides() ? OWNER : SECRETARY, aides() ? "自分でやる" : `${aide()}に任せた`)}
          >
            {assigneeOf(t(), aide())}
          </button>
        </Show>
      </span>
    </>
  );
}
