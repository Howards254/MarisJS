// @runsOn client
type FormProps = {};

export function SettingsForm(props: FormProps) {
  const name = signal('');
  const age = signal(30);
  const terms = signal(false);
  const theme = signal('light');
  const saved = signal(false);

  // Validation computed — each depends on its own signal
  const nameValid = computed(() => name.value.trim().length > 0);
  const ageValid = computed(() => age.value >= 1 && age.value <= 120);
  const termsAccepted = computed(() => terms.value === true);
  const formValid = computed(() => nameValid.value && ageValid.value && termsAccepted.value);

  // Named event handlers — deliberately multiple, per Milestone 4h
  function handleNameInput(e) {
    name.set(e.target.value);
  }

  function handleAgeChange(e) {
    const val = Number(e.target.value);
    age.set(isNaN(val) ? 0 : val);
  }

  function handleTermsChange(e) {
    terms.set(e.target.checked);
  }

  function handleThemeChange(e) {
    theme.set(e.target.value);
    saved.set(false);
  }

  function handleSubmit(e) {
    e.preventDefault();
    saved.set(true);
  }

  return (
    <div class="settings-form">
      <h2>Account Settings</h2>

      <div class="field">
        <label for="name">Name *</label>
        <input
          id="name"
          type="text"
          class="text-input"
          value={name.value}
          onInput={handleNameInput}
          placeholder="Enter your name"
        />
        {nameValid.value ? <span /> : <span class="error name-error">Name is required.</span>}
      </div>

      <div class="field">
        <label for="age">Age</label>
        <input
          id="age"
          type="number"
          class="text-input"
          value={age.value}
          onInput={handleAgeChange}
        />
        {ageValid.value ? <span /> : <span class="error age-error">Age must be 1–120.</span>}
      </div>

      <div class="field">
        <label for="theme">Theme</label>
        <select
          id="theme"
          class="select-input"
          value={theme.value}
          onChange={handleThemeChange}
        >
          <option value="light">Light</option>
          <option value="dark">Dark</option>
          <option value="system">System</option>
        </select>
      </div>

      <div class="field checkbox-field">
        <label>
          <input
            type="checkbox"
            class="terms-checkbox"
            checked={terms.value}
            onChange={handleTermsChange}
          />
          I accept the terms and conditions
        </label>
        {termsAccepted.value ? <span /> : <span class="error terms-error">You must accept the terms.</span>}
      </div>

      <div class="actions">
        <button
          type="submit"
          class="save-btn"
          disabled={!formValid.value}
          onClick={handleSubmit}
        >
          Save Settings
        </button>
        {saved.value ? <span class="success-msg">Settings saved!</span> : <span />}
      </div>
    </div>
  );
}
