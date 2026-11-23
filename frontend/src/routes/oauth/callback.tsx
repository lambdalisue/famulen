import { A, useLocation } from "@solidjs/router";
import { createSignal, onMount, Show } from "solid-js";
import type { LoginFinished } from "~/gen/famulen/v1/services_pb";
import { api } from "~/lib/api";
import { setView } from "~/lib/view";

/** Where Google, Slack and other services send the owner back after a login. */
export default function Callback() {
  setView("ログインの戻り");
  const location = useLocation();
  const [done, setDone] = createSignal<{ ok?: LoginFinished; error?: string }>();
  // Only once the owner's browser runs the page: whatever merely reads the
  // address must not finish the login.
  onMount(async () => {
    try {
      setDone({ ok: await api.connections.finishLogin({ query: location.search.replace(/^\?/, "") }) });
    } catch (e) {
      setDone({ error: e instanceof Error ? e.message : String(e) });
    }
  });
  return (
    <div class="card" style={{ "max-width": "560px" }}>
      <Show when={done()} fallback={<p class="sub">ログインを確かめている</p>}>
        {(d) => (
          <Show
            when={d().ok}
            fallback={
              <>
                <h1>つなげなかった</h1>
                <p class="error">{d().error}</p>
                <A class="btn" href="/settings/services">サービスへ戻る</A>
              </>
            }
          >
            {(ok) => (
              <>
                <h1>{ok().name} をつないだ</h1>
                <p class="sub">これで秘書が読めるようになった。</p>
                <A class="btn primary" href={`/settings/services?id=${encodeURIComponent(ok().id)}`}>
                  設定へ戻る
                </A>
              </>
            )}
          </Show>
        )}
      </Show>
    </div>
  );
}
