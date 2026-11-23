import { createAsync } from "@solidjs/router";
import LogSheet from "~/components/logs/LogSheet";
import TriageLog from "~/components/logs/TriageLog";
import RoleModel from "~/components/RoleModel";
import { Role } from "~/gen/famulen/v1/settings_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { workspacesQuery } from "~/lib/connections";
import { useOwner } from "~/lib/owner";
import { setView } from "~/lib/view";

/** How long a channel stays attended after the secretary is mentioned in it. */
function AttentionSection() {
  const workspaces = createAsync(() => workspacesQuery());
  const owner = useOwner();
  const name = owner.aide;
  return (
    <section class="pref">
      <div class="pref-head">
        <h2>注視の長さ</h2>
        <p class="sub">
          チャンネルで {name()} をメンションしてから、そのチャンネルを注視する長さ。そのあいだは、発言が届くたびにトリアージが{" "}
          {name()} に向けたものかをその場で確かめ、向けたものならメンションしなくてもすぐ応じる。話しかけるたびに延びる。0
          分にすると注視しない。注視していないチャンネルの発言も、見張りがまとめて読み、関わるものは {name()} に届く。
        </p>
      </div>
      <label class="row" style={{ gap: "6px" }}>
        <input
          type="number"
          class="input"
          min="0"
          max="120"
          style={{ width: "80px" }}
          aria-label="注視する分"
          value={workspaces()?.attentionMinutes ?? 10}
          onChange={(e) => {
            const minutes = Math.max(0, Math.min(120, Math.round(Number(e.currentTarget.value) || 0)));
            void act(() => api.channels.setAttention({ minutes }));
          }}
        />
        分
      </label>
    </section>
  );
}

/** Triage: whether a message in a watched Slack channel is meant for the secretary. */
export default function Triage() {
  setView("設定", "トリアージ");
  return (
    <>
      <h1>トリアージ</h1>
      <p class="sub">
        秘書をメンションしたあと、しばらく注視している Slack のチャンネルで、書かれた発言が秘書に向けたものか、秘書との会話の続きかだけを、届いたその場で問うエージェント。秘書宛てなら秘書に渡し、そうでなければ何もしない。見張りとは別に動く。
      </p>
      <div class="prefs">
        <RoleModel
          role={Role.TRIAGE}
          cap="上限に達した日は、注視しているチャンネルでもメンションにだけ応じる。値段のあるモデルは円で、ないモデルは回数で数える。"
        />
        <AttentionSection />
      </div>
      <LogSheet title="トリアージのログ">
        <TriageLog />
      </LogSheet>
    </>
  );
}
