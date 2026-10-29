import { type Accessor, createSignal, onCleanup, type Setter } from "solid-js";

/** Open state for a popup that closes on outside clicks and Escape. */
export function createPopover(): [Accessor<boolean>, Setter<boolean>, (el: HTMLElement) => void] {
  const [open, setOpen] = createSignal(false);
  let root: HTMLElement | undefined;
  const onPointer = (e: PointerEvent) => {
    if (open() && root && !root.contains(e.target as Node)) setOpen(false);
  };
  const onKey = (e: KeyboardEvent) => {
    if (open() && e.key === "Escape") {
      const trigger = root?.querySelector<HTMLButtonElement>("button");
      setOpen(false);
      trigger?.focus({ preventScroll: true });
    }
  };
  document.addEventListener("pointerdown", onPointer);
  document.addEventListener("keydown", onKey);
  onCleanup(() => {
    document.removeEventListener("pointerdown", onPointer);
    document.removeEventListener("keydown", onKey);
  });
  return [open, setOpen, (el) => (root = el)];
}

/** Keep keyboard navigation inside an open modal without changing native field focus. */
export function trapDialogFocus(event: KeyboardEvent, root: HTMLElement): void {
  if (event.key !== "Tab") return;
  const controls = [...root.querySelectorAll<HTMLElement>(
    'button:not(:disabled), a[href], input:not(:disabled):not([type="hidden"]), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])',
  )].filter((element) => element.tabIndex >= 0 && element.getClientRects().length > 0);
  const first = controls[0];
  const last = controls[controls.length - 1];
  if (!first) {
    event.preventDefault();
    root.focus({ preventScroll: true });
  } else if (event.shiftKey && (document.activeElement === root || document.activeElement === first || !root.contains(document.activeElement))) {
    event.preventDefault();
    last.focus({ preventScroll: true });
  } else if (!event.shiftKey && (document.activeElement === last || !root.contains(document.activeElement))) {
    event.preventDefault();
    first.focus({ preventScroll: true });
  }
}
