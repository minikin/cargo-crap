export function firstTag(post?: { tags?: string[] }) {
  return post?.tags?.[0]
}
export function titleOf(name?: string) {
  return name ?? "untitled"
}
