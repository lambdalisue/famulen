import { createAsync, query } from "@solidjs/router";
import { api } from "~/lib/api";
import { workgroup } from "~/lib/workgroup";
import * as icons from "~/lib/icons";
import { setView } from "~/lib/view";
import { IndexRow } from "~/components/IndexRow";
import { List } from "~/components/ui";

const countsQuery = query(async (workgroup: string) => {
  const [parties, things, contracts, layers] = await Promise.all([
    api.records.listRecords({ kind: "parties", workgroup }),
    api.records.listRecords({ kind: "holdings", workgroup }),
    api.records.listRecords({ kind: "contracts", workgroup }),
    api.memory.listLayers({}),
  ]);
  return {
    parties: parties.records.length,
    things: things.records.length,
    contracts: contracts.records.length,
    pending: layers.layers.find((l) => l.id === "pending")?.count ?? "",
  };
}, "more-counts");

/** The tabs that do not fit in the narrow layout's bottom bar. */
export default function More() {
  setView("ほか");
  const counts = createAsync(() => countsQuery(workgroup()));
  return (
    <List>
      <IndexRow href="/parties" icon={icons.users} name="相手" detail="人と会社・チーム・グループ" count={counts()?.parties} />
      <IndexRow href="/things" icon={icons.deviceLaptop} name="物" detail="機器 · 期限の近い順" count={counts()?.things} />
      <IndexRow
        href="/contracts"
        icon={icons.fileCertificate}
        name="契約"
        detail="ライセンス・サブスクリプション・回線や保険"
        count={counts()?.contracts}
      />
      <IndexRow href="/memory/menu" icon={icons.brain} name="記憶" detail={`夢 · 保留中の判断 ${counts()?.pending ?? ""}`} />
      <IndexRow href="/settings/menu" icon={icons.settings} name="設定" detail="ワークグループ · 秘書 · つながり" />
    </List>
  );
}
