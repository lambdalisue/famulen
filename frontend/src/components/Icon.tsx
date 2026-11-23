/** A Tabler outline icon from its raw SVG, drawn in the current text color. */
export default function Icon(props: { svg: string; size?: number }) {
  const body = () =>
    props.svg
      .replace(/^[\s\S]*?<svg[^>]*>/, "")
      .replace(/<\/svg>\s*$/, "")
      .replace(/<path stroke="none" d="M0 0h24v24H0z" fill="none"\s*\/>/, "")
      .trim();
  return (
    <svg
      class="icon"
      width={props.size ?? 16}
      height={props.size ?? 16}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.75"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
      innerHTML={body()}
    />
  );
}
