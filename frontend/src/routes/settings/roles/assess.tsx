import GatewayLog from "~/components/logs/GatewayLog";
import LogSheet from "~/components/logs/LogSheet";
import RoleModel from "~/components/RoleModel";
import { Role } from "~/gen/famulen/v1/settings_pb";
import { setView } from "~/lib/view";
import { PageHeader } from "~/components/ui";

/** Assessing: whether a tool call left to the auto permission may run without asking the owner. */
export default function Assess() {
  setView("設定", "見極め");
  return (
    <>
      <PageHeader
        title="見極め"
        note={
          <>
            <p>
              権限が「まかせる」の道具で、読むだけでない呼び出しが来るたびに、そのまま使ってよいかを判断するエージェント。道具と中身、あなたが直前に頼んだこと、記憶の短いまとめを読み、頼まれたとおりで、取り返しがつき、外の人を巻き込まないときだけ通す。そうでなければ、あなたに確かめる。
            </p>
            <p>
              消す・外の人へ送る・招待や辞退を知らせる・お金を動かす・権限や金庫を変えるものは、見極めが通しても必ず確かめる。上限に達した日や、モデルが使えないときも確かめる側に倒す。
            </p>
            <p>
              見極めが何をどう判断したかは、頁の下の「ログを見る」で開く。道具ごとの権限は<a href="/settings/services">サービス</a>で選ぶ。
            </p>
          </>
        }
      />
      <div class="prefs">
        <RoleModel
          role={Role.ASSESS}
          cap="上限に達した日は、「まかせる」の道具の書き込みをすべてあなたに確かめる。値段のあるモデルは円で、ないモデルは回数で数える。"
        />
      </div>
      <LogSheet title="見極めのログ">
        <GatewayLog actor="assessor" />
      </LogSheet>
    </>
  );
}
