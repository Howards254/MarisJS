// @runsOn client
type CardProps = { title: string; children: JSX.Element; };
export function Page(props: PageProps) {
  return (
    <Card title="Hi">
      <p>First</p>
      <p>Second</p>
    </Card>
  );
}