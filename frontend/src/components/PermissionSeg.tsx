import { Show } from "solid-js";
import Segs from "~/components/Segs";
import { Permission } from "~/gen/famulen/v1/services_pb";

export const PERMISSIONS: [Permission, string][] = [
  [Permission.AUTO, "まかせる"],
  [Permission.ALLOW, "許可する"],
  [Permission.CONFIRM, "確認する"],
  [Permission.FORBID, "拒否する"],
];

const label = (p: Permission) => PERMISSIONS.find(([r]) => r === p)?.[1];

/**
 * Whether a tool follows the permission it inherits or the owner's override,
 * with the permissions to choose from only for an override. An override
 * starts as what it overrides, so choosing to override changes nothing yet.
 */
export default function PermissionSeg(props: {
  tool: string;
  permission: Permission;
  inherited: Permission;
  onChoose: (r: Permission, done?: string) => void;
}) {
  const own = () => props.permission !== Permission.UNSPECIFIED;
  return (
    <div class="permission-seg">
      <Segs
        label={`${props.tool}の権限の決め方`}
        items={[
          ["default", `既定（${label(props.inherited) ?? "まかせる"}）`],
          ["own", "上書き"],
        ]}
        value={own() ? "own" : "default"}
        onChoose={(way) =>
          way === "own"
            ? props.onChoose(props.inherited, "既定を上書きした")
            : props.onChoose(Permission.UNSPECIFIED, "既定に戻した")
        }
      />
      <Show when={own()}>
        <Segs
          label={`${props.tool}の権限`}
          items={PERMISSIONS.map(([p, text]) => [String(p), text])}
          value={String(props.permission)}
          onChoose={(value) => {
            const chosen = PERMISSIONS.find(([p]) => String(p) === value);
            if (chosen) props.onChoose(chosen[0]);
          }}
        />
      </Show>
    </div>
  );
}
