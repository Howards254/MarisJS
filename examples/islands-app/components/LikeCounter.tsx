// @runsOn client
type LikeCounterProps = {};

export function LikeCounter(props: LikeCounterProps) {
  const likes = signal(0);

  function handleLike() {
    likes.set(likes.value + 1);
  }

  return (
    <div class="like-island">
      <span class="like-count">❤️ {likes.value} likes</span>
      <button class="like-btn" onClick={handleLike}>+1</button>
    </div>
  );
}
