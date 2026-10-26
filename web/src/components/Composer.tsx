import { createEffect, createMemo, createSignal, For, on, Show } from "solid-js";
import { store } from "../store";
import { confirmAction, toast } from "../ui";
import { EffortPicker, ModelPicker } from "./Pickers";

interface Command {
  name: string;
  hint: string;
  run: () => void;
}

const CLEAR_WARNING = "聊天记录、上传的图片和文件会从服务器上一起删掉，无法恢复。对话本身会保留为空对话。";

const COMMANDS: Command[] = [
  {
    name: "/clear",
    hint: "彻底删除当前对话的全部消息",
    run: async () => {
      const c = store.conv();
      if (!c) return;
      if (await confirmAction("清空当前对话？", CLEAR_WARNING, "彻底清空")) {
        await store.clearConversation(c.id);
      }
    },
  },
  {
    name: "/tree",
    hint: "打开对话树",
    run: () => {
      if (store.conv()) store.setTreeOpen(true);
      else toast("还没有打开对话");
    },
  },
];

export function Composer() {
  let textarea!: HTMLTextAreaElement;
  const [selected, setSelected] = createSignal(0);
  const [dismissed, setDismissed] = createSignal(false);

  const matches = createMemo(() => {
    const text = store.composerText();
    if (!/^\/\S*$/.test(text)) return [];
    return COMMANDS.filter((c) => c.name.startsWith(text));
  });
  const popupOpen = () => matches().length > 0 && !dismissed();

  createEffect(
    on(store.composerText, () => {
      setDismissed(false);
      setSelected(0);
      // Grow with the content up to the CSS max-height.
      textarea.style.height = "auto";
      textarea.style.height = `${textarea.scrollHeight}px`;
    }),
  );

  const generating = () => {
    const t = store.currentLive();
    return t !== undefined && t.error() === null;
  };

  const runCommand = (cmd: Command) => {
    store.setComposerText("");
    cmd.run();
  };

  const send = async () => {
    const text = store.composerText();
    if (!text.trim() || generating()) return;
    const editing = store.editing();
    store.setComposerText("");
    const ok = await store.runTurn(
      editing ? { action: "edit", node_id: editing.nodeId, content: text } : { action: "send", content: text },
    );
    if (!ok && store.composerText() === "") {
      store.setComposerText(text);
      if (editing) store.setEditing(editing);
    }
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.isComposing) return;
    if (popupOpen()) {
      const list = matches();
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const step = e.key === "ArrowDown" ? 1 : -1;
        setSelected((s) => (s + step + list.length) % list.length);
        return;
      }
      if (e.key === "Tab") {
        e.preventDefault();
        store.setComposerText(list[selected()].name);
        return;
      }
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        runCommand(list[selected()]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setDismissed(true);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send();
    } else if (e.key === "Escape" && store.editing()) {
      store.setEditing(null);
      store.setComposerText("");
    }
  };

  return (
    <div class="composer-wrap">
      <Show when={store.editing()}>
        <div class="editing-bar">
          <span>编辑消息：发送后会产生一个新分支</span>
          <button
            type="button"
            onClick={() => {
              store.setEditing(null);
              store.setComposerText("");
            }}
          >
            取消
          </button>
        </div>
      </Show>
      <div class="composer">
        <Show when={popupOpen()}>
          <div class="popup slash-popup">
            <For each={matches()}>
              {(cmd, i) => (
                <button
                  type="button"
                  class="popup-item"
                  classList={{ selected: i() === selected() }}
                  onMouseEnter={() => setSelected(i())}
                  onClick={() => runCommand(cmd)}
                >
                  <span class="cmd-name">{cmd.name}</span>
                  <small>{cmd.hint}</small>
                </button>
              )}
            </For>
          </div>
        </Show>
        <textarea
          ref={textarea}
          rows={1}
          placeholder="输入消息…"
          value={store.composerText()}
          onInput={(e) => store.setComposerText(e.currentTarget.value)}
          onKeyDown={onKeyDown}
        />
        <div class="composer-tools">
          <ModelPicker />
          <EffortPicker />
          <Show
            when={generating()}
            fallback={
              <button
                type="button"
                class="send"
                title="发送（Enter）"
                disabled={!store.composerText().trim()}
                onClick={() => void send()}
              >
                ↑
              </button>
            }
          >
            <button type="button" class="send stop" title="停止生成" onClick={() => store.currentLive()?.abort.abort()}>
              ■
            </button>
          </Show>
        </div>
      </div>
    </div>
  );
}
