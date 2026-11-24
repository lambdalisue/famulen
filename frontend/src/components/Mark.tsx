import mark from "../assets/mark.svg?raw";

/** The mark's drawing, cropped to what it draws (18..82 of its 100 box). */
const drawing = mark.replace(
  /<svg [^>]*>/,
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="14 14 72 72" width="100%" height="100%" aria-hidden="true">',
);

/** The Famulen mark: the figure in the current text color, its notebook in red. */
export default function Mark(props: { size?: number }) {
  const size = () => props.size ?? 24;
  return <span class="mark" style={{ display: "inline-flex", width: `${size()}px`, height: `${size()}px` }} innerHTML={drawing} />;
}
