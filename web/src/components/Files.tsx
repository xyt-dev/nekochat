import { Show } from "solid-js";
import { localImageUrl } from "../localImages";

export function formatFileSize(size: number): string {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KiB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MiB`;
}

export function Avatar(props: { name: string; url?: string | null }) {
  return <span class="avatar" aria-hidden="true"><Show when={localImageUrl(props.url)} fallback={Array.from(props.name.trim())[0]?.toUpperCase() ?? "?"}>
    {(url) => <img src={url()} alt="" decoding="async" />}
  </Show></span>;
}
