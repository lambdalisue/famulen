import { Icon, ListRow } from "~/components/ui";

/** A row of an index page in the narrow layout: icon, name, a line of detail, a count, and the way in. */
export function IndexRow(props: { href: string; icon: string; name: string; detail?: string; count?: number | string }) {
  return (
    <ListRow
      leads
      class="index-row"
      href={props.href}
      lead={
        <span class="index-icon">
          <Icon svg={props.icon} size={20} />
        </span>
      }
      title={props.name}
      sub={props.detail}
      end={props.count !== undefined && props.count !== "" ? <span class="mono sub">{props.count}</span> : undefined}
    />
  );
}
