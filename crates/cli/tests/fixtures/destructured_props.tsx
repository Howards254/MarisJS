// @runsOn server
import type { User } from "./types";

type DestructuredPropsProps = {
  user: User;
};

export function DestructuredProps({ user }: DestructuredPropsProps) {
  return <div>{user.name}</div>;
}
