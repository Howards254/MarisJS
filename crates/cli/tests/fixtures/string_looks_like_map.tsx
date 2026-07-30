// @runsOn client
type Props = { name: string };

export function StringLooksLikeMap(props: Props) {
  const doc = "Never use .map() inside JSX, use <For> instead";
  return <div>{props.name}</div>;
}
