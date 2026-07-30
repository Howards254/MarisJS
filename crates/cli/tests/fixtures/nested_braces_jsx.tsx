// @runsOn client
type Props = { items: { id: number; name: string }[]; isReady: boolean };

export function NestedBracesJsx(props: Props) {
  return (
    <div>
      <ul>
        {props.items
          .filter((x) => x.id > 0)
          .map((item) => (
            <li key={item.id}>
              {item.name}
              {isReady && <span>(ready)</span>}
            </li>
          ))}
      </ul>
    </div>
  );
}
