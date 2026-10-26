import { type Accessor, createSignal, onCleanup, type Setter } from "solid-js";

/** Open state for a popup that closes on outside clicks and Escape. */
export function createPopover(): [Accessor<boolean>, Setter<boolean>, (el: HTMLElement) => void] {
  const [open, setOpen] = createSignal(false);
  let root: HTMLElement | undefined;
  const onPointer = (e: PointerEvent) => {
    if (open() && root && !root.contains(e.target as Node)) setOpen(false);
  };
  const onKey = (e: KeyboardEvent) => {
    if (open() && e.key === "Escape") setOpen(false);
  };
  document.addEventListener("pointerdown", onPointer);
  document.addEventListener("keydown", onKey);
  onCleanup(() => {
    document.removeEventListener("pointerdown", onPointer);
    document.removeEventListener("keydown", onKey);
  });
  return [open, setOpen, (el) => (root = el)];
}
