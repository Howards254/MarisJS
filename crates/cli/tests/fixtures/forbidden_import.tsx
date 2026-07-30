// @runsOn client
import { useState } from "react";

type ForbiddenImportProps = {
  user: string;
};

export function ForbiddenImport(props: ForbiddenImportProps) {
  return <div>Hello {props.user}</div>;
}
