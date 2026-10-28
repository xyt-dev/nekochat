import { batch, createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { store } from "../store";
import { confirmAction, toast } from "../ui";
import { CharacterPicker } from "./Pickers";
import { formatFileSize } from "./Files";

interface Command { name: string; hint: string; run: () => void; }
const CLEAR_WARNING = "聊天记录、上传的图片和文件会从服务器上一起删掉，无法恢复。对话本身会保留为空对话。";
const COMMANDS: Command[] = [
  { name: "/clear", hint: "彻底删除当前对话的全部消息", run: async () => {
    const c = store.conv();
    if (c && await confirmAction("清空当前对话？", CLEAR_WARNING, "彻底清空")) await store.clearConversation(c.id);
  } },
  { name: "/tree", hint: "打开对话树", run: () => {
    if (store.conv()) store.setTreeOpen(true); else toast("还没有打开对话");
  } },
];

export function Composer() {
  let editor!: HTMLDivElement;
  let fileInput!: HTMLInputElement;
  let composing = false;
  let savedRange: Range | null = null;
  let renderedImages = new Set<string>();
  const [compositionRevision, setCompositionRevision] = createSignal(0);
  const commands = [...COMMANDS, { name: "/upload", hint: "上传图片、PDF 或文本文件", run: () => fileInput.click() }];
  const [selected, setSelected] = createSignal(0);
  const [dismissed, setDismissed] = createSignal(false);
  const documents = createMemo(() => store.composerAttachments().filter((item) => item.kind === "document"));
  const matches = createMemo(() => {
    const text = store.composerText();
    return /^\/\S*$/.test(text) ? commands.filter((command) => command.name.startsWith(text)) : [];
  });
  const popupOpen = () => matches().length > 0 && !dismissed();
  const generating = () => {
    const turn = store.currentLive();
    return turn !== undefined && turn.error() === null;
  };

  // Only input events walk the small draft DOM; streaming/history never touches it.
  const readText = (node: Node): string => {
    if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? "";
    if (!(node instanceof HTMLElement) || node.dataset.attachmentId) return "";
    if (node.tagName === "BR") return "\n";
    let text = "";
    for (const child of node.childNodes) {
      if (child instanceof HTMLElement && /^(DIV|P)$/.test(child.tagName) && text && !text.endsWith("\n")) text += "\n";
      text += readText(child);
    }
    return text;
  };
  const rememberCaret = () => {
    const selection = window.getSelection();
    if (selection?.rangeCount && editor.contains(selection.anchorNode)) savedRange = selection.getRangeAt(0).cloneRange();
  };
  const insertNode = (node: Node) => {
    const range = savedRange && editor.contains(savedRange.commonAncestorContainer) ? savedRange : document.createRange();
    if (range !== savedRange) { range.selectNodeContents(editor); range.collapse(false); }
    range.deleteContents();
    range.insertNode(node);
    range.setStartAfter(node);
    range.collapse(true);
    savedRange = range.cloneRange();
    if (document.activeElement === editor) {
      const selection = window.getSelection();
      selection?.removeAllRanges();
      selection?.addRange(range);
    }
  };
  const syncInput = () => {
    if (composing) return;
    const ids = new Set(Array.from(editor.querySelectorAll<HTMLElement>("[data-attachment-id]")).map((node) => node.dataset.attachmentId));
    batch(() => {
      store.setComposerText(readText(editor));
      store.setComposerAttachments((list) => list.filter((item) => item.kind !== "image" || !renderedImages.has(item.id) || ids.has(item.id)));
    });
    rememberCaret();
  };

  createEffect(() => {
    const text = store.composerText();
    const attachments = store.composerAttachments();
    compositionRevision();
    setDismissed(false);
    setSelected(0);
    if (composing) return;
    // External changes (edit/send/account reset) replace text. Ordinary typing leaves the caret alone.
    if (readText(editor) !== text) {
      editor.replaceChildren(document.createTextNode(text));
      savedRange = null;
    }
    const images = attachments.filter((item) => item.kind === "image");
    const ids = new Set(images.map((item) => item.id));
    for (const node of editor.querySelectorAll<HTMLElement>("[data-attachment-id]")) {
      if (!ids.has(node.dataset.attachmentId!)) node.remove();
    }
    let insertedImage = false;
    for (const attachment of images) {
      if (Array.from(editor.querySelectorAll<HTMLElement>("[data-attachment-id]")).some((node) => node.dataset.attachmentId === attachment.id)) continue;
      const chip = document.createElement("span");
      chip.className = "editor-image";
      chip.contentEditable = "false";
      chip.dataset.attachmentId = attachment.id;
      const image = document.createElement("img");
      image.src = attachment.url;
      image.alt = attachment.name;
      image.decoding = "async";
      image.draggable = false;
      const remove = document.createElement("button");
      remove.type = "button";
      remove.textContent = "×";
      remove.setAttribute("aria-label", `移除图片 ${attachment.name}`);
      remove.addEventListener("click", () => {
        chip.remove();
        store.setComposerAttachments((list) => list.filter((item) => item.id !== attachment.id));
        editor.focus();
      });
      chip.append(image, remove);
      insertNode(chip);
      insertedImage = true;
    }
    if (insertedImage) syncInput();
    renderedImages = ids;
    editor.dataset.empty = String(!readText(editor) && images.length === 0);
  });

  const cancelEdit = () => batch(() => {
    store.setEditing(null);
    store.setComposerText("");
    store.setComposerAttachments([]);
  });
  const runCommand = (command: Command) => {
    store.setComposerText("");
    command.run();
  };
  const send = async () => {
    const text = store.composerText();
    const attachments = store.composerAttachments();
    if ((!text.trim() && attachments.length === 0) || generating() || store.uploadingAttachments() > 0) return;
    if (attachments.some((item) => item.kind === "image") && !store.character()?.vision) {
      toast("当前角色不支持图片输入，请选择支持图片的角色");
      return;
    }
    const editing = store.editing();
    const version = store.sessionVersion();
    const conversationId = store.conv()?.id;
    batch(() => { store.setComposerText(""); store.setComposerAttachments([]); });
    const ok = await store.runTurn(editing
      ? { action: "edit", node_id: editing.nodeId, content: text, attachment_ids: attachments.map((item) => item.id) }
      : { action: "send", content: text, attachment_ids: attachments.map((item) => item.id) });
    if (!ok && version === store.sessionVersion() && (!conversationId || conversationId === store.conv()?.id) && !store.composerText() && store.composerAttachments().length === 0) {
      batch(() => { store.setComposerText(text); store.setComposerAttachments(attachments); if (editing) store.setEditing(editing); });
    }
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if ((event.target as HTMLElement).closest("[data-attachment-id] button")) return;
    if (composing || event.isComposing || event.keyCode === 229) return;
    if (popupOpen()) {
      const list = matches();
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        setSelected((index) => (index + (event.key === "ArrowDown" ? 1 : -1) + list.length) % list.length);
        return;
      }
      if (event.key === "Tab") { event.preventDefault(); store.setComposerText(list[selected()].name); return; }
      if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); runCommand(list[selected()]); return; }
      if (event.key === "Escape") { event.preventDefault(); setDismissed(true); return; }
    }
    if (event.key === "Enter") {
      event.preventDefault();
      if (event.shiftKey) { rememberCaret(); insertNode(document.createTextNode("\n")); syncInput(); }
      else void send();
    } else if (event.key === "Escape" && store.editing()) cancelEdit();
  };

  return (
    <div class="composer-wrap"
      onDragOver={(event) => event.preventDefault()}
      onDrop={(event) => {
        event.preventDefault(); rememberCaret();
        const files = Array.from(event.dataTransfer?.files ?? []);
        if (files.length) void store.uploadAttachments(files);
        else {
          const text = event.dataTransfer?.getData("text/plain");
          if (text) { insertNode(document.createTextNode(text)); syncInput(); }
        }
      }}>
      <input ref={fileInput} type="file" hidden multiple
        accept="image/png,image/jpeg,image/webp,image/gif,application/pdf,text/*,.pdf,.md,.markdown,.mdown,.txt,.text,.json,.jsonl,.ndjson,.csv,.tsv,.log,.rs,.py,.js,.jsx,.ts,.tsx,.mjs,.cjs,.css,.scss,.less,.html,.htm,.xml,.yaml,.yml,.toml,.ini,.cfg,.conf,.sh,.bash,.zsh,.fish,.ps1,.c,.h,.cc,.cpp,.hpp,.cs,.java,.go,.rb,.php,.sql,.vue,.svelte,.swift,.kt,.kts,.r,.lua,.tex,.rst,.adoc,.dockerfile,.dart,.ex,.exs,.erl,.hrl,.clj,.cljs,.scala,.pl,.pm,.bat"
        onChange={(event) => { void store.uploadAttachments(Array.from(event.currentTarget.files ?? [])); event.currentTarget.value = ""; }} />
      <Show when={store.editing()}><div class="editing-bar"><span>编辑消息：发送后会产生一个新分支</span><button type="button" onClick={cancelEdit}>取消</button></div></Show>
      <Show when={documents().length > 0}>
        <div class="document-drafts" aria-label="待发送文档">
          <For each={documents()}>{(attachment) => (
            <div class="document-card"><a href={attachment.url} download={attachment.name}><span>{attachment.name}</span><small>{formatFileSize(attachment.size)} · 下载</small></a>
              <button type="button" aria-label={`移除文档 ${attachment.name}`} onClick={() => store.setComposerAttachments((list) => list.filter((item) => item.id !== attachment.id))}>×</button>
            </div>
          )}</For>
        </div>
      </Show>
      <div class="composer">
        <Show when={popupOpen()}><div class="popup slash-popup"><For each={matches()}>{(command, index) => (
          <button type="button" class="popup-item" classList={{ selected: index() === selected() }} onMouseEnter={() => setSelected(index())} onClick={() => runCommand(command)}>
            <span class="cmd-name">{command.name}</span><small>{command.hint}</small>
          </button>
        )}</For></div></Show>
        <div ref={editor} class="composer-editor" contentEditable role="textbox" aria-label="输入消息" aria-multiline="true" data-placeholder="输入消息…" data-empty="true"
          onInput={syncInput} onKeyDown={onKeyDown} onKeyUp={rememberCaret} onMouseUp={rememberCaret} onBlur={rememberCaret}
          onCompositionStart={() => { composing = true; }} onCompositionEnd={() => { composing = false; batch(() => { syncInput(); setCompositionRevision((revision) => revision + 1); }); }}
          onPaste={(event) => {
            event.preventDefault(); rememberCaret();
            const files = Array.from(event.clipboardData?.files ?? []);
            const text = event.clipboardData?.getData("text/plain") ?? "";
            if (text) { insertNode(document.createTextNode(text)); syncInput(); }
            if (files.length) void store.uploadAttachments(files);
          }} />
        <Show when={store.uploadingAttachments() > 0}><small class="upload-status" role="status">附件上传中…</small></Show>
        <div class="composer-tools">
          <CharacterPicker />
          <button type="button" title="上传图片、PDF 或文本（也可粘贴或拖放）" aria-label="上传附件" onClick={() => { rememberCaret(); fileInput.click(); }}>附件</button>
          <Show when={generating()} fallback={<button type="button" class="send" title="发送（Enter）"
            disabled={(!store.composerText().trim() && store.composerAttachments().length === 0) || store.uploadingAttachments() > 0} onClick={() => void send()}>↑</button>}>
            <button type="button" class="send stop" title="停止生成" onClick={() => store.currentLive()?.abort.abort()}>■</button>
          </Show>
        </div>
      </div>
    </div>
  );
}
