// @runsOn client
type ThemeToggleProps = {};

export function ThemeToggle(props: ThemeToggleProps) {
  const dark = signal(false);

  function handleToggle() {
    dark.set(!dark.value);
  }

  return (
    <div class={'theme-island ' + (dark.value ? 'theme-dark' : 'theme-light')}>
      <span class="theme-label">{dark.value ? 'Dark' : 'Light'} mode</span>
      <button class="theme-btn" onClick={handleToggle}>Toggle Theme</button>
    </div>
  );
}
