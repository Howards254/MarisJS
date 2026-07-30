// @runsOn server
import { App } from '../components/App';

type IndexProps = {};

export function Index(props: IndexProps) {
  return (
    <div>
      <App client:hydrate />
    </div>
  );
}
