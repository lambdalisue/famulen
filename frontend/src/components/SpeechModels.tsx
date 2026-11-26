import { createAsync, query } from "@solidjs/router";
import { createEffect, createMemo, For, Match, onCleanup, Show, Switch } from "solid-js";
import { type SpeechModel, SpeechModelState, type SpeechModels as Held, SpeechServerState, SpeechStep } from "~/gen/famulen/v1/settings_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";
import { modelsQuery } from "~/lib/models";
import { reread } from "~/lib/remember";
import { Button, Callout, List, ListRow, Meter, Section, Tag, whenConfirmed } from "~/components/ui";

const speechQuery = query(() => api.models.getSpeechModels({}), "speech-models");

/** How often a download or a server not yet ready is looked at. */
const LOOK_MS = 1000;

/** A size as those who hand the files out write it: 488 MB, 1.6 GB. */
function bytesText(bytes: bigint): string {
  const n = Number(bytes);
  return n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : `${Math.max(1, Math.round(n / 1e6))} MB`;
}

const STEPS: Record<SpeechStep, string> = {
  [SpeechStep.UNSPECIFIED]: "ダウンロードを始めている",
  [SpeechStep.ENGINE]: "whisper.cpp 本体をダウンロードしている",
  [SpeechStep.BUILDING]: "whisper.cpp を組み立てている（数分かかる）",
  [SpeechStep.MODEL]: "モデルをダウンロードしている",
};

function Downloading(props: { model: SpeechModel }) {
  const mb = (n: bigint) => Math.round(Number(n) / 1e6);
  return (
    <div class="item-more" role="status">
      <Show when={mb(props.model.total) > 0} fallback={<span class="sub">{STEPS[props.model.step]}</span>}>
        <Meter label={STEPS[props.model.step]} value={mb(props.model.done)} max={mb(props.model.total)} unit="MB" />
      </Show>
    </div>
  );
}

/** What stands at the end of a downloaded model that is used. */
function Used(props: { server: SpeechServerState }) {
  return (
    <Switch fallback={<Tag tone="info">起動中</Tag>}>
      <Match when={props.server === SpeechServerState.READY}>
        <Tag tone="ok">使っている</Tag>
      </Match>
      <Match when={props.server === SpeechServerState.FAILED}>
        <Tag tone="danger">起動できない</Tag>
      </Match>
    </Switch>
  );
}

function Row(props: { model: SpeechModel; held: Held }) {
  const m = () => props.model;
  const by = () => ({ id: m().id });
  const remove = () =>
    whenConfirmed(
      `${m().id} を削除する`,
      "削除する",
      () => act(() => api.models.deleteSpeechModel(by()), "削除した"),
      m().used ? "声の聞き取りに使っている。削除すると、使うモデルを選ぶまで Even G2 で話せない。" : "また使うには、ダウンロードし直す。",
    );
  return (
    <ListRow
      class="wraps"
      title={m().id}
      sub={`${bytesText(m().bytes)} · 速さ: ${m().speed} · 精度: ${m().accuracy}`}
      end={
        <>
          <Show when={m().state === SpeechModelState.ABSENT || m().state === SpeechModelState.FAILED}>
            <Button small icon={icons.download} disabled={!props.held.supported} onClick={() => act(() => api.models.downloadSpeechModel(by()), "")}>
              ダウンロード
            </Button>
          </Show>
          <Show when={m().state === SpeechModelState.DOWNLOADING}>
            <Button small icon={icons.x} onClick={() => act(() => api.models.deleteSpeechModel(by()), "中止した")}>
              中止
            </Button>
          </Show>
          <Show when={m().state === SpeechModelState.DOWNLOADED}>
            <Show
              when={m().used}
              fallback={
                <Button small onClick={() => act(() => api.models.useSpeechModel(by()))}>
                  使う
                </Button>
              }
            >
              <Used server={props.held.server} />
            </Show>
            <Button small variant="ghost" icon={icons.trash} onClick={remove}>
              削除
            </Button>
          </Show>
        </>
      }
    >
      <Show when={m().state === SpeechModelState.DOWNLOADING}>
        <Downloading model={m()} />
      </Show>
      <Show when={m().state === SpeechModelState.FAILED}>
        <div class="item-more">
          <Callout tone="danger" live>
            ダウンロードできなかった：{m().failure}
          </Callout>
        </div>
      </Show>
      <Show when={m().used && props.held.server === SpeechServerState.FAILED}>
        <div class="item-more">
          <Callout tone="danger" live>
            サーバーを起動できなかった：{props.held.serverFailure}
          </Callout>
        </div>
      </Show>
    </ListRow>
  );
}

/** The models that can be brought in to hear on this machine. */
export default function SpeechModels() {
  const speech = createAsync(() => speechQuery());
  const busy = createMemo(() => {
    const now = speech();
    return (
      !!now &&
      now.models.some(
        (m) =>
          m.state === SpeechModelState.DOWNLOADING ||
          (m.used && m.state === SpeechModelState.DOWNLOADED && now.server !== SpeechServerState.READY),
      )
    );
  });
  createEffect(() => {
    if (!busy()) return;
    // A download that ends registers its model, so the choices are read too.
    const timer = setInterval(() => void reread([speechQuery.key, modelsQuery.key]), LOOK_MS);
    onCleanup(() => clearInterval(timer));
  });

  return (
    <Section
      title="このマシンで聞き取る"
      label="このマシンで聞き取る"
      note="whisper.cpp のモデルをこのマシンに置いて聞き取る。声は外に出ない。ダウンロードするのは押したときだけで、削除すると容量が戻る。サーバーは Famulen が起動する。"
    >
      <Show when={speech()}>
        {(now) => (
          <>
            <Show when={!now().supported}>
              <Callout tone="warn">この OS では用意できない。</Callout>
            </Show>
            <Show when={now().engineBytes > 0n}>
              <p class="sub">
                はじめのダウンロードでは、whisper.cpp 本体（{bytesText(now().engineBytes)}）も一緒にダウンロードする。
                <Show when={now().builds}>このマシンではソースから組み立てる。Xcode Command Line Tools が要り、数分かかる。</Show>
              </p>
            </Show>
            <List label="このマシンに置けるモデル">
              <For each={now().models.map((m) => m.id)}>
                {(id) => <Row model={now().models.find((m) => m.id === id)!} held={now()} />}
              </For>
            </List>
          </>
        )}
      </Show>
    </Section>
  );
}
