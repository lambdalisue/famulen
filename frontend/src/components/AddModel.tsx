import { Dialog } from "@ark-ui/solid/dialog";
import { RadioGroup } from "@ark-ui/solid/radio-group";
import { SegmentGroup } from "@ark-ui/solid/segment-group";
import { createSignal, For, Match, Show, Switch } from "solid-js";
import Icon from "~/components/Icon";
import Modal from "~/components/Modal";
import { AgentCli, type Model, ModelKind } from "~/gen/famulen/v1/settings_pb";
import { act, look } from "~/lib/act";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";

type Preset = { label: string; url: string };

type Way = {
  id: string;
  label: string;
  note: string;
  icon: string;
  kind: ModelKind;
  /** Services whose URL is known; the last one leaves it to the owner. */
  presets?: Preset[];
  modelHint?: string;
};

const OTHER = "ほか";
const BACK = "back";

const WAYS: Way[] = [
  {
    id: "local",
    label: "ローカルのサーバー",
    note: "LM Studio、llama.cpp、別のマシンの Ollama など",
    icon: icons.deviceDesktop,
    kind: ModelKind.LOCAL,
    presets: [
      { label: "LM Studio", url: "http://localhost:1234/v1" },
      { label: "llama.cpp", url: "http://localhost:8080/v1" },
      { label: OTHER, url: "" },
    ],
    modelHint: "qwen3:8b",
  },
  {
    id: "api",
    label: "API",
    note: "OpenAI、OpenRouter など。鍵で呼ぶ",
    icon: icons.cloud,
    kind: ModelKind.API,
    presets: [
      { label: "OpenAI", url: "https://api.openai.com/v1" },
      { label: "OpenRouter", url: "https://openrouter.ai/api/v1" },
      { label: "Gemini", url: "https://generativelanguage.googleapis.com/v1beta/openai" },
      { label: "Groq", url: "https://api.groq.com/openai/v1" },
      { label: OTHER, url: "" },
    ],
    modelHint: "サービスの説明にあるモデル名",
  },
  {
    id: "cli",
    label: "エージェント CLI",
    note: "一覧に出ないモデル名や、別の実行ファイルを使う",
    icon: icons.terminal,
    kind: ModelKind.AGENT_CLI,
  },
  {
    id: "judge",
    label: "判定 API",
    note: "選択肢から選ぶだけの API。トリアージに使う",
    icon: icons.listCheck,
    kind: ModelKind.JUDGE,
  },
  {
    id: "transcribe",
    label: "文字起こし",
    note: "声を文字にする。whisper など",
    icon: icons.microphone,
    kind: ModelKind.TRANSCRIBE,
    presets: [
      { label: "OpenAI", url: "https://api.openai.com/v1" },
      { label: OTHER, url: "" },
    ],
    modelHint: "whisper-1",
  },
];

/** Keeps a password manager from taking a field for a login. */
const PLAIN = {
  autocomplete: "off",
  "data-1p-ignore": true,
  "data-lpignore": "true",
  "data-bwignore": true,
} as const;

function amount(s: string): number {
  const n = Number(s.trim());
  return Number.isFinite(n) && n > 0 ? n : 0;
}

/** Adds a model that Famulen does not find by itself, or one the owner removed. */
export default function AddModel(props: { candidates: Model[] }) {
  const [open, setOpen] = createSignal(false);
  const [way, setWay] = createSignal(WAYS[0]!.id);
  const chosen = () => WAYS.find((w) => w.id === way());
  return (
    <Modal
      open={open()}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) setWay(props.candidates.length > 0 ? BACK : WAYS[0]!.id);
      }}
      title="モデルの追加"
      description="このマシンの Claude Code・Codex・Ollama のモデルは自動で一覧に入る。それ以外をここで足す。"
      size="wide"
      trigger={
        <Dialog.Trigger class="btn primary">
          <Icon svg={icons.plus} size={14} />
          モデルの追加
        </Dialog.Trigger>
      }
    >
      <div class="model-add">
        <RadioGroup.Root class="model-ways" value={way()} onValueChange={(d) => setWay(d.value ?? WAYS[0]!.id)}>
          <RadioGroup.Label class="visually-hidden">足し方</RadioGroup.Label>
          <Show when={props.candidates.length > 0}>
            <WayCard id={BACK} label="外したモデルを戻す" note="このマシンにあり、一覧から外したもの" icon={icons.arrowBackUp} />
          </Show>
          <For each={WAYS}>{(w) => <WayCard {...w} />}</For>
        </RadioGroup.Root>
        <Switch>
          <Match when={way() === BACK}>
            <div>
              <For each={props.candidates} fallback={<p class="sub">外したモデルはない</p>}>
                {(m) => (
                  <div class="model-back">
                    <span>{m.name}</span>
                    <button
                      type="button"
                      class="btn small"
                      aria-label={`${m.name}を戻す`}
                      onClick={() => void act(() => api.models.restoreModel({ id: m.id }), "戻した")}
                    >
                      戻す
                    </button>
                  </div>
                )}
              </For>
            </div>
            <div class="dialog-actions">
              <Dialog.CloseTrigger class="btn">閉じる</Dialog.CloseTrigger>
            </div>
          </Match>
          <Match when={chosen()} keyed>
            {(w) => <Fields way={w} onAdded={() => setOpen(false)} />}
          </Match>
        </Switch>
      </div>
    </Modal>
  );
}

function WayCard(props: { id: string; label: string; note: string; icon: string }) {
  return (
    <RadioGroup.Item value={props.id} class="model-way">
      <RadioGroup.ItemControl class="visually-hidden" />
      <Icon svg={props.icon} size={18} />
      <RadioGroup.ItemText class="model-way-text">
        <b>{props.label}</b>
        <span class="sub">{props.note}</span>
      </RadioGroup.ItemText>
      <RadioGroup.ItemHiddenInput />
    </RadioGroup.Item>
  );
}

function Segments(props: { label: string; items: string[]; value: string; onChange: (value: string) => void }) {
  return (
    <div class="dialog-field">
      <span>{props.label}</span>
      <SegmentGroup.Root
        class="segs"
        aria-label={props.label}
        value={props.value}
        onValueChange={(d) => props.onChange(d.value ?? props.value)}
      >
        <SegmentGroup.Indicator class="segs-indicator" />
        <For each={props.items}>
          {(item) => (
            <SegmentGroup.Item value={item} class="segs-item">
              <SegmentGroup.ItemText>{item}</SegmentGroup.ItemText>
              <SegmentGroup.ItemControl />
              <SegmentGroup.ItemHiddenInput />
            </SegmentGroup.Item>
          )}
        </For>
      </SegmentGroup.Root>
    </div>
  );
}

const CLIS: Record<string, { cli: AgentCli; bin: string; hint: string; note: string }> = {
  "Claude Code": { cli: AgentCli.CLAUDE_CODE, bin: "claude", hint: "claude-opus-5-5", note: "" },
  Codex: { cli: AgentCli.CODEX, bin: "codex", hint: "gpt-6.1-sol", note: "秘書には使えない。" },
};

const METERED = "使ったぶん";
const FLAT = "定額・無料";

/** Asks only for what the chosen way needs. */
function Fields(props: { way: Way; onAdded: () => void }) {
  const w = props.way;
  const first = w.presets?.[0];
  const [preset, setPreset] = createSignal(first?.label ?? OTHER);
  const [url, setUrl] = createSignal(first?.url ?? "");
  const [model, setModel] = createSignal("");
  const [offered, setOffered] = createSignal<string[]>();
  const [asking, setAsking] = createSignal(false);
  const [cli, setCli] = createSignal("Claude Code");
  const [pay, setPay] = createSignal(METERED);
  const [saving, setSaving] = createSignal(false);

  const isCli = w.kind === ModelKind.AGENT_CLI;
  const isLocal = w.kind === ModelKind.LOCAL;
  const keyed = !isCli && !isLocal;
  const perToken = w.kind === ModelKind.API;

  const pickPreset = (label: string) => {
    setPreset(label);
    setUrl(w.presets?.find((p) => p.label === label)?.url ?? "");
    setOffered(undefined);
  };
  const ask = async () => {
    setAsking(true);
    const asked = url();
    const listed = await look(() => api.models.listServerModels({ endpoint: asked }));
    setAsking(false);
    if (listed && asked === url()) setOffered(listed.models);
  };
  const submit = async (e: SubmitEvent & { currentTarget: HTMLFormElement }) => {
    e.preventDefault();
    if (saving()) return;
    const f = new FormData(e.currentTarget);
    const text = (k: string) => String(f.get(k) ?? "").trim();
    const fallback = model().trim() || (preset() === OTHER ? w.label : preset());
    setSaving(true);
    const done = await act(
      () =>
        api.models.registerModel({
          kind: w.kind,
          name: text("name") || fallback,
          model: model().trim(),
          endpoint: isCli ? text("bin") : url().trim(),
          cli: isCli ? CLIS[cli()]!.cli : AgentCli.UNSPECIFIED,
          apiKey: text("api_key"),
          inputYen: amount(text("input_yen")),
          outputYen: amount(text("output_yen")),
          callYen: amount(text("call_yen")),
          monthlyYen: amount(text("monthly_yen")),
        }),
      "足した",
    );
    setSaving(false);
    if (done) props.onAdded();
  };

  const Price = (p: { id: string; label: string }) => (
    <div class="dialog-field">
      <label for={`m-${p.id}`}>{p.label}</label>
      <input id={`m-${p.id}`} class="input" name={p.id} inputmode="decimal" placeholder="円" {...PLAIN} />
    </div>
  );

  return (
    <form class="model-fields" autocomplete="off" onSubmit={submit}>
      <Show when={isCli}>
        <Segments label="道具" items={Object.keys(CLIS)} value={cli()} onChange={setCli} />
      </Show>
      <Show when={w.presets}>
        {(presets) => (
          <Segments label="サービス" items={presets().map((p) => p.label)} value={preset()} onChange={pickPreset} />
        )}
      </Show>
      <Show when={!isCli}>
        <div class="dialog-field">
          <label for="m-url">URL</label>
          <div class="model-url">
            <input
              id="m-url"
              class="input mono"
              type="url"
              required
              placeholder="https://…/v1"
              value={url()}
              onInput={(e) => {
                setUrl(e.currentTarget.value);
                setOffered(undefined);
              }}
              {...PLAIN}
            />
            <Show when={isLocal}>
              <button type="button" class="btn" disabled={asking() || url().trim() === ""} onClick={() => void ask()}>
                {asking() ? "探している" : "モデルを探す"}
              </button>
            </Show>
          </div>
        </div>
      </Show>
      <Show when={keyed}>
        <div class="dialog-field">
          <label for="m-key">API キー</label>
          <input id="m-key" class="input" name="api_key" type="password" {...PLAIN} autocomplete="new-password" />
          <span class="sub optional">金庫にしまい、ゲートウェイが呼ぶときだけ付ける。</span>
        </div>
      </Show>
      <div class="dialog-field">
        <label for="m-model">モデル名</label>
        <input
          id="m-model"
          class="input mono"
          required={w.kind !== ModelKind.JUDGE}
          placeholder={isCli ? CLIS[cli()]!.hint : w.modelHint}
          value={model()}
          onInput={(e) => setModel(e.currentTarget.value)}
          {...PLAIN}
        />
        <Show when={isCli && CLIS[cli()]!.note}>{(note) => <span class="sub optional">{note()}</span>}</Show>
        <Show when={offered()}>
          {(found) => (
            <div class="model-picks" role="group" aria-label="サーバーにあるモデル">
              <For each={found()} fallback={<span class="sub optional">このサーバーにはモデルがない</span>}>
                {(name) => (
                  <button
                    type="button"
                    class="model-pick"
                    aria-pressed={model() === name}
                    onClick={() => setModel(name)}
                  >
                    {name}
                  </button>
                )}
              </For>
            </div>
          )}
        </Show>
      </div>
      <Show when={keyed}>
        <Segments label="料金" items={[METERED, FLAT]} value={pay()} onChange={setPay} />
        <div class="model-prices">
          <Show when={pay() === METERED} fallback={<Price id="monthly_yen" label="月額" />}>
            <Show when={perToken} fallback={<Price id="call_yen" label="1 回あたり" />}>
              <Price id="input_yen" label="入力 100 万トークンあたり" />
              <Price id="output_yen" label="出力 100 万トークンあたり" />
            </Show>
          </Show>
        </div>
      </Show>
      <Show when={isCli}>
        <div class="dialog-field">
          <label for="m-bin">
            実行ファイル <span class="optional">（なくてもよい）</span>
          </label>
          <input id="m-bin" class="input mono" name="bin" placeholder={CLIS[cli()]!.bin} {...PLAIN} />
        </div>
      </Show>
      <div class="dialog-field">
        <label for="m-name">
          名前 <span class="optional">（なくてもよい）</span>
        </label>
        <input id="m-name" class="input" name="name" placeholder={model() || "モデル名になる"} {...PLAIN} />
      </div>
      <div class="dialog-actions">
        <Dialog.CloseTrigger class="btn">やめる</Dialog.CloseTrigger>
        <button class="btn primary" disabled={saving()}>
          追加
        </button>
      </div>
    </form>
  );
}
