import { createAsync } from "@solidjs/router";
import { For, Show } from "solid-js";
import DreamNow from "~/components/DreamNow";
import DreamLog from "~/components/logs/DreamLog";
import LogSheet from "~/components/logs/LogSheet";
import RoleModel from "~/components/RoleModel";
import { DreamFrequency, type DreamSettings } from "~/gen/famulen/v1/memory_pb";
import { Role } from "~/gen/famulen/v1/settings_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { WEEKDAYS } from "~/lib/dates";
import { dreamSettingsQuery } from "~/lib/memory";
import { setView } from "~/lib/view";
import { Field, NumberField, PageHeader, Section, SegmentedOf, Select, Switch } from "~/components/ui";

const FREQUENCIES: [DreamFrequency, string][] = [
  [DreamFrequency.NIGHTLY, "毎晩"],
  [DreamFrequency.EVERY_OTHER_DAY, "一日おき"],
  [DreamFrequency.WEEKLY, "週に一度"],
];

/** The times offered, every half hour; a time saved otherwise is offered too. */
const TIMES = Array.from({ length: 48 }, (_, i) => `${String(Math.floor(i / 2)).padStart(2, "0")}:${i % 2 ? "30" : "00"}`);

/** How a time reads on the page: 4:00, not 04:00. */
const shownTime = (at: string) => at.replace(/^0(\d)/, "$1");

type Change = Partial<Pick<DreamSettings, "at" | "frequency" | "weekday" | "earlyAt" | "coreLines" | "tidyRecords" | "restAfter">>;

function When(props: { plan: DreamSettings; save: (c: Change) => void }) {
  const p = () => props.plan;
  const times = () => (TIMES.includes(p().at) ? TIMES : [...TIMES, p().at].sort());
  return (
    <Section
      title="いつ見るか"
      note="決めた頻度で、決めた時刻を過ぎたら夢を見る。秘書の会話が少ない夜更けがよい。時刻はワークグループ全体のタイムゾーンで数える。"
    >
      <div class="role-model">
        <Field label="頻度" group>
          <SegmentedOf label="頻度" items={FREQUENCIES} value={p().frequency} onChoose={(frequency) => props.save({ frequency })} />
        </Field>
        <Show when={p().frequency === DreamFrequency.WEEKLY}>
          <Field label="曜日" group>
            <SegmentedOf
              label="曜日"
              items={WEEKDAYS.map((label, i) => [i, label] as const)}
              value={p().weekday}
              onChoose={(weekday) => props.save({ weekday })}
            />
          </Field>
        </Show>
        <Field label="時刻">
          <Select class="fit" onChange={(e) => props.save({ at: e.currentTarget.value })}>
            <For each={times()}>
              {(t) => (
                <option value={t} selected={t === p().at}>
                  {shownTime(t)}
                </option>
              )}
            </For>
          </Select>
        </Field>
        <NumberField
          label="早めに見る"
          unit="件たまったら"
          value={p().earlyAt}
          min={0}
          max={1000}
          onSave={(earlyAt) => props.save({ earlyAt })}
          hint="秘書が書き留めた気づきがこの数たまったら、時刻を待たずに見る。0 にすると早めには見ず、決めた時刻だけに見る。"
        />
        <p class="sub">次は {p().next}</p>
      </div>
    </Section>
  );
}

function Reach(props: { plan: DreamSettings; save: (c: Change) => void }) {
  return (
    <Section
      title="どこまで整えるか"
      note={
        <>
          <a href="/memory/core">心得</a>
          は、秘書が毎回の会話で必ず読む記憶。夢はこれを決めた行数に収め、あふれるときは使われていないものから長期記憶へ移す。
        </>
      }
    >
      <NumberField
        label="記憶の核"
        unit="行まで"
        value={props.plan.coreLines}
        min={1}
        max={1000}
        onSave={(coreLines) => props.save({ coreLines })}
        hint="新しい会話にも、この行数までを渡す。一晩で半分より減らすことはしないので、大きく縮めたときは幾晩かかけて収まる。"
      />
    </Section>
  );
}

function Records(props: { plan: DreamSettings; save: (c: Change) => void }) {
  return (
    <Section
      title="記録を整える"
      note="夢のあとに、することを書いたメモをタスクにし、古くなったかもしれない記録に印を付けて、確かめるタスクを足す。一晩に見るのは少しずつで、書いてあることは変えず、消さない。"
    >
      <Switch checked={props.plan.tidyRecords ?? true} onChange={(tidyRecords) => props.save({ tidyRecords })}>
        記録も整える
      </Switch>
      <NumberField
        label="相手を休みにする"
        unit="日出てこなければ"
        value={props.plan.restAfter ?? 60}
        min={0}
        max={3650}
        onSave={(restAfter) => props.save({ restAfter })}
        hint="秘書が足した人のうち、家族と勤め先の人を除いて、この日数出てこない人を相手の一覧の下に畳む。消さず、また出ると戻る。0 にすると休みにしない。"
      />
    </Section>
  );
}

/** Dreaming: putting the day's memories in order overnight. */
export default function Dream() {
  setView("設定", "夢");
  const plan = createAsync(() => dreamSettingsQuery());
  // Worked out from the settings as they are by then: what was saved a moment
  // ago may not have come back to the page yet.
  const save = (change: Change) =>
    void act(async () => {
      const { at, frequency, weekday, earlyAt, coreLines, tidyRecords, restAfter } = await api.memory.getDreamSettings({});
      return api.memory.setDreamSettings({ at, frequency, weekday, earlyAt, coreLines, tidyRecords, restAfter, ...change });
    });
  return (
    <>
      <PageHeader
        title="夢"
        note={
          <>
            <p>
              その日に秘書が書き留めたことと元の会話を読み直し、近い記憶と比べて、足す・直す・置き換える・忘れるを選んで記憶を整えるエージェント。秘書の会話とは別のモデルで動く。
            </p>
            <p>
              夜ごとに何を変えたかは、<a href="/memory">記憶</a>の夢で読む。
            </p>
          </>
        }
      />
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
        <Section title="今すぐ見る" note="決めた時を待たずに、いま一度だけ夢を見る。次の決めた時は変わらない。">
          <DreamNow />
        </Section>
      </div>
      <LogSheet title="夢のログ">
        <DreamLog />
      </LogSheet>
    </>
  );
}
