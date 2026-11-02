import { createVirtualizer } from "@tanstack/solid-virtual";
import { createEffect, createMemo, For, Match, on, onCleanup, onMount, Show, Switch } from "solid-js";
import type { NodeView } from "../api";
import { copyText } from "../clipboard";
import { toast } from "../ui";
import { type LiveTurn, store } from "../store";
import { AssistantMessage, LiveMessage, UserMessage } from "./Message";
import { createMessageViewport, messageAboveViewport, type MessageViewport } from "./messageViewport";

type Item = { key: string; node: NodeView; turn?: undefined } | { key: string; node?: undefined; turn: LiveTurn };

export function MessageList() {
  let scrollEl!: HTMLDivElement;
  let sizerEl!: HTMLDivElement;
  let viewport: MessageViewport | undefined;

  const items = createMemo<Item[]>(() => {
    const list: Item[] = store.path().map((node) => ({ key: node.id, node }));
    const turn = store.currentLive();
    if (turn && (turn.started() || turn.error() !== null)) {
      list.push({ key: turn.assistantId() || "live", turn });
    }
    return list;
  });
  const itemByKey = createMemo(() => new Map(items().map((it) => [it.key, it])));

  const virtualizer = createVirtualizer({
    get count() {
      return items().length;
    },
    get getItemKey() {
      const list = items();
      return (i: number) => list[i]?.key ?? i;
    },
    getScrollElement: () => scrollEl,
    estimateSize: () => 140,
    overscan: 4,
    paddingStart: 16,
    paddingEnd: 24,
    // TanStack owns physical writes, iOS gesture holds and range coordinates.
    // The page controller supplies follow commands, never its own corrections.
  });

  virtualizer.shouldAdjustScrollPositionOnItemSizeChange = (item, _delta, instance) => {
    // A cold tall reply can span the fold even if its estimate ends above it.
    if (!instance.itemSizeCache.has(item.key)) {
      const element = instance.elementsCache.get(item.key);
      return element ? messageAboveViewport(scrollEl, element) : false;
    }
    return item.end <= (instance.scrollOffset ?? 0);
  };
  onMount(() => {
    const active = createMessageViewport(scrollEl, sizerEl, () => virtualizer.scrollToEnd(), () => virtualizer.cancelScroll());
    viewport = active;
    active.followLatest();
    onCleanup(() => { viewport = undefined; active.dispose(); });
  });

  const conversationId = createMemo(() => store.conv()?.id);
  createEffect(on(conversationId, () => viewport?.followLatest()));
  createEffect(on(store.currentLive, (turn) => {
    if (turn && !turn.started() && !turn.assistantId()) viewport?.followLatest();
  }));

  // Code block "复制" buttons come from worker HTML, so they are handled by delegation.
  const onClick = async (e: MouseEvent) => {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>(".nk-copy");
    if (!btn || btn.disabled || btn.getAttribute("aria-busy") === "true") return;
    const code = btn.closest(".nk-code")?.querySelector(":scope > pre > code");
    if (!code) { toast("未找到要复制的代码"); return; }
    btn.setAttribute("aria-busy", "true");
    try {
      await copyText(code.textContent ?? "");
      btn.textContent = "已复制";
      toast("代码已复制");
    } catch {
      btn.textContent = "复制失败";
      toast("复制失败，请选中代码后手动复制");
    } finally {
      btn.removeAttribute("aria-busy");
      window.setTimeout(() => { if (btn.isConnected) btn.textContent = "复制"; }, 1500);
    }
  };

  const keys = () => virtualizer.getVirtualItems().map((v) => v.key as string);
  const firstStart = () => virtualizer.getVirtualItems()[0]?.start ?? 0;

  return (
    <div
      class="messages"
      ref={scrollEl}
      onClick={onClick}
    >
      <Show when={items().length === 0}>
        <div class="empty-hint" role="status">{store.loadingConv() ? "正在加载对话…" : "有什么想聊的？"}</div>
      </Show>
      <div class="messages-sizer" ref={sizerEl}>
        <div class="messages-spacer" aria-hidden="true" style={{ height: `${Math.max(0, firstStart())}px`, "margin-top": `${Math.min(0, firstStart())}px` }} />
        <For each={keys()}>
          {(key) => {
            const vi = createMemo(() => virtualizer.getVirtualItems().find((v) => v.key === key));
            const item = () => itemByKey().get(key);
            let rowEl!: HTMLDivElement;
            onMount(() => virtualizer.measureElement(rowEl));
            return (
              <div
                class="row"
                ref={rowEl}
                data-index={vi()?.index}
              >
                <Switch>
                  <Match when={item()?.node}>
                    {(node) => (
                      <Show when={node().role === "user"} fallback={<AssistantMessage node={node()} />}>
                        <UserMessage node={node()} />
                      </Show>
                    )}
                  </Match>
                  <Match when={item()?.turn}>{(turn) => <LiveMessage turn={turn()} />}</Match>
                </Switch>
              </div>
            );
          }}
        </For>
        <div class="messages-spacer" aria-hidden="true" style={{ height: `${Math.max(0, virtualizer.getTotalSize() - (virtualizer.getVirtualItems().at(-1)?.end ?? 0))}px` }} />
      </div>
    </div>
  );
}
