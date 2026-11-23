import { NumberInput } from "@ark-ui/solid/number-input";
import { Select, createListCollection } from "@ark-ui/solid/select";
import { Slider } from "@ark-ui/solid/slider";
import { createAsync } from "@solidjs/router";
import { For, type JSX, Show, createEffect, createMemo, createSignal } from "solid-js";
import { Portal } from "solid-js/web";
import Icon from "~/components/Icon";
import { type Model, ModelKind, type Role, type RoleSetting } from "~/gen/famulen/v1/settings_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { createDraft } from "~/lib/draft";
import { chevronDown, check } from "~/lib/icons";
import { modelsQuery, yen } from "~/lib/models";

const NONE = "none";

type Choice = { label: string; value: string };

const capText = (cap: number) => (cap > 0 ? String(cap) : "");

/** The stops of 考える深さ, as `claude --effort` names them, shallow to deep. */
const EFFORTS = [
  { value: "low", label: "浅い", means: "いちばん速く、使う量も少ない。決まった形の短い判断に向く。" },
  { value: "medium", label: "ふつう", means: "速さと丁寧さの釣り合いがよい。たいていの仕事はこれで足りる。" },
  { value: "high", label: "深い", means: "込み入った頼みも筋道を立てて考える。そのぶん遅く、多く使う。" },
  { value: "xhigh", label: "とても深い", means: "時間をかけて考え直す。難しい仕事向けで、使う量はかなり増える。" },
  { value: "max", label: "最大", means: "考えられるだけ考える。いちばん遅く、使う量もいちばん多い。" },
] as const;

/** Where the thumb rests while the model's own default applies. */
const DEFAULT_STOP = 2;

/** How deep the role's model thinks; saved when the thumb is let go. */
function Effort(props: { effort: string; fallback: string; onChange: (effort: string) => void }) {
  const stop = () => EFFORTS.findIndex((e) => e.value === props.effort);
  const [shown, setShown] = createSignal(stop());
  createEffect(() => setShown(stop()));
  const current = () => EFFORTS[shown()];
  return (
    <div class="role-field effort">
      <Slider.Root
        class="effort-slider"
        data-default={shown() < 0 ? "" : undefined}
        min={0}
        max={EFFORTS.length - 1}
        step={1}
        value={[shown() < 0 ? DEFAULT_STOP : shown()]}
        getAriaValueText={(d) => (shown() < 0 ? "モデルの既定" : EFFORTS[d.value]?.label ?? "")}
        onValueChange={(d) => setShown(d.value[0] ?? DEFAULT_STOP)}
        onValueChangeEnd={(d) => {
          const next = EFFORTS[d.value[0] ?? DEFAULT_STOP]?.value ?? "";
          if (next !== props.effort) props.onChange(next);
        }}
      >
        <div class="effort-head">
          <Slider.Label>考える深さ</Slider.Label>
          <span class="effort-value">{current()?.label ?? "モデルの既定"}</span>
          <Show when={props.effort !== props.fallback}>
            <button
              type="button"
              class="btn ghost small effort-reset"
              title={`このエージェントの既定は「${EFFORTS.find((e) => e.value === props.fallback)?.label ?? "モデルの既定"}」`}
              onClick={() => props.onChange(props.fallback)}
            >
              既定に戻す
            </button>
          </Show>
        </div>
        <Slider.Control class="effort-control">
          <Slider.Track class="effort-track">
            <Slider.Range class="effort-range" />
          </Slider.Track>
          <Slider.Thumb index={0} class="effort-thumb">
            <Slider.HiddenInput />
          </Slider.Thumb>
        </Slider.Control>
        <Slider.MarkerGroup class="effort-marks">
          <For each={EFFORTS}>
            {(e, i) => (
              <Slider.Marker value={i()} class="effort-mark">
                {e.label}
              </Slider.Marker>
            )}
          </For>
        </Slider.MarkerGroup>
        <div class="effort-ends sub">
          <span>速く・安く</span>
          <span>じっくり・高く</span>
        </div>
      </Slider.Root>
      <p class="sub effort-means">
        {current()?.means ?? "モデルに任せる。どの深さで考えるかは、モデルとその実行ファイルの既定のまま。"}
      </p>
    </div>
  );
}

function Section(props: { setting: RoleSetting; models: Model[]; cap: string; children?: JSX.Element }) {
  const r = () => props.setting;
  const collection = createMemo(() =>
    createListCollection<Choice>({
      items: [
        { label: "使わない", value: NONE },
        ...props.models.filter((m) => r().choices.includes(m.id)).map((m) => ({ label: m.name, value: m.id })),
      ],
    }),
  );
  const assign = (model: string, cap: number, effort = r().effort) =>
    act(() => api.models.assignRole({ role: r().role, model, cap, effort }));
  const kind = () => props.models.find((m) => m.id === r().model)?.kind;

  const [cap, setCap] = createDraft(() => capText(r().cap));

  return (
    <section class="pref">
      <div class="pref-head">
        <h2>モデル</h2>
        <p class="sub">考えるのに使うモデルと、その使い方。</p>
      </div>
      <div class="role-model">
        <Select.Root
          class="role-field"
          collection={collection()}
          value={[r().model || NONE]}
          onValueChange={(d) => {
            const v = d.value[0] ?? NONE;
            const model = v === NONE ? "" : v;
            if (model !== r().model) void assign(model, r().cap);
          }}
          positioning={{ sameWidth: true }}
        >
          <Select.Label>使うモデル</Select.Label>
          <Select.Control>
            <Select.Trigger class="role-select-trigger">
              <Select.ValueText placeholder="使わない" />
              <Icon svg={chevronDown} size={14} />
            </Select.Trigger>
          </Select.Control>
          <Portal>
            <Select.Positioner>
              <Select.Content class="menu-list">
                <For each={collection().items}>
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
        <Show when={kind() === ModelKind.AGENT_CLI}>
          <Effort effort={r().effort} fallback={r().defaultEffort} onChange={(effort) => void assign(r().model, r().cap, effort)} />
        </Show>
        <NumberInput.Root
          class="role-field"
          min={0}
          value={cap()}
          onValueChange={(d) => setCap(d.value)}
          onValueCommit={(d) => {
            const next = Number.isFinite(d.valueAsNumber) && d.valueAsNumber > 0 ? d.valueAsNumber : 0;
            if (next !== r().cap) void assign(r().model, next);
          }}
        >
          <NumberInput.Label>一日の上限</NumberInput.Label>
          <div class="role-cap">
            <NumberInput.Control>
              <NumberInput.Input class="input" placeholder="なし" />
            </NumberInput.Control>
            <span class="sub">{r().capUnit}</span>
            <span class="sub role-today">
              今日 {r().yenToday > 0 ? `${yen(r().yenToday)} · ` : ""}
              {String(r().callsToday)} 回
            </span>
          </div>
          <span class="sub">{props.cap}</span>
        </NumberInput.Root>
      </div>
      {props.children}
    </section>
  );
}

/**
 * Which model a role runs on, how deep it thinks when the model can be told,
 * and how much of it a day may use.
 */
export default function RoleModel(props: { role: Role; cap: string; children?: JSX.Element }) {
  const models = createAsync(() => modelsQuery());
  const setting = () => models()?.roles.find((r) => r.role === props.role);
  return (
    <Show when={setting()}>
      {(s) => (
        <Section setting={s()} models={models()?.models ?? []} cap={props.cap}>
          {props.children}
        </Section>
      )}
    </Show>
  );
}
