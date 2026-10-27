import { createMemo, Show } from "solid-js";
import { localImageUrl } from "../localImages";
import { store } from "../store";

/** No message or live-turn dependency: token updates never repaint this decoration. */
export function CharacterBackground() {
  const background = createMemo(() => {
    const value = store.character()?.background;
    const url = localImageUrl(value?.url);
    return value && url ? { url, opacity: Math.max(0, Math.min(1, value.opacity)) } : null;
  });
  return <Show when={background()}>{(value) => <div class="character-background" aria-hidden="true" style={{
    "background-image": `url("${value().url}")`, opacity: value().opacity,
  }} />}</Show>;
}
