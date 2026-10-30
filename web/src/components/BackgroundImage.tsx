import { createMemo, Show } from "solid-js";
import type { CharacterBackground } from "../api";
import { localImageUrl } from "../localImages";

/** Shared by the chat decoration and editor preview; no message or live-turn state. */
export function BackgroundImage(props: { background: CharacterBackground | null | undefined; alt?: string }) {
  const background = createMemo(() => {
    const value = props.background;
    const url = localImageUrl(value?.url);
    return value && url ? { url, opacity: Math.max(0, Math.min(1, value.opacity)), position: value.position ?? "center" } : null;
  });
  return <Show when={background()}>{(value) => <img
    class="character-background-image" src={value().url} alt={props.alt ?? ""}
    data-position={value().position} decoding="async" draggable={false}
    style={{ opacity: value().opacity }}
  />}</Show>;
}
