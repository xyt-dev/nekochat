import { createMemo, For, Show } from "solid-js";
import type { PickerModel } from "../api";
import { store } from "../store";
import { createPopover } from "./Popover";

/** Listed models grouped by provider, in the admin's order; each shows its effort levels. */
export function ModelPicker() {
  const [open, setOpen, root] = createPopover();
  const groups = createMemo(() => {
    const byProvider = new Map<string, PickerModel[]>();
    for (const m of store.models().models) {
      let list = byProvider.get(m.provider_label);
      if (!list) byProvider.set(m.provider_label, (list = []));
      list.push(m);
    }
    return [...byProvider.entries()];
  });
  return (
    <div class="picker" ref={root}>
      <button
        type="button"
        class="pill"
        title={store.model()?.label ?? "没有可用的模型"}
        onClick={() => setOpen((o) => !o)}
      >
        {store.model()?.display_name ?? "无模型"} <span class="pill-caret">▾</span>
      </button>
      <Show when={open()}>
        <div class="popup popup-right model-popup">
          <Show when={groups().length > 0} fallback={<div class="popup-empty">管理端还没有上架模型</div>}>
            <For each={groups()}>
              {([provider, models]) => (
                <div class="popup-group">
                  <div class="popup-group-title">{provider}</div>
                  <For each={models}>
                    {(m) => (
                      <button
                        type="button"
                        class="popup-item"
                        classList={{ selected: store.model()?.id === m.id }}
                        onClick={() => {
                          store.chooseModel(m);
                          setOpen(false);
                        }}
                      >
                        <span>{m.display_name}</span>
                        <small>{m.reasoning.levels.join(" / ")}</small>
                      </button>
                    )}
                  </For>
                </div>
              )}
            </For>
          </Show>
        </div>
      </Show>
    </div>
  );
}

/** Reasoning effort of the chosen model; absent for models without levels. */
export function EffortPicker() {
  const [open, setOpen, root] = createPopover();
  const levels = () => store.model()?.reasoning.levels ?? [];
  return (
    <Show when={levels().length > 0}>
      <div class="picker" ref={root}>
        <button type="button" class="pill" onClick={() => setOpen((o) => !o)}>
          思考 <b>{store.effort()}</b> <span class="pill-caret">▾</span>
        </button>
        <Show when={open()}>
          <div class="popup popup-right effort-popup">
            <For each={levels()}>
              {(level) => (
                <button
                  type="button"
                  class="popup-item"
                  classList={{ selected: store.effort() === level }}
                  onClick={() => {
                    store.chooseEffort(level);
                    setOpen(false);
                  }}
                >
                  <span>{level}</span>
                  <Show when={store.model()?.reasoning.default === level}>
                    <small>默认</small>
                  </Show>
                </button>
              )}
            </For>
          </div>
        </Show>
      </div>
    </Show>
  );
}
