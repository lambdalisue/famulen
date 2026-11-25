import { For, Show } from "solid-js";
import { type LookoutDefault, type LookoutPlace, LookoutRoute } from "~/gen/famulen/v1/lookout_pb";
import { useWorkgroups } from "~/lib/workgroups";
import { Checkbox, Field, Segmented, Select, TextField } from "~/components/ui";

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
    <Field label="間隔" hint={<>既定は、{defaults()}。</>} group>
      <div class="lookout-interval-wrap">
        <Segmented
          label="間隔の決め方"
          items={[
            ["default", "既定"],
            ["own", "上書き"],
          ]}
          value={props.own ? "own" : "default"}
          onChoose={(way) => props.setOwn(way === "own")}
        />
        <Show when={props.own}>
          <span class="row">
            <TextField
              class="number"
              type="number"
              inputmode="numeric"
              min="1"
              required
              aria-label="間隔（分）"
              value={props.minutes}
              onInput={(e) => props.setMinutes(e.currentTarget.value)}
              ref={(el) => queueMicrotask(() => el.focus())}
            />
            <span class="sub">分ごと</span>
          </span>
        </Show>
      </div>
    </Field>
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
    <Field
      label="どこへ"
      hint={
        <>
          {ROUTES.find(([r]) => r === props.route)?.[2] ?? "急ぎと中身を見て、秘書か朝のまとめかをそのつど決める。"}
          {own() ? "関わるかどうかは、これまでどおり見張りが決める。" : ""}
        </>
      }
      group
    >
      <div class="lookout-interval-wrap">
        <Segmented
          label="どこへの決め方"
          items={[
            ["default", "見張りが決める"],
            ["own", "上書き"],
          ]}
          value={own() ? "own" : "default"}
          onChoose={(way) => props.setRoute(way === "own" ? LookoutRoute.SECRETARY : LookoutRoute.UNSPECIFIED)}
        />
        <Show when={own()}>
          <Segmented
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
    </Field>
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
    <Field label={props.label} group>
      <Show
        when={!props.restReader}
        fallback={
          <small class="sub">
            {props.label}を選ばないものは「{props.restReader}」が見張っている。ここで選んだものだけを、こちらで見張る。
          </small>
        }
      >
        <Segmented
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
              <Checkbox
                checked={on(p.id)}
                onChange={(e) =>
                  props.setPlaces(e.currentTarget.checked ? [...props.places, p] : props.places.filter((q) => q.id !== p.id))
                }
              >
                {p.name}
              </Checkbox>
            )}
          </For>
        </div>
      </Show>
    </Field>
  );
}

/** The built-in workgroup that holds everything. */
export const WHOLE = "all";

export function WorkgroupField(props: { workgroup: string; setWorkgroup: (id: string) => void }) {
  const workgroups = useWorkgroups();
  return (
    <Field label="ワークグループ" hint="拾ったものを、このワークグループのこととして扱う。">
      <Select onChange={(e) => props.setWorkgroup(e.currentTarget.value)}>
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
      </Select>
    </Field>
  );
}
