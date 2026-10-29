import { batch, createEffect, createMemo, createSignal, For, onCleanup, onMount } from "solid-js";
import { store } from "../store";
import { outline, type OutlineRow } from "../tree";

function snippet(text: string): string {
  const line = text.trim().split("\n", 1)[0] ?? "";
  return line.length > 80 ? `${line.slice(0, 80)}…` : line || "（空）";
}

/** `/tree`: every node as an outline; ↑↓ + Enter or a click jumps there. */
export function TreePanel() {
  const rows = createMemo(() => outline(store.index()));
  const onPath = createMemo(() => new Set(store.path().map((n) => n.id)));
  const leafId = () => store.conv()?.leaf_id ?? null;
  const [selected, setSelected] = createSignal(
    Math.max(
      0,
      rows().findIndex((r) => r.node.id === leafId()),
    ),
  );
  let listEl!: HTMLDivElement;
  let closeEl!: HTMLButtonElement;
  let opener: HTMLElement | null = null;

  const close = () => store.setTreeOpen(false);
  const choose = (row: OutlineRow) => {
    close();
    if (row.node.role === "user") {
      batch(() => {
        store.setEditing({ nodeId: row.node.id });
        store.setComposerText(row.node.content);
        store.setComposerAttachments(row.node.attachments);
      });
      requestAnimationFrame(() => document.querySelector<HTMLElement>(".composer-editor")?.focus());
    } else {
      batch(() => {
        store.setEditing(null);
        store.setComposerText("");
        store.setComposerAttachments([]);
      });
      void store.moveLeaf(row.node.id, false);
    }
  };

  const onKey = (e: KeyboardEvent) => {
    const n = rows().length;
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "Tab") {
      e.preventDefault();
      (document.activeElement === listEl ? closeEl : listEl).focus({ preventScroll: true });
    } else if ((e.target as HTMLElement).closest("button")) {
      return;
    } else if (n === 0) {
      return;
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const step = e.key === "ArrowDown" ? 1 : -1;
      setSelected((s) => Math.min(n - 1, Math.max(0, s + step)));
    } else if (e.key === "Enter") {
      e.preventDefault();
      choose(rows()[selected()]);
    }
  };
  onMount(() => {
    opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    listEl.focus({ preventScroll: true });
    document.addEventListener("keydown", onKey);
  });
  onCleanup(() => {
    document.removeEventListener("keydown", onKey);
    if (opener?.isConnected) opener.focus({ preventScroll: true });
  });

  createEffect(() => {
    listEl.children[selected()]?.scrollIntoView({ block: "nearest" });
  });

  return (
    <div class="overlay" onClick={(e) => e.target === e.currentTarget && close()}>
      <div class="tree-panel" role="dialog" aria-modal="true" aria-label="对话树">
        <div class="tree-head">
          <span>对话树</span>
          <small>↑↓ 选择 · Enter 跳转 · Esc 关闭</small>
          <button ref={closeEl} type="button" class="tree-close" aria-label="关闭对话树" onClick={close}>×</button>
        </div>
        <div class="tree-list" ref={listEl} role="listbox" tabIndex={0} aria-label="对话分支"
          aria-activedescendant={rows()[selected()] ? `tree-option-${rows()[selected()].node.id}` : undefined}>
          <For each={rows()} fallback={<div class="popup-empty">还没有消息</div>}>
            {(row, i) => (
              <div
                class="tree-row"
                id={`tree-option-${row.node.id}`}
                role="option"
                aria-selected={i() === selected()}
                aria-label={`${row.node.role === "user" ? "编辑你的消息" : "跳转到回复"}：${snippet(row.node.content)}`}
                classList={{
                  selected: i() === selected(),
                  current: row.node.id === leafId(),
                  "on-path": onPath().has(row.node.id),
                  user: row.node.role === "user",
                  assistant: row.node.role === "assistant",
                }}
                onMouseMove={() => setSelected(i())}
                onClick={() => choose(row)}
              >
                <span class="tree-prefix" aria-hidden="true">{row.prefix}</span>
                <span class="tree-role">{row.node.role === "user" ? "你" : "答"}</span>
                <span class="tree-text">{snippet(row.node.content)}</span>
              </div>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}
