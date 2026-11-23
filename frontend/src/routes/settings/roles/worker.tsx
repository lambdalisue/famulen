import { Switch } from "@ark-ui/solid/switch";
import { NumberInput } from "@ark-ui/solid/number-input";
import { A, createAsync } from "@solidjs/router";
import { For, Show } from "solid-js";
import LogSheet from "~/components/logs/LogSheet";
import { RunResult } from "~/components/records/Delegated";
import RoleModel from "~/components/RoleModel";
import { Role } from "~/gen/famulen/v1/settings_pb";
import type { WorkerSettings } from "~/gen/famulen/v1/workers_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { createDraft } from "~/lib/draft";
import { setView } from "~/lib/view";
import { delegatedQuery, workerSettingsQuery } from "~/lib/workers";

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
  const [text, setText] = createDraft(() => String(s().slots));
  return (
    <section class="pref">
      <div class="pref-head">
        <h2>同時に動く数</h2>
        <p class="sub">
          委ねたタスクは、優先度、期日、古さの順に、ここで決めた数までを同時に動かす。あなたの承認や答えを待っているものは数に入らない。並びは
          <A href="/tasks">タスク</A>の頁で見られる。
        </p>
      </div>
      <NumberInput.Root
        class="role-field"
        min={1}
        max={20}
        value={text()}
        onValueChange={(d) => setText(d.value)}
        onValueCommit={(d) => {
          const n = d.valueAsNumber;
          if (!Number.isInteger(n)) setText(String(s().slots));
          else if (n !== s().slots) save({ slots: n });
        }}
      >
        <NumberInput.Label>同時に動く数</NumberInput.Label>
        <div class="role-cap">
          <NumberInput.Control>
            <NumberInput.Input class="input" />
          </NumberInput.Control>
          <span class="sub">件</span>
        </div>
      </NumberInput.Root>
      <Switch.Root class="row" checked={s().paused} onCheckedChange={(d) => save({ paused: d.checked })}>
        <Switch.Control class="switch">
          <Switch.Thumb class="switch-thumb" />
        </Switch.Control>
        <Switch.Label>一時停止する（動いているものは続く）</Switch.Label>
        <Switch.HiddenInput />
      </Switch.Root>
    </section>
  );
}

function Ended() {
  const delegated = createAsync(() => delegatedQuery());
  return (
    <div class="list">
      <For each={delegated()?.ended} fallback={<p class="empty">まだない</p>}>
        {(r) => <RunResult run={r} />}
      </For>
    </div>
  );
}

/** Workers: what does the tasks that were delegated. */
export default function Worker() {
  setView("設定", "手代");
  const settings = createAsync(() => workerSettingsQuery());
  return (
    <>
      <h1>手代</h1>
      <p class="sub">
        秘書やあなたがタスクとして委ねた仕事を、一つずつ片付けるエージェント。仕事ごとに生まれ、渡された道具と手帳のファイルだけを使い、済めば消える。あなたに確かめる道具は使えない。Adroit
        に委ねた仕事は、あなたの承認を得てから Adroit のセッションとして始まる。
      </p>
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
