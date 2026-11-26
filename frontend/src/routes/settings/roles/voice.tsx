import RoleModel from "~/components/RoleModel";
import { Role } from "~/gen/famulen/v1/settings_pb";
import { setView } from "~/lib/view";
import { PageHeader } from "~/components/ui";

/** The voice: what the owner says aloud to the Even G2, as text. */
export default function Voice() {
  setView("設定", "声の聞き取り");
  return (
    <>
      <PageHeader
        title="声の聞き取り"
        note={
          <>
            <p>
              Even G2 に話しかけた声を文字にするモデル。文字になったものが、コンソールに書いたのと同じように秘書に届く。声は文字にするあいだだけ使い、残さない。
            </p>
            <p>
              選べるのは、<a href="/settings/models">登録したモデル</a>
              のうち文字起こしのもの。手元で動く whisper のサーバーか、文字起こしの API を足す。モデルを選ぶまで、Even G2 では話せず、そう出る。
            </p>
          </>
        }
      />
      <div class="prefs">
        <RoleModel
          role={Role.VOICE}
          cap="上限に達した日は、Even G2 で話せない。話しているあいだも途中の言葉を何度か聞き取るので、一度話すと数回に数える。途中の聞き取りは、話し終えたときの分が残るうちだけする。1 回ごとの値段のあるモデルは円で、ほかは回数で数える。"
        />
      </div>
    </>
  );
}
