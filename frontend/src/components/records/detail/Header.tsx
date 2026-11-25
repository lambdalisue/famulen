import { createAsync } from "@solidjs/router";
import { createEffect, For, type JSX, onCleanup, onMount } from "solid-js";
import AskAbout from "~/components/AskAbout";
import { Icon, IconButton, Segmented, Tag, TextField } from "~/components/ui";
import { say } from "~/lib/act";
import { workgroupsQuery } from "~/lib/data";
import * as icons from "~/lib/icons";
import { KIND_NAMES, type Rec } from "~/lib/records";
import type { SaveRecord } from "./useRecordEdit";

export const copy = (text: string, what: string) => void navigator.clipboard?.writeText(text).then(() => say(`${what}を写した`));

/** The title, as long as it is, changed where it stands. */
function Title(props: { title: string; onSave: (title: string) => void }) {
  let box!: HTMLTextAreaElement;
  const fit = () => {
    box.style.height = "auto";
    box.style.height = `${box.scrollHeight}px`;
  };
  createEffect(() => {
    box.value = props.title;
    fit();
  });
  onMount(() => {
    const watch = new ResizeObserver(fit);
    if (box.parentElement) watch.observe(box.parentElement);
    onCleanup(() => watch.disconnect());
  });
  return (
    <textarea
      ref={box}
      class="rec-name"
      rows="1"
      aria-label="題名"
      onInput={fit}
      onKeyDown={(e) => {
        if (e.isComposing) return;
        if (e.key === "Enter") e.preventDefault();
        if (e.key === "Escape") box.value = props.title;
        if (e.key === "Enter" || e.key === "Escape") box.blur();
      }}
      onChange={() => {
        const title = box.value.replace(/\s*\n\s*/g, " ").trim();
        if (title && title !== props.title) props.onSave(title);
        else box.value = props.title;
        fit();
      }}
    >
      {props.title}
    </textarea>
  );
}

function Labels(props: { record: Rec; save: SaveRecord }) {
  const set = (change: (labels: string[]) => string[]) => void props.save((now) => ({ labels: change(now.labels), setLabels: true }));
  return (
    <>
      <For each={props.record.labels}>
        {(l) => (
          <Tag tone="label" class="removable">
            {l}
            <button type="button" aria-label={`ラベル ${l} を外す`} onClick={() => set((labels) => labels.filter((x) => x !== l))}>
              <Icon svg={icons.x} size={12} />
            </button>
          </Tag>
        )}
      </For>
      <TextField
        class="label-add"
        placeholder="ラベルを足す"
        aria-label="ラベルを足す"
        onKeyDown={(e) => {
          if (e.key !== "Enter" || e.isComposing) return;
          e.preventDefault();
          const l = e.currentTarget.value.trim();
          if (l) set((labels) => (labels.includes(l) ? labels : [...labels, l]));
          e.currentTarget.value = "";
        }}
      />
    </>
  );
}

function Workgroups(props: { record: Rec; save: SaveRecord }) {
  const workgroups = createAsync(() => workgroupsQuery());
  return (
    <span class="chips-line" role="group" aria-label="入るワークグループ">
      <For each={(workgroups() ?? []).filter((w) => w.id !== "all")}>
        {(w) => {
          const on = () => props.record.workgroups.includes(w.id);
          return (
            <button
              type="button"
              class="tag wg"
              aria-pressed={on()}
              onClick={() =>
                void props.save((now) => ({
                  workgroups: now.workgroups.includes(w.id) ? now.workgroups.filter((x) => x !== w.id) : [...now.workgroups, w.id],
                  setWorkgroups: true,
                }))
              }
            >
              <span class="dot" style={{ background: on() ? w.color : "var(--line-strong)" }} />
              {w.name}
            </button>
          );
        }}
      </For>
    </span>
  );
}

/** What a record is called, what it is tagged with, and what is done with it whole. */
export default function Header(props: {
  record: Rec;
  view: string;
  onView: (v: string) => void;
  save: SaveRecord;
  onRemove: () => void;
  lead?: JSX.Element;
}) {
  const r = () => props.record;
  return (
    <header class="rec-title">
      <div class="row">
        <button
          type="button"
          class="tag mono"
          title="呼び名を写す"
          aria-label={`呼び名 ${r().code} を写す`}
          onClick={() => copy(r().code, "呼び名")}
        >
          {r().code}
          <Icon svg={icons.copy} size={11} />
        </button>
        <span class="grow" />
        <Segmented
          label="見方"
          items={[
            ["view", "内容"],
            ["raw", "原文"],
          ]}
          value={props.view}
          onChoose={props.onView}
        />
        <AskAbout framed what={`${KIND_NAMES[r().kind] ?? r().kind}：${r().title}（${r().code}）`} />
        <IconButton variant="danger" icon={icons.trash} label={`${r().title}を消す`} onClick={() => props.onRemove()} />
      </div>
      <div class="rec-named">
        {props.lead}
        <Title title={r().title} onSave={(title) => void props.save({ title })} />
      </div>
      <div class="rec-tags">
        <span class="chips-edit">
          <Labels record={r()} save={props.save} />
        </span>
        <Workgroups record={r()} save={props.save} />
      </div>
    </header>
  );
}
