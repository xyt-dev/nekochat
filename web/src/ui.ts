// Small app-wide UI state: toasts and the confirmation dialog.
import { createSignal } from "solid-js";

const [toastText, setToastText] = createSignal<string | null>(null);
export { toastText };
let toastTimer: number | undefined;

export function toast(text: string): void {
  setToastText(text);
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => setToastText(null), 4000);
}

export interface ConfirmRequest {
  title: string;
  message: string;
  confirmLabel: string;
  resolve: (ok: boolean) => void;
}

const [confirmRequest, setConfirmRequest] = createSignal<ConfirmRequest | null>(null);
export { confirmRequest };
export function resetAccountUi(): void {
  confirmRequest()?.resolve(false);
  clearTimeout(toastTimer);
  setToastText(null);
}

/** Opens the confirmation dialog; resolves `true` when the user confirms. */
export function confirmAction(title: string, message: string, confirmLabel: string): Promise<boolean> {
  confirmRequest()?.resolve(false);
  const { promise, resolve } = Promise.withResolvers<boolean>();
  setConfirmRequest({
    title,
    message,
    confirmLabel,
    resolve: (ok) => {
      setConfirmRequest(null);
      resolve(ok);
    },
  });
  return promise;
}
