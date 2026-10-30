/** Image URLs are server-owned local assets, never arbitrary URLs or filesystem paths. */
export function localImageUrl(value: string | null | undefined): string | null {
  if (!value || value.includes("..")) return null;
  return /^\/api\/(?:avatars\/[A-Za-z0-9_-]+|resources\/[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+|backgrounds\/[A-Za-z0-9_-]+\.(?:png|jpe?g|webp|gif|avif))$/.test(value) ? value : null;
}
