// @runsOn server
import { PostList } from '../components/PostList';
import { AuthorBio } from '../components/AuthorBio';

type IndexProps = {};

export function Index(props: IndexProps) {
  // Multiple concurrent data() calls — v1 behavior: these are
  // declared sequentially in the source but executed as independent
  // async operations. In Node.js, each await resolves individually;
  // if one is slow, the next doesn't start until the first completes
  // (serialized by await).
  const posts = await data(async () => [
    { id: 1, title: 'Getting Started', authorId: 1, excerpt: 'Learn the basics.' },
    { id: 2, title: 'Advanced Patterns', authorId: 2, excerpt: 'Deep dive into signals.' },
    { id: 3, title: 'Server Rendering', authorId: 1, excerpt: 'How SSR works.' },
    { id: 4, title: 'Styling Guide', authorId: 3, excerpt: 'CSS conventions.' },
    { id: 5, title: 'Deployment', authorId: 2, excerpt: 'Ship to production.' },
  ]);

  // Second data() call — fetches featured post metadata
  const featured = await data(async () => ({
    title: 'Featured: Getting Started',
    readCount: 1247,
  }));

  return (
    <div>
      <h1>Blog</h1>
      <div class="featured">
        <span class="featured-title">{featured.title}</span>
        <span class="featured-reads">{featured.readCount} reads</span>
      </div>
      <PostList posts={posts} />
      <AuthorBio authorId={1} />
    </div>
  );
}
