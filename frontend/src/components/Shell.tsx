import { Dialog } from "@ark-ui/solid/dialog";
import { Title } from "@solidjs/meta";
import { A, createAsync, revalidate, useLocation } from "@solidjs/router";
import Icon from "~/components/Icon";
import * as icons from "~/lib/icons";
import { createEffect, createSignal, For, type JSX, on, onCleanup, onMount, Show, Suspense } from "solid-js";
import { chooseWorkgroup, workgroup, tint } from "~/lib/workgroup";
import { useWaiting } from "~/lib/inbox";
import { statusQuery, workgroupsQuery } from "~/lib/data";
import { followEvents } from "~/lib/events";
import { ASIDE, BOTTOM, SECTIONS, tabOf, wayBack } from "~/lib/tabs";
import { pageTitle } from "~/lib/view";
import Chat from "./Chat";
import Toast from "./Toast";
import Mark from "./Mark";
import SideSheet from "./SideSheet";
import { useOwner } from "~/lib/owner";
import StatusPanel from "./StatusPanel";

/** How the secretary is, in a word; pressing it opens the whole state. */
function Alive() {
  const status = createAsync(() => statusQuery());
  const waiting = useWaiting();
  const busy = () => status()?.state === "running";
  return (
    <Dialog.Root lazyMount unmountOnExit>
      <Dialog.Trigger class="alive" aria-label="秘書の状態">
        <span class="dot" style={{ background: busy() ? "#2a78d6" : "#1A8C7F" }} />
        <span role="status">{busy() ? "考えている" : "待っている"}</span>
        <Show when={waiting.count() > 0}>
          <span class="alive-waiting" title="あなたを待っているもの">
            {waiting.count()}
          </span>
        </Show>
        <Icon svg={icons.chevronDown} size={12} />
      </Dialog.Trigger>
      <SideSheet name="status" title="秘書の状態">
        <StatusPanel />
      </SideSheet>
    </Dialog.Root>
  );
}

/** The console around one page: the bars, the workgroups and the chat. */
export default function Shell(props: { children?: JSX.Element }) {
  const location = useLocation();
  const workgroups = createAsync(() => workgroupsQuery());
  const owner = useOwner();
  const [sheet, setSheet] = createSignal<"" | "half" | "full">("");
  let workgroupSheet: HTMLDialogElement | undefined;

  const tab = () => tabOf(location.pathname);
  const current = () => workgroups()?.find((c) => c.id === workgroup()) ?? workgroups()?.[0];
  const color = () => current()?.color ?? "#0F6E68";
  const aide = owner.aide;
  const more = () => !BOTTOM.includes(tab().key);
  const mobileTitle = () => pageTitle() || tab().label;

  onMount(() => {
    followEvents(workgroup);
    // The narrow layout's rules read these from <body>: while typing, the
    // bottom bar gives way to the keyboard.
    const narrow = () => window.matchMedia("(max-width: 700px)").matches;
    const focusIn = (e: FocusEvent) => {
      if (narrow() && e.target instanceof HTMLElement && e.target.id === "say") document.body.classList.add("typing");
    };
    const focusOut = (e: FocusEvent) => {
      if (e.target instanceof HTMLElement && e.target.id === "say") document.body.classList.remove("typing");
    };
    document.addEventListener("focusin", focusIn);
    document.addEventListener("focusout", focusOut);
    onCleanup(() => {
      document.removeEventListener("focusin", focusIn);
      document.removeEventListener("focusout", focusOut);
    });
  });
  createEffect(() => {
    document.body.classList.toggle("sheet-half", sheet() === "half");
    document.body.classList.toggle("sheet-full", sheet() === "full");
  });
  createEffect(on(() => location.pathname, () => setSheet(""), { defer: true }));

  const choose = async (id: string) => {
    chooseWorkgroup(id);
    workgroupSheet?.close();
    await revalidate();
  };

  return (
    <div
      class="app"
      style={{ "--workgroup": color(), "--wg-tint": tint(color()) }}
    >
      <Title>
        {tab().label} · {aide()}
      </Title>
      <header class="topbar">
        <span class="logo">
          <Mark size={24} />
        </span>
        <span class="brand">
          <b>{aide()}</b>
          <small>Famulen</small>
        </span>
        <div class="wg-tabs" role="tablist" aria-label="ワークグループ">
          <For each={workgroups()}>
            {(c) => (
              <button
                type="button"
                role="tab"
                aria-selected={c.id === current()?.id}
                class={`wg-btn${c.id === current()?.id ? " on" : ""}`}
                onClick={() => choose(c.id)}
              >
                <span class="dot" style={{ background: c.color }} />
                {c.name}
              </button>
            )}
          </For>
          <A class="wg-btn" href="/settings/workgroups" aria-label="ワークグループを足す">
            <Icon svg={icons.plus} size={14} />
          </A>
        </div>
        <span class="wg-hint">
          {current()?.id === "all" || !current()
            ? "すべてのワークグループを使う"
            : `${current()?.name}のものだけを使い、ほかの予定は薄く出す`}
        </span>
        <Alive />
      </header>
      <header id="m-head" class="m-head">
        <Show when={wayBack(location.pathname, location.search)}>
          {(back) => (
            <A class="m-back" href={back().href} aria-label={`${back().label}へ戻る`}>
              <Icon svg={icons.chevronLeft} size={26} />
            </A>
          )}
        </Show>
        <h1 class="m-title">{mobileTitle()}</h1>
        <button type="button" class="m-wg" aria-label={`ワークグループを選ぶ（いまは${current()?.name ?? ""}）`} onClick={() => workgroupSheet?.showModal()}>
          <span class="dot" style={{ background: color() }} />
          {current()?.name}
          <Icon svg={icons.chevronDown} size={16} />
        </button>
      </header>
      <div class="body">
        <Chat sheet={setSheet} />
        <div id="main" class="main">
          <nav class="tabs" aria-label="セクション">
            <For each={SECTIONS}>
              {(t) => (
                <A class={`tab${t.key === tab().key ? " on" : ""}`} href={t.path} end={t.path === "/"}>
                  <Icon svg={t.icon} size={16} />
                  {t.label}
                </A>
              )}
            </For>
            <span class="gap" />
            <For each={ASIDE}>
              {(t) => (
                <A class={`tab${t.key === tab().key ? " on" : ""}`} href={t.path}>
                  <Icon svg={t.icon} size={16} />
                  {t.label}
                </A>
              )}
            </For>
          </nav>
          <div id="pane" class="pane" data-tab={tab().key}>
            <Suspense fallback={<p class="empty">読み込んでいる</p>}>{props.children}</Suspense>
          </div>
        </div>
      </div>
      <nav id="m-tabbar" class="m-tabbar" aria-label="セクション">
        <For each={SECTIONS.filter((t) => BOTTOM.includes(t.key))}>
          {(t) => (
            <A class={`m-tab${t.key === tab().key ? " on" : ""}`} href={t.path} end={t.path === "/"}>
              <Icon svg={t.icon} size={24} />
              <span>{t.label}</span>
            </A>
          )}
        </For>
        <A class={`m-tab${more() ? " on" : ""}`} href="/more">
          <Icon svg={icons.layoutGrid} size={24} />
          <span>ほか</span>
        </A>
      </nav>
      <dialog
        ref={workgroupSheet}
        class="sheet"
        aria-label="ワークグループ"
        onClick={(e) => {
          if (e.target === workgroupSheet) workgroupSheet?.close();
        }}
      >
        <div class="sheet-grip" />
        <h2>ワークグループ</h2>
        <p class="sub">会話と画面の両方が絞られる</p>
        <div class="list">
          <For each={workgroups()}>
            {(c) => (
              <div class="item">
                <button type="button" class="sheet-row" aria-pressed={c.id === current()?.id} onClick={() => choose(c.id)}>
                  <span class="dot" style={{ background: c.color, width: "12px", height: "12px" }} />
                  <b class="grow">{c.name}</b>
                  <Show when={c.id === current()?.id}>
                    <Icon svg={icons.check} size={18} />
                  </Show>
                </button>
              </div>
            )}
          </For>
        </div>
        <A class="btn" href="/settings/workgroups" onClick={() => workgroupSheet?.close()}>
          <Icon svg={icons.plus} size={16} />
          ワークグループを足す
        </A>
      </dialog>
      <Toast />
    </div>
  );
}
