import { createAsync } from "@solidjs/router";
import { For, Show } from "solid-js";
import LogSheet from "~/components/logs/LogSheet";
import { RunResult } from "~/components/records/Delegated";
import RoleModel from "~/components/RoleModel";
import { Role } from "~/gen/famulen/v1/settings_pb";
import type { WorkerSettings } from "~/gen/famulen/v1/workers_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { setView } from "~/lib/view";
import { delegatedQuery, workerSettingsQuery } from "~/lib/workers";
import { EmptyState, List, NumberField, PageHeader, Section, Switch } from "~/components/ui";

/** How many delegated tasks run at once, and whether any is started. */
function Pool(props: { settings: WorkerSettings }) {
  const s = () => props.settings;
  let saving: Promise<unknown> = Promise.resolve();
  // One after another over what is saved by then: what is on screen may be older.
  const save = (change: Partial<Pick<WorkerSettings, "slots" | "paused">>) => {
    saving = saving.then(() =>
      act(async () => {
        const { slots, paused } = await api.workers.getWorkerSettings({});
        return api.workers.setWorkerSettings({ slots, paused, ...change });
      }),
    );
  };
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
      <NumberField label="同時に動く数" min={1} max={20} value={s().slots} unit="件" onSave={(slots) => save({ slots })} />
      <Switch checked={s().paused} onChange={(paused) => save({ paused })}>
        一時停止する（動いているものは続く）
      </Switch>
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
            秘書やあなたがタスクとして委ねた仕事を、一つずつ片付けるエージェント。仕事ごとに生まれ、渡された道具と手帳のファイルだけを使い、済めば消える。あなたに確かめる道具は使えない。Adroit
            に委ねた仕事は、あなたの承認を得てから Adroit のセッションとして始まる。
          </>
        }
      />
      <div class="prefs">
        <RoleModel
          role={Role.WORKER}
          cap="使わないにすると、秘書と同じモデルで、一日に決まった数まで動く。上限に達した日は、手代に委ねた仕事は次の日まで待つ。値段のあるモデルは円で、ないモデルは回数で数える。"
        />
        <Show when={settings()}>{(s) => <Pool settings={s()} />}</Show>
      </div>
      <LogSheet title="済んだ仕事" label="済んだ仕事を見る">
        <Ended />
      </LogSheet>
    </>
  );
}
