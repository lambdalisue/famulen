import { NumberInput } from "@ark-ui/solid/number-input";
import { createAsync } from "@solidjs/router";
import { For, Show } from "solid-js";
import Segs from "~/components/Segs";
import { type LookoutDefault, type LookoutPlace, LookoutRoute } from "~/gen/famulen/v1/lookout_pb";
import { workgroupsQuery } from "~/lib/data";

/** Whether the default interval is used or overridden, in a dialog that saves it later. */
export function IntervalField(props: {
  defaults: LookoutDefault[];
  own: boolean;
  setOwn: (own: boolean) => void;
  minutes: string;
  setMinutes: (minutes: string) => void;
}) {
  const defaults = () => props.defaults.map((d) => `${d.label} ${d.intervalMinutes} 分`).join("・");
  return (
    <div class="dialog-field">
      <span>間隔</span>
      <div class="lookout-interval-wrap">
        <Segs
          label="間隔の決め方"
          items={[
            ["default", "既定"],
            ["own", "上書き"],
          ]}
          value={props.own ? "own" : "default"}
          onChoose={(way) => props.setOwn(way === "own")}
        />
        <Show when={props.own}>
          <NumberInput.Root
            class="lookout-interval"
            min={1}
            required
            value={props.minutes}
            onValueChange={(d) => props.setMinutes(d.value)}
          >
            <NumberInput.Label class="visually-hidden">間隔（分）</NumberInput.Label>
            <NumberInput.Control>
              <NumberInput.Input ref={(el) => queueMicrotask(() => el.focus())} class="input" inputmode="numeric" />
            </NumberInput.Control>
            <span class="sub">分ごと</span>
          </NumberInput.Root>
        </Show>
      </div>
      <small class="sub">既定は、{defaults()}。</small>
    </div>
  );
}

/** The routes an owner can choose, as the list and the dialog name them. */
export const ROUTES: [LookoutRoute, string, string][] = [
  [LookoutRoute.SECRETARY, "秘書へ", "関わるものは、いつもすぐ秘書に渡す。"],
  [LookoutRoute.DIGEST, "朝のまとめ", "関わるものは、いつも朝のまとめに回す。"],
  [LookoutRoute.LOG, "ログだけ", "関わるものも、見張りのログに残すだけで、誰にも渡さない。"],
];

/** Whether the lookout decides where what concerns the owner goes, or it always goes to one place. */
export function RouteField(props: { route: LookoutRoute; setRoute: (route: LookoutRoute) => void }) {
  const own = () => props.route !== LookoutRoute.UNSPECIFIED;
  return (
    <div class="dialog-field">
      <span>どこへ</span>
      <div class="lookout-interval-wrap">
        <Segs
          label="どこへの決め方"
          items={[
            ["default", "見張りが決める"],
            ["own", "上書き"],
          ]}
          value={own() ? "own" : "default"}
          onChoose={(way) => props.setRoute(way === "own" ? LookoutRoute.SECRETARY : LookoutRoute.UNSPECIFIED)}
        />
        <Show when={own()}>
          <Segs
            label="どこへ"
            items={ROUTES.map(([r, label]) => [String(r), label])}
            value={String(props.route)}
            onChoose={(value) => {
              const chosen = ROUTES.find(([r]) => String(r) === value);
              if (chosen) props.setRoute(chosen[0]);
            }}
          />
        </Show>
      </div>
      <small class="sub">
        {ROUTES.find(([r]) => r === props.route)?.[2] ?? "急ぎと中身を見て、秘書か朝のまとめかをそのつど決める。"}
        {own() ? "関わるかどうかは、これまでどおり見張りが決める。" : ""}
      </small>
    </div>
  );
}

/** The places a source is narrowed to, kept even when the service no longer lists one. */
export function PlacesField(props: {
  label: string;
  choices: LookoutPlace[];
  /** Why whatever arrives cannot be chosen: another source reads it. */
  restReader?: string;
  narrowed: boolean;
  setNarrowed: (narrowed: boolean) => void;
  places: LookoutPlace[];
  setPlaces: (places: LookoutPlace[]) => void;
}) {
  const listed = () => [...props.choices, ...props.places.filter((p) => !props.choices.some((c) => c.id === p.id))];
  const on = (id: string) => props.places.some((p) => p.id === id);
  return (
    <div class="dialog-field">
      <span>{props.label}</span>
      <Show
        when={!props.restReader}
        fallback={
          <small class="sub">
            {props.label}を選ばないものは「{props.restReader}」が見張っている。ここで選んだものだけを、こちらで見張る。
          </small>
        }
      >
        <Segs
          label={`${props.label}の決め方`}
          items={[
            ["all", "すべて"],
            ["some", "選ぶ"],
          ]}
          value={props.narrowed ? "some" : "all"}
          onChoose={(way) => props.setNarrowed(way === "some")}
        />
        <small class="sub">「すべて」は、ほかの見張りが選んでいないものをすべて読む。</small>
      </Show>
      <Show when={props.narrowed}>
        <div class="lookout-places" role="group" aria-label={props.label}>
          <For each={listed()}>
            {(p) => (
              <label>
                <input
                  type="checkbox"
                  checked={on(p.id)}
                  onChange={(e) =>
                    props.setPlaces(e.currentTarget.checked ? [...props.places, p] : props.places.filter((q) => q.id !== p.id))
                  }
                />
                {p.name}
              </label>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

/** The built-in workgroup that holds everything. */
export const WHOLE = "all";

export function WorkgroupField(props: { workgroup: string; setWorkgroup: (id: string) => void }) {
  const workgroups = createAsync(() => workgroupsQuery());
  return (
    <label class="dialog-field">
      <span>ワークグループ</span>
      <select class="input" onChange={(e) => props.setWorkgroup(e.currentTarget.value)}>
        <option value={WHOLE} selected={props.workgroup === WHOLE}>
          全体
        </option>
        <For each={(workgroups() ?? []).filter((g) => g.id !== WHOLE)}>
          {(g) => (
            <option value={g.id} selected={props.workgroup === g.id}>
              {g.name}
            </option>
          )}
        </For>
      </select>
      <small class="sub">拾ったものを、このワークグループのこととして扱う。</small>
    </label>
  );
}
