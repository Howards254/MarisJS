// @runsOn server
import { App } from '../components/App';
import { Newsletter } from '../components/Newsletter';

type IndexProps = {};

export function Index(props: IndexProps) {
  return (
    <div>
      <App client:hydrate />
      <Newsletter client:hydrate />
    </div>
  );
}
