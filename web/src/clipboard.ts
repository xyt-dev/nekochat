/** Copy real plain text, rejecting when the browser cannot write the clipboard. */
export async function copyText(text: string): Promise<void> {
  if (typeof navigator.clipboard?.writeText === "function") {
    await navigator.clipboard.writeText(text);
    return;
  }
  if (typeof document.execCommand !== "function") throw new Error("当前浏览器不支持复制");

  const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const selection = window.getSelection();
  const ranges = selection ? Array.from({ length: selection.rangeCount }, (_, index) => selection.getRangeAt(index).cloneRange()) : [];
  const anchor = selection?.anchorNode ?? null;
  const anchorOffset = selection?.anchorOffset ?? 0;
  const focus = selection?.focusNode ?? null;
  const focusOffset = selection?.focusOffset ?? 0;
  const input = active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement ? active : null;
  const inputSelection = input && input.selectionStart !== null
    ? { start: input.selectionStart, end: input.selectionEnd!, direction: input.selectionDirection ?? "none" }
    : null;
  const scrolls = new Map<Element, { left: number; top: number }>();
  for (const node of [active, anchor instanceof Element ? anchor : anchor?.parentElement]) {
    for (let element = node; element; element = element.parentElement) {
      if (!scrolls.has(element)) scrolls.set(element, { left: element.scrollLeft, top: element.scrollTop });
    }
  }
  const scrollX = window.scrollX;
  const scrollY = window.scrollY;
  const field = document.createElement("textarea");
  field.value = text;
  field.readOnly = true;
  field.tabIndex = -1;
  field.setAttribute("aria-hidden", "true");
  field.style.cssText = "position:fixed;top:0;left:0;width:1px;height:1px;padding:0;border:0;opacity:0;font-size:16px;pointer-events:none";
  document.body.append(field);
  try {
    field.focus({ preventScroll: true });
    field.select();
    field.setSelectionRange(0, text.length);
    if (!document.execCommand("copy")) throw new Error("浏览器未允许复制");
  } finally {
    field.value = "";
    field.remove();
    if (selection) {
      selection.removeAllRanges();
      if (ranges.length === 1 && anchor?.isConnected && focus?.isConnected) {
        selection.setBaseAndExtent(anchor, anchorOffset, focus, focusOffset);
      } else {
        for (const range of ranges) {
          if (range.commonAncestorContainer.isConnected) selection.addRange(range);
        }
      }
    }
    // Restoring an editable selection can move focus; restore the trigger last.
    if (active?.isConnected) {
      active.focus({ preventScroll: true });
      if (active === document.body && document.activeElement instanceof HTMLElement && document.activeElement !== active) document.activeElement.blur();
    }
    if (inputSelection && input?.isConnected) input.setSelectionRange(inputSelection.start, inputSelection.end, inputSelection.direction);
    for (const [element, scroll] of scrolls) {
      element.scrollLeft = scroll.left;
      element.scrollTop = scroll.top;
    }
    if (window.scrollX !== scrollX || window.scrollY !== scrollY) window.scrollTo(scrollX, scrollY);
  }
}
