import { formatPrice } from "./utils";

type MissingRunsOnProps = {
  items: string[];
};

export function MissingRunsOn(props: MissingRunsOnProps) {
  return <div>{props.items.length} items</div>;
}
