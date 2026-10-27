import { createEffect, createMemo, createSignal, For, on, Show } from "solid-js";
import { store } from "../store";
import { confirmAction, toast } from "../ui";
import { CharacterPicker } from "./Pickers";

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
  let fileInput!: HTMLInputElement;
  const commands = [...COMMANDS, { name: "/upload", hint: "上传图片（支持粘贴或拖放）", run: () => fileInput.click() }];
  const [selected, setSelected] = createSignal(0);
  const [dismissed, setDismissed] = createSignal(false);

  const matches = createMemo(() => {
    const text = store.composerText();
    if (!/^\/\S*$/.test(text)) return [];
    return commands.filter((c) => c.name.startsWith(text));
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
    const attachments = store.composerAttachments();
    if ((!text.trim() && attachments.length === 0) || generating() || store.uploadingImages() > 0) return;
    if (attachments.length > 0 && !store.character()?.vision) {
      toast("当前角色不支持图片输入，请选择支持图片的角色");
      return;
    }
    const editing = store.editing();
    store.setComposerText("");
    store.setComposerAttachments([]);
    const ok = await store.runTurn(
      editing ? { action: "edit", node_id: editing.nodeId, content: text, attachment_ids: attachments.map((item) => item.id) } : { action: "send", content: text, attachment_ids: attachments.map((item) => item.id) },
    );
    if (!ok && store.composerText() === "") {
      store.setComposerText(text);
      if (store.composerAttachments().length === 0) store.setComposerAttachments(attachments);
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
        e.stopPropagation();
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
      store.setComposerAttachments([]);
    }
  };

  return (
    <div class="composer-wrap"
      onDragOver={(event) => { if (event.dataTransfer?.types.includes("Files")) event.preventDefault(); }}
      onDrop={(event) => {
        if (!event.dataTransfer?.files.length) return;
        event.preventDefault();
        void store.uploadImages(Array.from(event.dataTransfer.files));
      }}
    >
      <input ref={fileInput} type="file" hidden multiple accept="image/png,image/jpeg,image/webp,image/gif"
        onChange={(event) => {
          void store.uploadImages(Array.from(event.currentTarget.files ?? []));
          event.currentTarget.value = "";
        }}
      />
      <Show when={store.editing()}>
        <div class="editing-bar">
          <span>编辑消息：发送后会产生一个新分支</span>
          <button
            type="button"
            onClick={() => {
              store.setEditing(null);
              store.setComposerText("");
              store.setComposerAttachments([]);
            }}
          >
            取消
          </button>
        </div>
      </Show>
      <div class="composer">
        <Show when={store.composerAttachments().length > 0 || store.uploadingImages() > 0}>
          <div class="attachment-draft">
            <For each={store.composerAttachments()}>{(attachment) => (
              <div class="attachment-preview">
                <img src={attachment.url} alt="待发送的图片" decoding="async" />
                <button type="button" aria-label="移除图片" onClick={() => store.setComposerAttachments((list) => list.filter((item) => item.id !== attachment.id))}>×</button>
              </div>
            )}</For>
            <Show when={store.uploadingImages() > 0}><span role="status">图片上传中…</span></Show>
          </div>
        </Show>
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
          onPaste={(event) => {
            const files = Array.from(event.clipboardData?.items ?? [])
              .filter((item) => item.kind === "file")
              .map((item) => item.getAsFile()).filter((file): file is File => file !== null);
            if (files.length === 0) return;
            event.preventDefault();
            void store.uploadImages(files);
          }}
        />
        <div class="composer-tools">
          <CharacterPicker />
          <button type="button" title="上传图片（也可粘贴或拖放）" aria-label="上传图片" onClick={() => fileInput.click()}>图片</button>
          <Show when={store.character()?.uses_chatgpt_plan}>
            <small class="chatgpt-disclosure">Using ChatGPT plan · <a href="https://chatgpt.com/settings/usage" target="_blank" rel="noopener noreferrer">Manage usage</a></small>
          </Show>
          <Show
            when={generating()}
            fallback={
              <button
                type="button"
                class="send"
                title="发送（Enter）"
                disabled={(!store.composerText().trim() && store.composerAttachments().length === 0) || store.uploadingImages() > 0}
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
