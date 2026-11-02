const BOTTOM_SLACK = 32;

export interface MessageViewport {
  followLatest(): void;
  layoutChanged(): void;
  dispose(): void;
}

/** Owns scroll writes; the virtualizer supplies ranges, never estimated-height corrections. */
export function createMessageViewport(scroll: HTMLElement, content: HTMLElement, nativeAnchoring = true): MessageViewport {
  const view = scroll.ownerDocument.defaultView!;
  let following = true;
  let frame = 0;
  let intent = 0;
  let dragging = false;
  let pointerTop = 0;
  let pointerFollowing = false;
  let touchY: number | undefined;
  let lastTop = scroll.scrollTop;
  let ownTop: number | undefined;
  let anchor: { element: Element; contentTop: number } | undefined;

  function captureAnchor(): void {
    if (nativeAnchoring) return;
    const box = scroll.getBoundingClientRect();
    const hit = scroll.ownerDocument.elementFromPoint(box.left + scroll.clientWidth / 2, box.top + scroll.clientTop + Math.min(16, scroll.clientHeight / 2));
    const element = hit?.closest(".md-block, .bubble, .reasoning, .row");
    anchor = element && content.contains(element)
      ? { element, contentTop: element.getBoundingClientRect().top - box.top - scroll.clientTop + scroll.scrollTop }
      : undefined;
  }

  function cancelFrame(): void {
    if (frame) view.cancelAnimationFrame(frame);
    frame = 0;
  }
  function beginReading(): void {
    following = false;
    cancelFrame();
    captureAnchor();
  }
  function writeTop(top: number): void {
    if (Math.abs(top - scroll.scrollTop) <= 0.5) return;
    scroll.scrollTop = top;
    lastTop = ownTop = scroll.scrollTop;
  }
  function layoutChanged(): void {
    if (!following && !nativeAnchoring) {
      if (anchor?.element.isConnected && content.contains(anchor.element)) {
        const top = anchor.element.getBoundingClientRect().top - scroll.getBoundingClientRect().top - scroll.clientTop + scroll.scrollTop;
        const delta = top - anchor.contentTop;
        anchor.contentTop = top;
        if (Math.abs(delta) > 0.5) writeTop(scroll.scrollTop + delta);
      } else captureAnchor();
      return;
    }
    if (!following || frame) return;
    frame = view.requestAnimationFrame(() => {
      frame = 0;
      if (following) writeTop(Math.max(0, scroll.scrollHeight - scroll.clientHeight));
    });
  }
  function followLatest(): void {
    intent = 0;
    following = true;
    anchor = undefined;
    layoutChanged();
  }
  function onScroll(): void {
    const top = scroll.scrollTop;
    const delta = top - lastTop;
    lastTop = top;
    if (ownTop !== undefined && Math.abs(top - ownTop) <= 0.5) { ownTop = undefined; return; }
    ownTop = undefined;
    if (dragging && Math.abs(delta) > 0.5) intent = Math.sign(delta);
    const atBottom = scroll.scrollHeight - top - scroll.clientHeight <= BOTTOM_SLACK;
    if (intent < 0 && delta < -0.5 && !atBottom) following = false;
    if (intent > 0 && atBottom) followLatest();
    else if (!following) captureAnchor();
  }
  function onWheel(event: WheelEvent): void {
    if (!event.deltaY) return;
    intent = Math.sign(event.deltaY);
    if (intent < 0) beginReading();
    else if (scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight <= BOTTOM_SLACK) followLatest();
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
    touchY = event.touches[0]?.clientY;
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
    if (intent > 0 && scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight <= BOTTOM_SLACK) followLatest();
  }
  function onPointerDown(event: PointerEvent): void {
    dragging = true;
    lastTop = scroll.scrollTop;
    pointerTop = scroll.scrollTop;
    pointerFollowing = following;
    // Cancel pending following before a scrollbar drag can generate its first scroll event.
    const box = scroll.getBoundingClientRect();
    const gutter = scroll.offsetWidth - scroll.clientWidth;
    if (event.target === scroll && (event.clientX >= box.right - gutter || event.clientX <= box.left + scroll.clientLeft)) beginReading();
  }
  function onPointerUp(): void {
    if (dragging && pointerFollowing && Math.abs(scroll.scrollTop - pointerTop) <= 0.5) followLatest();
    dragging = false;
  }

  // User intent is processed before the passive virtualizer handles the same scroll event.
  scroll.addEventListener("scroll", onScroll, { capture: true, passive: true });
  scroll.addEventListener("wheel", onWheel, { passive: true });
  scroll.addEventListener("keydown", onKey);
  scroll.addEventListener("touchstart", onTouchStart, { passive: true });
  scroll.addEventListener("touchmove", onTouchMove, { passive: true });
  scroll.addEventListener("touchend", onTouchEnd, { passive: true });
  scroll.addEventListener("pointerdown", onPointerDown);
  view.addEventListener("pointerup", onPointerUp);
  const observer = new ResizeObserver(layoutChanged);
  observer.observe(content);
  observer.observe(scroll);

  return {
    followLatest,
    layoutChanged,
    dispose() {
      cancelFrame();
      observer.disconnect();
      scroll.removeEventListener("scroll", onScroll, true);
      scroll.removeEventListener("wheel", onWheel);
      scroll.removeEventListener("keydown", onKey);
      scroll.removeEventListener("touchstart", onTouchStart);
      scroll.removeEventListener("touchmove", onTouchMove);
      scroll.removeEventListener("touchend", onTouchEnd);
      scroll.removeEventListener("pointerdown", onPointerDown);
      view.removeEventListener("pointerup", onPointerUp);
      anchor = undefined;
    },
  };
}
