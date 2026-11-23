import { createSignal, For, Match, Show, Switch } from "solid-js";
import { AgentCli, type Model, ModelKind } from "~/gen/famulen/v1/settings_pb";
import { act, look } from "~/lib/act";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";
import { Button, Dialog, DialogActions, DialogTrigger, Field, RadioCards, Segmented, TextField } from "~/components/ui";

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

function amount(s: string): number {
  const n = Number(s.trim());
  return Number.isFinite(n) && n > 0 ? n : 0;
}

const RESTORE = { id: BACK, label: "外したモデルを戻す", note: "このマシンにあり、一覧から外したもの", icon: icons.arrowBackUp };

/** Adds a model that Famulen does not find by itself, or one the owner removed. */
export default function AddModel(props: { candidates: Model[] }) {
  const [open, setOpen] = createSignal(false);
  const [way, setWay] = createSignal(WAYS[0]!.id);
  const chosen = () => WAYS.find((w) => w.id === way());
  return (
    <Dialog
      open={open()}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) setWay(props.candidates.length > 0 ? BACK : WAYS[0]!.id);
      }}
      title="モデルの追加"
      description="このマシンの Claude Code・Codex・Ollama のモデルは自動で一覧に入る。それ以外をここで足す。"
      size="wide"
      trigger={
        <DialogTrigger variant="primary" icon={icons.plus}>
モデルの追加
        </DialogTrigger>
      }
    >
      <div class="model-add">
        <RadioCards
          label="足し方"
          items={[...(props.candidates.length > 0 ? [RESTORE] : []), ...WAYS]}
          value={way()}
          onChoose={(id) => setWay(id || WAYS[0]!.id)}
        />
        <Switch>
          <Match when={way() === BACK}>
            <div>
              <For each={props.candidates} fallback={<p class="sub">外したモデルはない</p>}>
                {(m) => (
                  <div class="model-back">
                    <span>{m.name}</span>
                    <Button small aria-label={`${m.name}を戻す`} onClick={() => void act(() => api.models.restoreModel({ id: m.id }), "戻した")}>
                      戻す
                    </Button>
                  </div>
                )}
              </For>
            </div>
            <DialogActions cancel="閉じる" />
          </Match>
          <Match when={chosen()} keyed>
            {(w) => <Fields way={w} onAdded={() => setOpen(false)} />}
          </Match>
        </Switch>
      </div>
    </Dialog>
  );
}

function Segments(props: { label: string; items: string[]; value: string; onChange: (value: string) => void }) {
  return (
    <Field label={props.label} group>
      <Segmented label={props.label} items={props.items.map((i) => [i, i])} value={props.value} onChoose={props.onChange} />
    </Field>
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
    <Field label={p.label}><TextField id={`m-${p.id}`} name={p.id} inputmode="decimal" placeholder="円" /></Field>
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
        <Field label="URL">
          <div class="model-url">
            <TextField class="mono" id="m-url" type="url" required placeholder="https://…/v1" value={url()} onInput={(e) => {
                setUrl(e.currentTarget.value);
                setOffered(undefined);
              }} />
            <Show when={isLocal}>
              <Button disabled={asking() || url().trim() === ""} onClick={() => void ask()}>
                {asking() ? "探している" : "モデルを探す"}
              </Button>
            </Show>
          </div>
        </Field>
      </Show>
      <Show when={keyed}>
        <Field label="API キー" hint="金庫にしまい、ゲートウェイが呼ぶときだけ付ける。">
          <TextField id="m-key" name="api_key" type="password" autocomplete="new-password" />
          </Field>
      </Show>
      <div class="field"><label class="field-label" for="m-model">モデル名</label>
        <TextField class="mono" id="m-model" required={w.kind !== ModelKind.JUDGE} placeholder={isCli ? CLIS[cli()]!.hint : w.modelHint} value={model()} onInput={(e) => setModel(e.currentTarget.value)} />
        <Show when={isCli && CLIS[cli()]!.note}>{(note) => <span class="sub">{note()}</span>}</Show>
        <Show when={offered()}>
          {(found) => (
            <div class="model-picks" role="group" aria-label="サーバーにあるモデル">
              <For each={found()} fallback={<span class="sub">このサーバーにはモデルがない</span>}>
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
        <Field label="実行ファイル（なくてもよい）">
          <TextField class="mono" id="m-bin" name="bin" placeholder={CLIS[cli()]!.bin} />
        </Field>
      </Show>
      <Field label="名前（なくてもよい）">
        <TextField id="m-name" name="name" placeholder={model() || "モデル名になる"} />
      </Field>
      <DialogActions submit="追加" disabled={saving()} />
    </form>
  );
}
