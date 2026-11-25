import { createAsync, revalidate } from "@solidjs/router";
import { createSignal, For, type JSX, Match, Show, Switch } from "solid-js";
import type { SlackApp } from "~/gen/famulen/v1/settings_pb";
import { message } from "~/lib/act";
import { api } from "~/lib/api";
import { slackAppsQuery } from "~/lib/data";
import { externalLink } from "~/lib/icons";
import { iconFile, PORTRAITS, type Portrait } from "~/lib/portrait";
import { Button, Callout, Dialog, DialogActions, DialogTrigger, Field, Icon, Keyed, LinkButton, Tag, TextField } from "~/components/ui";

const SLACK_APPS = "https://api.slack.com/apps";
const STEPS = ["Slack を開く", "トークンを作る", "貼る", "できた"] as const;
const PASTE = 2;
const DONE = 3;

/** Whether the icons of the connected Slack apps follow the portrait, and the way to let them. */
export default function SlackIconSteps(props: { portrait: Portrait }) {
  const apps = createAsync(() => slackAppsQuery());
  return (
    <Show when={(apps() ?? []).length > 0}>
      <div class="pref-sub">
        <h3>Slack の一覧のアイコン</h3>
        <p class="sub">サイドバーや DM の一覧に出る、アプリのアイコン。一度つなげば、性別を選ぶたびに替わる。</p>
        <ul class="slack-names" aria-label="Slack のアプリ">
          <Keyed each={apps() ?? []} by={(a) => a.id}>
            {(app) => (
              <li>
                <span class="ws">{app().workspaces.join("・")}</span>
                <Tag tone={app().follows ? "ok" : undefined}>{app().follows ? "つないである" : "つないでいない"}</Tag>
                <Steps app={app()} portrait={props.portrait} />
              </li>
            )}
          </Keyed>
        </ul>
      </div>
    </Show>
  );
}

/** One step at a time, from opening Slack to the icon having changed. */
function Steps(props: { app: SlackApp; portrait: Portrait }) {
  const [open, setOpen] = createSignal(false);
  const [at, setAt] = createSignal(0);
  const [token, setToken] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [trouble, setTrouble] = createSignal("");
  const paste = async (e: SubmitEvent) => {
    e.preventDefault();
    if (busy()) return;
    setBusy(true);
    setTrouble("");
    // Said in the dialog, beside the field to mend, rather than by `act`.
    let said: string;
    try {
      const answer = await api.settings.letSlackAppFollow({ app: props.app.id, refreshToken: token() });
      // What Slack took is spent: pasted again, it would replace the next one.
      setToken("");
      await revalidate(slackAppsQuery.key);
      said = answer.troubles.join("。");
    } catch (e) {
      said = message(e);
    }
    setBusy(false);
    if (said) return setTrouble(said);
    setAt(DONE);
  };
  return (
    <Dialog
      open={open()}
      onOpenChange={(next) => {
        setOpen(next);
        setAt(0);
        setToken("");
        setTrouble("");
      }}
      title="一覧のアイコンを性別に合わせる"
      trigger={<DialogTrigger small>{props.app.follows ? "つなぎ直す" : "性別に合わせる"}</DialogTrigger>}
    >
      <ol class="steps" aria-label="手順">
        <For each={STEPS}>
          {(name, i) => (
            <li classList={{ done: i() < at(), now: i() === at() }} aria-current={i() === at() ? "step" : undefined}>
              <span class="step-no">{i() + 1}</span>
              {name}
            </li>
          )}
        </For>
      </ol>
      <Switch>
        <Match when={at() === 0}>
          <Step next={() => setAt(1)}>
            <p>下のボタンで、Slack のアプリの一覧を開く。新しいタブで開くので、この画面はそのままにしておく。</p>
            <LinkButton variant="primary" href={SLACK_APPS} onClick={() => setAt(1)}>
              Slack を開く
              <Icon svg={externalLink} size={14} />
            </LinkButton>
            <p class="sub">サインインを求められたら、アプリを作ったアカウントで入る。</p>
          </Step>
        </Match>
        <Match when={at() === 1}>
          <Step back={() => setAt(0)} next={() => setAt(PASTE)}>
            <p>開いた頁で、次の順に押す。</p>
            <ol class="step-list">
              <li>
                アプリの一覧の下にある「Your App Configuration Tokens」の「Generate Token」を押す。
              </li>
              <li>
                ワークスペースを選ぶ。一覧で App ID が <span class="mono">{props.app.id}</span> の行の「Workspace」と同じもの。
              </li>
              <li>「Generate」を押す。</li>
            </ol>
            <p class="sub">アプリの中の頁にはない。見つからなければ、Slack の右上の「Your Apps」を押して一覧に戻る。</p>
          </Step>
        </Match>
        <Match when={at() === PASTE}>
          <form class="dialog-form" onSubmit={paste}>
            <p>
              できた 2 つのうち、<b>Refresh Token</b>（<span class="mono">xoxe-</span> で始まる）の「Copy」を押して、ここに貼る。
            </p>
            <Field label="Refresh Token">
              <TextField
                type="password"
                class="mono"
                autocomplete="off"
                placeholder="xoxe-…"
                required
                value={token()}
                onInput={(e) => setToken(e.currentTarget.value)}
              />
            </Field>
            <Show when={trouble()}>
              {(text) => (
                <Callout tone="warn" live>
                  {text()}
                </Callout>
              )}
            </Show>
            <p class="sub">
              金庫に入り、使うたびに新しいものに替わる。このトークンは Slack のほかのアプリの設定も変えられるので、ほかには貼らない。Famulen が使うのはアイコンを替えるときだけ。
            </p>
            <DialogActions
              before={<Button onClick={() => setAt(1)}>戻る</Button>}
              submit={busy() ? "替えている…" : "アイコンを替える"}
              disabled={busy() || !token().trim()}
            />
          </form>
        </Match>
        <Match when={at() === DONE}>
          <div class="dialog-form">
            <div class="row">
              <img src={iconFile(props.portrait, "avatar.svg")} alt="" width="56" height="56" />
              <p>
                Slack のアイコンを「{PORTRAITS.find((p) => p.id === props.portrait)?.label}」に替えた。これからは、性別を選ぶと一緒に替わる。
              </p>
            </div>
            <p class="sub">Slack の画面に出るまで、少しかかることがある。</p>
            <div class="dialog-actions">
              <span />
              <Button variant="primary" onClick={() => setOpen(false)}>
                閉じる
              </Button>
            </div>
          </div>
        </Match>
      </Switch>
    </Dialog>
  );
}

function Step(props: { back?: () => void; next: () => void; children: JSX.Element }) {
  return (
    <div class="dialog-form">
      {props.children}
      <div class="dialog-actions">
        <Show when={props.back} fallback={<span />}>
          {(back) => <Button onClick={() => back()()}>戻る</Button>}
        </Show>
        <Button variant={props.back ? "primary" : undefined} onClick={() => props.next()}>
          次へ
        </Button>
      </div>
    </div>
  );
}
