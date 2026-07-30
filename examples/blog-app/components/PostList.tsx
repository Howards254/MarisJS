// @runsOn server
type PostListProps = { posts: { id: number; title: string; excerpt: string; }[]; };

export function PostList(props: PostListProps) {
  return (
    <ul class="post-list">
      <For each={props.posts} key={(p) => p.id}>
        {(post) => (
          <li class="post-item">
            <h2 class="post-title">{post.title}</h2>
            <p class="post-excerpt">{post.excerpt}</p>
          </li>
        )}
      </For>
    </ul>
  );
}
