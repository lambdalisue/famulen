import { useOwner } from "~/lib/owner";
import { iconFile } from "~/lib/portrait";

/** The secretary's icon. The name beside it says who it is, so it is only seen. */
export default function AideIcon(props: { size: number }) {
  const owner = useOwner();
  return <img class="aide-icon" src={iconFile(owner.portrait(), "app-icon.svg")} alt="" width={props.size} height={props.size} />;
}
