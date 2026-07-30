// @runsOn server
import { SearchBar } from '../components/SearchBar';
import { LikeCounter } from '../components/LikeCounter';
import { ThemeToggle } from '../components/ThemeToggle';

type IndexProps = {};

export function Index(props: IndexProps) {
  return (
    <div>
      <h1>Product Catalog</h1>
      <SearchBar client:hydrate />
      <hr />
      <LikeCounter client:hydrate />
      <hr />
      <ThemeToggle client:hydrate />
    </div>
  );
}
