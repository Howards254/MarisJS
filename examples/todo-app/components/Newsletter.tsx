// @runsOn client
// Independent interactive island — must not interfere with App island
type NewsletterProps = {};

export function Newsletter(props: NewsletterProps) {
  const email = signal('');
  const subscribed = signal(false);

  function handleSubmit(e) {
    e.preventDefault();
    if (email.value.trim()) {
      subscribed.set(true);
    }
  }

  return (
    <div class="newsletter">
      <h2>Newsletter</h2>
      {subscribed.value ? (
        <p class="success-msg">Subscribed!</p>
      ) : (
        <form onSubmit={handleSubmit}>
          <input
            class="email-input"
            type="text"
            placeholder="email"
            value={email.value}
            onInput={(e) => email.set(e.target.value)}
          />
          <button type="submit" class="subscribe-btn">Subscribe</button>
        </form>
      )}
    </div>
  );
}
