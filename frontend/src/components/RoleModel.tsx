import { Slider } from "@ark-ui/solid/slider";
import { createAsync } from "@solidjs/router";
import { For, type JSX, Show, createEffect, createSignal } from "solid-js";
import { type Model as RegisteredModel, ModelKind, type Role, type RoleSetting } from "~/gen/famulen/v1/settings_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { modelsQuery, yen } from "~/lib/models";
import { Button, Field, NumberField, Section, Select } from "~/components/ui";

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
    <div class="field effort">
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
            <Button variant="ghost" small class="effort-reset" title={`このエージェントの既定は「${EFFORTS.find((e) => e.value === props.fallback)?.label ?? "モデルの既定"}」`} onClick={() => props.onChange(props.fallback)}>
              既定に戻す
            </Button>
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

function Model(props: { setting: RoleSetting; models: RegisteredModel[]; cap: string; children?: JSX.Element }) {
  const r = () => props.setting;
  const choices = () => props.models.filter((m) => r().choices.includes(m.id));
  const assign = (model: string, cap: number, effort = r().effort) =>
    act(() => api.models.assignRole({ role: r().role, model, cap, effort }));
  const kind = () => props.models.find((m) => m.id === r().model)?.kind;

  return (
    <Section title="モデル" note="考えるのに使うモデルと、その使い方。">
      <div class="role-model">
        <Field label="使うモデル">
          <Select
            class="fit"
            onChange={(e) => {
              const model = e.currentTarget.value;
              if (model !== r().model) void assign(model, r().cap);
            }}
          >
            <option value="" selected={!r().model}>
              使わない
            </option>
            <For each={choices()}>
              {(m) => (
                <option value={m.id} selected={m.id === r().model}>
                  {m.name}
                </option>
              )}
            </For>
          </Select>
        </Field>
        <Show when={kind() === ModelKind.AGENT_CLI}>
          <Effort effort={r().effort} fallback={r().defaultEffort} onChange={(effort) => void assign(r().model, r().cap, effort)} />
        </Show>
        <NumberField
          label="一日の上限"
          min={0}
          none="なし"
          value={r().cap}
          unit={
            <>
              {r().capUnit}（今日 {r().yenToday > 0 ? `${yen(r().yenToday)} · ` : ""}
              {String(r().callsToday)} 回）
            </>
          }
          hint={props.cap}
          onSave={(cap) => void assign(r().model, cap)}
        />
      </div>
      {props.children}
    </Section>
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
        <Model setting={s()} models={models()?.models ?? []} cap={props.cap}>
          {props.children}
        </Model>
      )}
    </Show>
  );
}
