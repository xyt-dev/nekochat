import { For, Show } from "solid-js";
import { store } from "../store";
import { createPopover } from "./Popover";
import { Avatar } from "./Files";

/** Only server-authorized public cards are available; recipes remain administrator-only. */
export function CharacterPicker() {
  const [open, setOpen, root] = createPopover();
  let trigger!: HTMLButtonElement;
  return (
    <div class="picker sidebar-character" ref={root} onKeyDown={(event) => {
      if (event.key === "Escape" && open()) { event.preventDefault(); event.stopPropagation(); setOpen(false); trigger.focus(); }
    }}>
      <button ref={trigger} type="button" class="role-card" aria-label="选择角色" aria-expanded={open()} aria-controls="character-options" onClick={() => {
        const next = !open(); setOpen(next);
        if (next) void store.loadCharacters();
      }}>
        <Show when={store.character()}>{(character) => <Avatar name={character().display_name} url={character().avatar} />}</Show>
        <span class="role-card-name">{store.character()?.display_name ?? "无可用角色"}</span><span class="role-card-caret" aria-hidden="true">▾</span>
      </button>
      <Show when={open()}>
        <div id="character-options" class="popup character-popup" aria-label="可用角色">
          <Show when={store.characters().characters.length > 0} fallback={<div class="popup-empty">请管理员配置并启用角色</div>}>
            <For each={store.characters().characters}>{(character) => (
              <button type="button" class="popup-item" aria-pressed={store.character()?.id === character.id}
                classList={{ selected: store.character()?.id === character.id }} onClick={() => {
                  store.chooseCharacter(character); setOpen(false); trigger.focus();
                }}><Avatar name={character.display_name} url={character.avatar} /><span>{character.display_name}</span></button>
            )}</For>
            <div class="popup-empty">从下一轮开始使用，保留当前对话</div>
          </Show>
        </div>
      </Show>
    </div>
  );
}
