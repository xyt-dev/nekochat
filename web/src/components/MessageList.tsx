import { createVirtualizer } from "@tanstack/solid-virtual";
import { createEffect, createMemo, For, Match, on, onCleanup, onMount, Show, Switch } from "solid-js";
import type { NodeView } from "../api";
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
  let lastUserInput = 0;
  const userInput = () => (lastUserInput = performance.now());
  const toBottom = () => {
    scrollEl.scrollTop = scrollEl.scrollHeight;
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
    onCleanup(() => ro.disconnect());
  });

  // A newly opened conversation and a newly sent message start at the bottom.
  createEffect(
    on(
      () => [store.conv()?.id, store.currentLive()] as const,
      () => {
        follow = true;
        requestAnimationFrame(toBottom);
      },
    ),
  );

  // Code block "复制" buttons come from worker HTML, so they are handled by delegation.
  const onClick = (e: MouseEvent) => {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>(".nk-copy");
    if (!btn) return;
    const code = btn.parentElement?.querySelector("code")?.textContent ?? "";
    void navigator.clipboard.writeText(code).then(() => {
      btn.textContent = "已复制";
      setTimeout(() => (btn.textContent = "复制"), 1500);
    });
  };

  const keys = () => virtualizer.getVirtualItems().map((v) => v.key as string);

  return (
    <div
      class="messages"
      ref={scrollEl}
      onScroll={onScroll}
      onWheel={userInput}
      onTouchMove={userInput}
      onPointerDown={userInput}
      onKeyDown={userInput}
      onClick={onClick}
    >
      <Show when={items().length === 0}>
        <div class="empty-hint">{store.loadingConv() ? "" : "有什么想聊的？"}</div>
      </Show>
      <div class="messages-sizer" ref={sizerEl} style={{ height: `${virtualizer.getTotalSize()}px` }}>
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
                style={{ transform: `translateY(${vi()?.start ?? 0}px)` }}
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
  );
}
