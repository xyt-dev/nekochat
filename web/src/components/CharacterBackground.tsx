import { Show } from "solid-js";
import { BackgroundImage } from "./BackgroundImage";
import { store } from "../store";

/** No message or live-turn dependency: token updates never repaint this decoration. */
export function CharacterBackground() {
  return <Show when={store.character()?.background}>{(background) =>
    <div class="character-background" aria-hidden="true"><BackgroundImage background={background()} /></div>
  }</Show>;
}
