import { createMemo, createSignal, Show } from "solid-js";
import { encode } from "uqr";
import { Callout, Dialog, DialogTrigger } from "~/components/ui";
import { reachesOnlyThisMachine } from "~/lib/phone";

const DEVELOPER_LOGIN = "hub.evenrealities.com/login";

/** How the owner opens the page for the glasses on the phone, from the console opened at `origin`. */
export default function EvenG2Open(props: { origin: string }) {
  const [open, setOpen] = createSignal(false);
  return (
    <Dialog open={open()} onOpenChange={setOpen} title="Even G2 で開く" trigger={<DialogTrigger small>QR コード</DialogTrigger>}>
      <Show
        when={!reachesOnlyThisMachine(new URL(props.origin).hostname)}
        fallback={
          <Callout tone="warn">
            この番地（<span class="mono">{props.origin}</span>）はスマートフォンからは届きません。スマートフォンから開ける番地（Tailscale Serve の https の URL など）でこのコンソールを開き直すと、ここに QR コードが出ます。その番地は、URL なら <span class="mono">FAMULEN_CONSOLE_URL</span> に、ホスト名だけなら <span class="mono">FAMULEN_CONSOLE_HOSTS</span> に入れておきます。
          </Callout>
        }
      >
        <QrCode text={`${props.origin}/g2`} />
        <p class="sub qr-address mono">{`${props.origin}/g2`}</p>
      </Show>
      <p class="sub">スマートフォンでの読み方。開発者モードにするのは、はじめの一度だけ。</p>
      <ol class="step-list">
        <li>
          <a href={`https://${DEVELOPER_LOGIN}`} target="_blank" rel="noreferrer">
            {DEVELOPER_LOGIN}
          </a>{" "}
          に、Even App と同じアカウントでログインする。
        </li>
        <li>Even App を終了して開き直すと、開発者モードになる。</li>
        <li>Even Hub タブの右上の、開発者向けの項目の「Scan QR」で、QR コードを読む。</li>
      </ol>
    </Dialog>
  );
}

/**
 * Drawn here, so that no service outside is told the address. Black on white
 * with a margin whatever the theme, as cameras read it.
 */
function QrCode(props: { text: string }) {
  // The content `evenhub qr` puts in its code is the URL and nothing else.
  const code = createMemo(() => encode(props.text, { ecc: "M", border: 4 }));
  const dark = () =>
    code()
      .data.flatMap((row, y) => row.map((on, x) => (on ? `M${x} ${y}h1v1h-1z` : "")))
      .join("");
  return (
    <svg class="qr" role="img" aria-label={`${props.text} の QR コード`} viewBox={`0 0 ${code().size} ${code().size}`} shape-rendering="crispEdges">
      <rect width="100%" height="100%" fill="#fff" />
      <path d={dark()} fill="#000" />
    </svg>
  );
}
