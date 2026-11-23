import { NumberInput } from "@ark-ui/solid/number-input";
import { SegmentGroup } from "@ark-ui/solid/segment-group";
import { Select, createListCollection } from "@ark-ui/solid/select";
import { Switch } from "@ark-ui/solid/switch";
import { A, createAsync } from "@solidjs/router";
import { For, Show, createMemo } from "solid-js";
import { Portal } from "solid-js/web";
import DreamNow from "~/components/DreamNow";
import DreamLog from "~/components/logs/DreamLog";
import LogSheet from "~/components/logs/LogSheet";
import Icon from "~/components/Icon";
import RoleModel from "~/components/RoleModel";
import { DreamFrequency, type DreamSettings } from "~/gen/famulen/v1/memory_pb";
import { Role } from "~/gen/famulen/v1/settings_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { createDraft } from "~/lib/draft";
import { check, chevronDown } from "~/lib/icons";
import { dreamSettingsQuery } from "~/lib/memory";
import { setView } from "~/lib/view";

const FREQUENCIES: [DreamFrequency, string, string][] = [
  [DreamFrequency.NIGHTLY, "nightly", "毎晩"],
  [DreamFrequency.EVERY_OTHER_DAY, "other", "一日おき"],
  [DreamFrequency.WEEKLY, "weekly", "週に一度"],
];

const WEEKDAYS = ["日", "月", "火", "水", "木", "金", "土"];

/** The times offered, every half hour; a time saved otherwise is offered too. */
const TIMES = Array.from({ length: 48 }, (_, i) => `${String(Math.floor(i / 2)).padStart(2, "0")}:${i % 2 ? "30" : "00"}`);

/** How a time reads on the page: 4:00, not 04:00. */
const shownTime = (at: string) => at.replace(/^0(\d)/, "$1");

type Change = Partial<Pick<DreamSettings, "at" | "frequency" | "weekday" | "earlyAt" | "coreLines" | "tidyRecords">>;

/** A count saved when its field is left; an empty or broken field goes back. */
function Count(props: {
  label: string;
  unit: string;
  value: number;
  min: number;
  max: number;
  hint: string;
  onSave: (n: number) => void;
}) {
  const [text, setText] = createDraft(() => String(props.value));
  return (
    <NumberInput.Root
      class="role-field"
      min={props.min}
      max={props.max}
      value={text()}
      onValueChange={(d) => setText(d.value)}
      onValueCommit={(d) => {
        const n = d.valueAsNumber;
        if (!Number.isInteger(n)) setText(String(props.value));
        else if (n !== props.value) props.onSave(n);
      }}
    >
      <NumberInput.Label>{props.label}</NumberInput.Label>
      <div class="role-cap">
        <NumberInput.Control>
          <NumberInput.Input class="input" />
        </NumberInput.Control>
        <span class="sub">{props.unit}</span>
      </div>
      <span class="sub">{props.hint}</span>
    </NumberInput.Root>
  );
}

function When(props: { plan: DreamSettings; save: (c: Change) => void }) {
  const p = () => props.plan;
  const times = createMemo(() =>
    createListCollection({
      items: (TIMES.includes(p().at) ? TIMES : [...TIMES, p().at].sort()).map((t) => ({
        label: shownTime(t),
        value: t,
      })),
    }),
  );
  return (
    <section class="pref">
      <div class="pref-head">
        <h2>いつ見るか</h2>
        <p class="sub">決めた頻度で、決めた時刻を過ぎたら夢を見る。秘書の会話が少ない夜更けがよい。時刻はワークグループ全体のタイムゾーンで数える。</p>
      </div>
      <div class="role-model">
        <div class="role-field">
          <span class="role-label">頻度</span>
          <SegmentGroup.Root
            class="segs"
            aria-label="頻度"
            value={FREQUENCIES.find(([f]) => f === p().frequency)?.[1] ?? "nightly"}
            onValueChange={(d) => {
              const f = FREQUENCIES.find(([, key]) => key === d.value)?.[0];
              if (f !== undefined && f !== p().frequency) props.save({ frequency: f });
            }}
          >
            <SegmentGroup.Indicator class="segs-indicator" />
            <For each={FREQUENCIES}>
              {([, key, label]) => (
                <SegmentGroup.Item value={key} class="segs-item">
                  <SegmentGroup.ItemText>{label}</SegmentGroup.ItemText>
                  <SegmentGroup.ItemControl />
                  <SegmentGroup.ItemHiddenInput />
                </SegmentGroup.Item>
              )}
            </For>
          </SegmentGroup.Root>
        </div>
        <Show when={p().frequency === DreamFrequency.WEEKLY}>
          <div class="role-field">
            <span class="role-label">曜日</span>
            <SegmentGroup.Root
              class="segs"
              aria-label="曜日"
              value={String(p().weekday)}
              onValueChange={(d) => {
                const day = Number(d.value);
                if (Number.isInteger(day) && day !== p().weekday) props.save({ weekday: day });
              }}
            >
              <SegmentGroup.Indicator class="segs-indicator" />
              <For each={WEEKDAYS}>
                {(label, i) => (
                  <SegmentGroup.Item value={String(i())} class="segs-item">
                    <SegmentGroup.ItemText>{label}</SegmentGroup.ItemText>
                    <SegmentGroup.ItemControl />
                    <SegmentGroup.ItemHiddenInput />
                  </SegmentGroup.Item>
                )}
              </For>
            </SegmentGroup.Root>
          </div>
        </Show>
        <Select.Root
          class="role-field"
          collection={times()}
          value={[p().at]}
          onValueChange={(d) => {
            const at = d.value[0];
            if (at && at !== p().at) props.save({ at });
          }}
          positioning={{ sameWidth: true }}
        >
          <Select.Label>時刻</Select.Label>
          <Select.Control>
            <Select.Trigger class="role-select-trigger dream-time">
              <Select.ValueText />
              <Icon svg={chevronDown} size={14} />
            </Select.Trigger>
          </Select.Control>
          <Portal>
            <Select.Positioner>
              <Select.Content class="menu-list dream-times">
                <For each={times().items}>
                  {(item) => (
                    <Select.Item item={item} class="menu-item">
                      <Select.ItemText>{item.label}</Select.ItemText>
                      <Select.ItemIndicator>
                        <Icon svg={check} size={14} />
                      </Select.ItemIndicator>
                    </Select.Item>
                  )}
                </For>
              </Select.Content>
            </Select.Positioner>
          </Portal>
          <Select.HiddenSelect />
        </Select.Root>
        <Count
          label="早めに見る"
          unit="件たまったら"
          value={p().earlyAt}
          min={0}
          max={1000}
          onSave={(earlyAt) => props.save({ earlyAt })}
          hint="秘書が書き留めた気づきがこの数たまったら、時刻を待たずに見る。0 にすると早めには見ず、決めた時刻だけに見る。"
        />
        <p class="sub dream-next">次は {p().next}</p>
      </div>
    </section>
  );
}

function Reach(props: { plan: DreamSettings; save: (c: Change) => void }) {
  return (
    <section class="pref">
      <div class="pref-head">
        <h2>どこまで整えるか</h2>
        <p class="sub">
          <A href="/memory/core">心得</A>
          は、秘書が毎回の会話で必ず読む記憶。夢はこれを決めた行数に収め、あふれるときは使われていないものから長期記憶へ移す。
        </p>
      </div>
      <Count
        label="記憶の核"
        unit="行まで"
        value={props.plan.coreLines}
        min={1}
        max={1000}
        onSave={(coreLines) => props.save({ coreLines })}
        hint="新しい会話にも、この行数までを渡す。一晩で半分より減らすことはしないので、大きく縮めたときは幾晩かかけて収まる。"
      />
    </section>
  );
}

function Records(props: { plan: DreamSettings; save: (c: Change) => void }) {
  return (
    <section class="pref">
      <div class="pref-head">
        <h2>記録を整える</h2>
        <p class="sub">
          夢のあとに、することを書いたメモをタスクにし、古くなったかもしれない記録に印を付けて、確かめるタスクを足す。一晩に見るのは少しずつで、書いてあることは変えず、消さない。
        </p>
      </div>
      <Switch.Root
        class="role-field row"
        checked={props.plan.tidyRecords ?? true}
        onCheckedChange={(d) => props.save({ tidyRecords: d.checked })}
      >
        <Switch.Label class="grow">記録も整える</Switch.Label>
        <Switch.Control class="switch">
          <Switch.Thumb class="switch-thumb" />
        </Switch.Control>
        <Switch.HiddenInput />
      </Switch.Root>
    </section>
  );
}

/** Dreaming: putting the day's memories in order overnight. */
export default function Dream() {
  setView("設定 · 夢");
  const plan = createAsync(() => dreamSettingsQuery());
  const save = (change: Change) => {
    const p = plan();
    if (!p) return;
    void act(() =>
      api.memory.setDreamSettings({
        at: p.at,
        frequency: p.frequency,
        weekday: p.weekday,
        earlyAt: p.earlyAt,
        coreLines: p.coreLines,
        tidyRecords: p.tidyRecords,
        ...change,
      }),
    );
  };
  return (
    <>
      <h1>夢</h1>
      <p class="sub">
        その日に秘書が書き留めたことと元の会話を読み直し、近い記憶と比べて、足す・直す・置き換える・忘れるを選んで記憶を整えるエージェント。秘書の会話とは別のモデルで動く。
      </p>
      <p class="sub">
        夜ごとに何を変えたかは、<A href="/memory">記憶</A>の夢で読む。
      </p>
      <div class="prefs">
        <RoleModel
          role={Role.DREAM}
          cap="上限に達した日は、それ以上夢を見ず、次の日に回す。値段のあるモデルは円で、ないモデルは回数で数える。"
        />
        <Show when={plan()}>
          {(p) => (
            <>
              <When plan={p()} save={save} />
              <Reach plan={p()} save={save} />
              <Records plan={p()} save={save} />
            </>
          )}
        </Show>
        <section class="pref">
          <div class="pref-head">
            <h2>今すぐ見る</h2>
            <p class="sub">決めた時を待たずに、いま一度だけ夢を見る。次の決めた時は変わらない。</p>
          </div>
          <DreamNow />
        </section>
      </div>
      <LogSheet title="夢のログ">
        <DreamLog />
      </LogSheet>
    </>
  );
}
