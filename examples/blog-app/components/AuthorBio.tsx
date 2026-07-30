// @runsOn server
type AuthorBioProps = { authorId: number; };

export function AuthorBio(props: AuthorBioProps) {
  const author = await data(async () => {
    const authors = {
      1: { name: 'Alice Cheng', bio: 'Full-stack developer and author.' },
      2: { name: 'Bob Martinez', bio: 'Open-source contributor since 2015.' },
      3: { name: 'Carol Nguyen', bio: 'Design systems enthusiast.' },
    };
    return authors[props.authorId] || { name: 'Unknown', bio: '' };
  });

  return (
    <div class="author-bio">
      <span class="author-name">{author.name}</span>
      <span class="author-bio">{author.bio}</span>
    </div>
  );
}
