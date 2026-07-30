// @runsOn server
import { SettingsForm } from '../components/SettingsForm';

type IndexProps = {};

export function Index(props: IndexProps) {
  return (
    <div>
      <SettingsForm client:hydrate />
    </div>
  );
}
