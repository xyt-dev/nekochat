import { createVirtualizer } from "@tanstack/solid-virtual";
import { createEffect, createMemo, For, Match, on, onCleanup, onMount, Show, Switch } from "solid-js";
import type { NodeView } from "../api";
import { copyText } from "../clipboard";
import { toast } from "../ui";
import { type LiveTurn, store } from "../store";
import { AssistantMessage, LiveMessage, UserMessage } from "./Message";

type Item = { key: string; node: NodeView; turn?: undefined } | { key: string; node?: undefined; turn: LiveTurn };

/** Distance from the bottom (px) still counted as "at the bottom". */
const BOTTOM_SLACK = 32;
/** A scroll this soon (ms) after wheel/touch/keys/scrollbar input is the user's. */
const USER_SCROLL_WINDOW = 400;

export function MessageList() {
  let scrollEl!: HTMLDivElement;
  let sizerEl!: HTMLDivElement;

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
  });

  // Follow the bottom while content grows, until the user scrolls up.
  let follow = true;
  let lastUserInput = -Infinity;
  let followFrame = 0;
  const cancelFollow = () => { if (followFrame) cancelAnimationFrame(followFrame); followFrame = 0; };
  const userInput = (event: WheelEvent | TouchEvent | PointerEvent | KeyboardEvent) => {
    lastUserInput = performance.now();
    const leavingBottom = event.type === "touchmove"
      || (event instanceof WheelEvent && event.deltaY < 0)
      || (event instanceof KeyboardEvent && (["ArrowUp", "PageUp", "Home"].includes(event.key) || (event.key === " " && event.shiftKey)));
    if (leavingBottom) follow = false;
    else if (scrollEl.scrollHeight - scrollEl.scrollTop - scrollEl.clientHeight <= BOTTOM_SLACK) follow = true;
    cancelFollow();
  };
  const toBottom = () => {
    if (!follow || followFrame) return;
    followFrame = requestAnimationFrame(() => {
      followFrame = 0;
      if (follow) scrollEl.scrollTop = scrollEl.scrollHeight;
    });
  };
  const onScroll = () => {
    const distance = scrollEl.scrollHeight - scrollEl.scrollTop - scrollEl.clientHeight;
    if (distance <= BOTTOM_SLACK) follow = true;
    else if (performance.now() - lastUserInput < USER_SCROLL_WINDOW) follow = false;
  };

  onMount(() => {
    const ro = new ResizeObserver(() => {
      if (follow) toBottom();
    });
    ro.observe(sizerEl);
    onCleanup(() => { ro.disconnect(); cancelFollow(); });
  });

  // Only opening another conversation or explicitly starting a new turn resets reading position.
  const conversationId = createMemo(() => store.conv()?.id);
  const startFollowing = () => { lastUserInput = -Infinity; follow = true; toBottom(); };
  createEffect(on(conversationId, startFollowing));
  createEffect(on(store.currentLive, (turn) => {
    if (turn && !turn.started() && !turn.assistantId()) startFollowing();
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

  return (
    <div
      class="messages"
      ref={scrollEl}
      onScroll={onScroll}
      onWheel={userInput}
      onTouchMove={userInput}
      onTouchEnd={userInput}
      onPointerDown={userInput}
      onKeyDown={userInput}
      onClick={onClick}
    >
      <Show when={items().length === 0}>
        <div class="empty-hint" role="status">{store.loadingConv() ? "正在加载对话…" : "有什么想聊的？"}</div>
      </Show>
      <div class="messages-sizer" ref={sizerEl} style={{ height: `${virtualizer.getTotalSize()}px` }}>
        <div class="messages-window" style={{ transform: `translateY(${virtualizer.getVirtualItems()[0]?.start ?? 0}px)` }}>
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
        </div>
      </div>
    </div>
  );
}
