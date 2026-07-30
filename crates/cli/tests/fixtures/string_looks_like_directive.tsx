// @runsOn client
type Props = { name: string };

export function StringLooksLikeDirective(props: Props) {
  const help = "Every component needs // @runsOn client or server";
  return <div>{props.name}</div>;
}
