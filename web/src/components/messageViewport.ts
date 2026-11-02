const BOTTOM_SLACK = 32;

export interface MessageViewport {
  followLatest(): void;
  layoutChanged(): void;
  dispose(): void;
}

/** Layout coordinates never mix WebKit compositor scroll with stale rectangles. */
export function messageAboveViewport(scroll: HTMLElement, element: Element): boolean {
  let top = 0;
  let node: HTMLElement | null = element as HTMLElement;
  const height = node.offsetHeight;
  while (node && node !== scroll) {
    top += node.offsetTop;
    node = node.offsetParent as HTMLElement | null;
    if (node && node !== scroll) top += node.clientTop;
  }
  return node === scroll && top + height <= scroll.scrollTop;
}

/** User intent only. TanStack owns measurement corrections, gesture holds and writes. */
export function createMessageViewport(scroll: HTMLElement, content: HTMLElement, scrollToLatest: () => void, cancelScroll: () => void): MessageViewport {
  const view = scroll.ownerDocument.defaultView!;
  let following = true;
  let frame = 0;
  let intent = 0;
  let dragging = false;
  let pointerTop = 0;
  let pointerFollowing = false;
  let touching = false;
  let touchY: number | undefined;
  let scrolling = false;
  let inGesture = false;
  let settleTimer = 0;
  let lastTop = scroll.scrollTop;

  function maxTop(): number { return Math.max(0, scroll.scrollHeight - scroll.clientHeight); }
  function outsideEdges(): boolean { return scroll.scrollTop < -0.5 || scroll.scrollTop > maxTop() + 0.5; }
  function atBottom(): boolean { return !outsideEdges() && maxTop() - scroll.scrollTop <= BOTTOM_SLACK; }
  function cancelFrame(): void {
    if (frame) view.cancelAnimationFrame(frame);
    frame = 0;
  }
  function beginReading(): void { following = false; cancelFrame(); cancelScroll(); }
  function layoutChanged(): void {
    if (!following || inGesture || outsideEdges() || frame) return;
    frame = view.requestAnimationFrame(() => {
      frame = 0;
      if (following && !inGesture && !outsideEdges() && maxTop() - scroll.scrollTop > 0.5) scrollToLatest();
    });
  }
  function followLatest(): void {
    intent = 0;
    following = true;
    layoutChanged();
  }
  function clearSettleTimer(): void {
    if (settleTimer) view.clearTimeout(settleTimer);
    settleTimer = 0;
  }
  function settle(): void {
    if (touching || outsideEdges()) return;
    clearSettleTimer();
    inGesture = false;
    scrolling = false;
    if (!following && intent > 0 && atBottom()) following = true;
    layoutChanged();
  }
  function awaitSettle(): void {
    clearSettleTimer();
    if (touching || Reflect.has(scroll, "onscrollend")) return;
    // Old WebKit lacks scrollend. Match TanStack's quiet-window convention,
    // then confirm the physical offset stopped; this never decides intent.
    const top = scroll.scrollTop;
    settleTimer = view.setTimeout(() => {
      settleTimer = 0;
      if (Math.abs(scroll.scrollTop - top) > 0.5 || outsideEdges()) awaitSettle();
      else settle();
    }, 150);
  }
  function onScrollEnd(): void { scrolling = false; settle(); }
  function onScroll(): void {
    const top = scroll.scrollTop;
    const delta = top - lastTop;
    lastTop = top;
    if (inGesture && Math.abs(delta) > 0.5) scrolling = true;
    if (outsideEdges()) { cancelFrame(); return; }
    if (dragging && Math.abs(delta) > 0.5) intent = Math.sign(delta);
    if (intent < 0 && delta < -0.5) beginReading();
    if (inGesture) awaitSettle();
    else if (!following && intent > 0 && atBottom()) { following = true; layoutChanged(); }
  }
  function onWheel(event: WheelEvent): void {
    if (!event.deltaY) return;
    intent = Math.sign(event.deltaY);
    if (intent < 0) beginReading();
  }
  function onKey(event: KeyboardEvent): void {
    switch (event.key) {
      case "ArrowUp": case "PageUp": case "Home": intent = -1; break;
      case "ArrowDown": case "PageDown": case "End": intent = 1; break;
      case " ": intent = event.shiftKey ? -1 : 1; break;
      default: return;
    }
    if (intent < 0) beginReading();
  }
  function onTouchStart(event: TouchEvent): void {
    clearSettleTimer();
    touching = inGesture = true;
    scrolling = false;
    touchY = event.touches[0]?.clientY;
    cancelFrame();
    cancelScroll();
  }
  function onTouchMove(event: TouchEvent): void {
    const y = event.touches[0]?.clientY;
    if (y === undefined || touchY === undefined) return;
    const delta = touchY - y;
    touchY = y;
    if (!delta) return;
    intent = Math.sign(delta);
    beginReading();
  }
  function onTouchEnd(): void {
    touchY = undefined;
    touching = false;
    awaitSettle();
    if (!scrolling) settle();
  }
  function onPointerDown(event: PointerEvent): void {
    if (event.pointerType === "touch") return;
    dragging = true;
    lastTop = scroll.scrollTop;
    pointerTop = scroll.scrollTop;
    pointerFollowing = following;
    const box = scroll.getBoundingClientRect();
    const gutter = scroll.offsetWidth - scroll.clientWidth;
    if (event.target === scroll && (event.clientX >= box.right - gutter || event.clientX <= box.left + scroll.clientLeft)) beginReading();
  }
  function onPointerUp(): void {
    if (dragging && pointerFollowing && Math.abs(scroll.scrollTop - pointerTop) <= 0.5) followLatest();
    dragging = false;
  }
  function onPointerCancel(): void { dragging = false; }

  scroll.addEventListener("scroll", onScroll, { capture: true, passive: true });
  scroll.addEventListener("scrollend", onScrollEnd, { passive: true });
  scroll.addEventListener("wheel", onWheel, { passive: true });
  scroll.addEventListener("keydown", onKey);
  scroll.addEventListener("touchstart", onTouchStart, { capture: true, passive: true });
  scroll.addEventListener("touchmove", onTouchMove, { capture: true, passive: true });
  scroll.addEventListener("touchend", onTouchEnd, { capture: true, passive: true });
  scroll.addEventListener("touchcancel", onTouchEnd, { capture: true, passive: true });
  scroll.addEventListener("pointerdown", onPointerDown);
  view.addEventListener("pointerup", onPointerUp);
  view.addEventListener("pointercancel", onPointerCancel);
  const observer = new ResizeObserver(layoutChanged);
  observer.observe(content);
  observer.observe(scroll);

  return {
    followLatest,
    layoutChanged,
    dispose() {
      cancelFrame();
      clearSettleTimer();
      observer.disconnect();
      scroll.removeEventListener("scroll", onScroll, true);
      scroll.removeEventListener("scrollend", onScrollEnd);
      scroll.removeEventListener("wheel", onWheel);
      scroll.removeEventListener("keydown", onKey);
      scroll.removeEventListener("touchstart", onTouchStart, true);
      scroll.removeEventListener("touchmove", onTouchMove, true);
      scroll.removeEventListener("touchend", onTouchEnd, true);
      scroll.removeEventListener("touchcancel", onTouchEnd, true);
      scroll.removeEventListener("pointerdown", onPointerDown);
      view.removeEventListener("pointerup", onPointerUp);
      view.removeEventListener("pointercancel", onPointerCancel);
    },
  };
}
