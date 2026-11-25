import { useLocation } from "@solidjs/router";
import { For, type JSX, Show } from "solid-js";
import * as icons from "~/lib/icons";
import { Icon } from "~/components/ui";

/** Every settings page, by group: path, label and icon. */
export const NAV: [string, [string, string, string][]][] = [
  [
    "",
    [
      ["/settings/workgroups", "ワークグループ", icons.layoutGrid],
      ["/settings/look", "表示", icons.palette],
    ],
  ],
  [
    "エージェント",
    [
      ["/settings", "秘書", icons.idBadge2],
      ["/settings/roles/dream", "夢", icons.moonStars],
      ["/settings/lookout", "見張り", icons.eye],
      ["/settings/roles/triage", "トリアージ", icons.arrowsSplit],
      ["/settings/roles/assess", "見極め", icons.shieldCheck],
      ["/settings/roles/worker", "手代", icons.robot],
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
      ["/settings/notices", "通知", icons.bell],
      ["/settings/channels", "チャネル", icons.devices],
      ["/settings/services", "サービス", icons.plugConnected],
      ["/settings/vault", "金庫", icons.vault],
    ],
  ],
];

/** The settings pages share one frame: the list of pages beside the page. */
export default function Settings(props: { children?: JSX.Element }) {
  const location = useLocation();
  const on = (href: string) => (href === "/settings" ? location.pathname === "/settings" : location.pathname.startsWith(href));
  return (
    <div class="framed">
      <nav class="side settings-nav" aria-label="設定">
        <For each={NAV}>
          {([group, items]) => (
            <>
              <Show when={group}>
                <h3>{group}</h3>
              </Show>
              <For each={items}>
                {([href, label, icon]) => (
                  <a class="nav-item" classList={{ on: on(href) }} href={href} aria-current={on(href) ? "page" : undefined}>
                    <Icon svg={icon} size={16} />
                    <span>{label}</span>
                  </a>
                )}
              </For>
            </>
          )}
        </For>
      </nav>
      <div class="detail">
        <div class="page settings-page">{props.children}</div>
      </div>
    </div>
  );
}
