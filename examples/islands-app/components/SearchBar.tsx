// @runsOn client
type SearchBarProps = {};

export function SearchBar(props: SearchBarProps) {
  const query = signal('');
  const visible = signal(false);

  function handleInput(e) {
    query.set(e.target.value);
    visible.set(e.target.value.trim().length > 0);
  }

  function handleClear() {
    query.set('');
    visible.set(false);
  }

  return (
    <div class="search-island">
      <input
        type="text"
        class="search-input"
        placeholder="Search products..."
        value={query.value}
        onInput={handleInput}
      />
      <button class="clear-btn" onClick={handleClear}>Clear</button>
      {visible.value ? (
        <span class="search-status">Searching for: {query.value}</span>
      ) : (
        <span />
      )}
    </div>
  );
}
