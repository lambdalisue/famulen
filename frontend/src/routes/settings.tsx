import { A, useLocation } from "@solidjs/router";
import { For, type JSX, Show } from "solid-js";
import Icon from "~/components/Icon";
import * as icons from "~/lib/icons";

/** Every settings page, by group: path, label and icon. */
export const NAV: [string, [string, string, string][]][] = [
  ["", [["/settings/workgroups", "ワークグループ", icons.layoutGrid]]],
  [
    "エージェント",
    [
      ["/settings", "秘書", icons.idBadge2],
      ["/settings/roles/dream", "夢", icons.moonStars],
      ["/settings/lookout", "見張り", icons.eye],
      ["/settings/roles/triage", "トリアージ", icons.arrowsSplit],
      ["/settings/roles/assess", "見極め", icons.shieldCheck],
    ],
  ],
  [
    "モデル",
    [
      ["/settings/models", "登録したモデル", icons.brain],
      ["/settings/usage", "使った量", icons.chartBar],
    ],
  ],
  [
    "つながり",
    [
      ["/settings/channels", "チャネル", icons.devices],
      ["/settings/services", "サービス", icons.plugConnected],
      ["/settings/vault", "金庫", icons.vault],
    ],
  ],
];

/** The settings pages share one frame: the list of pages beside the page. */
export default function Settings(props: { children?: JSX.Element }) {
  const location = useLocation();
  const on = (href: string) =>
    href === "/settings" ? location.pathname === "/settings" : location.pathname.startsWith(href);
  return (
    <div class="pane split" style={{ margin: "-22px -26px", flex: 1 }}>
      <nav class="side settings-nav" aria-label="設定">
        <For each={NAV}>
          {([group, items]) => (
            <>
              <Show when={group}>
                <h3 style={{ padding: "8px 8px 2px" }}>{group}</h3>
              </Show>
              <For each={items}>
                {([href, label, icon]) => (
                  <A class={`nav-item${on(href) ? " on" : ""}`} href={href} end>
                    <Icon svg={icon} size={16} />
                    <span>{label}</span>
                  </A>
                )}
              </For>
            </>
          )}
        </For>
      </nav>
      <div class="detail">
        <div class="settings-page">{props.children}</div>
      </div>
    </div>
  );
}
