import { createEffect, For, Show } from "solid-js";
import { store } from "../store";
import { allThemes, applyTheme, BUILTIN_THEMES, CUSTOM_GROUP, customThemes, type ThemeInfo, themeId } from "../themes";
import { confirmAction, confirmRequest, toastText } from "../ui";
import { createPopover, trapDialogFocus } from "./Popover";
import { AccountMenu } from "./Account";
import { CharacterPicker } from "./Pickers";

const DELETE_WARNING = "聊天记录、上传的图片和文件会从服务器上一起删掉，无法恢复。";

export function Sidebar() {
  createEffect(() => {
    if (store.sidebarOpen()) requestAnimationFrame(() => document.querySelector<HTMLButtonElement>("#conversation-sidebar button")?.focus());
  });
  const remove = async (id: string, title: string) => {
    if (await confirmAction(`彻底删除「${title}」？`, DELETE_WARNING, "彻底删除")) {
      await store.deleteConversation(id);
    }
  };
  return (
    <aside id="conversation-sidebar" class="sidebar" aria-label="对话列表" onKeyDown={(event) => {
      if (!store.sidebarOpen() || !matchMedia("(max-width: 760px)").matches) return;
      if (event.key === "Escape") {
        store.setSidebarOpen(false);
        document.querySelector<HTMLButtonElement>(".menu-button")?.focus();
      } else if (event.key === "Tab") {
        const controls = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
        const first = controls[0], last = controls[controls.length - 1];
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
      }
    }}>
      <button type="button" class="sidebar-close" onClick={() => { store.setSidebarOpen(false); document.querySelector<HTMLButtonElement>(".menu-button")?.focus(); }}><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg>关闭对话列表</button>
      <CharacterPicker />
      <button type="button" class="new-chat" onClick={() => void store.newConversation()}>
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg>新对话
      </button>
      <div class="sidebar-section-title"><span>最近对话</span><span>{store.conversations().length}</span></div>
      <nav class="conv-list" aria-label="历史对话">
        <For each={store.conversations()} fallback={<div class="sidebar-empty">还没有对话<small>从上方开始一段新的聊天。</small></div>}>
          {(c) => (
            <div
              class="conv-item"
              classList={{ active: store.conv()?.id === c.id, generating: !!store.live()[c.id] }}
            >
              <button type="button" class="conv-title" aria-current={store.conv()?.id === c.id ? "page" : undefined} onClick={() => void store.openConversation(c.id)}>{c.title}</button>
              <button
                type="button"
                class="conv-delete"
                title="彻底删除"
                aria-label={`彻底删除 ${c.title}`}
                onClick={() => void remove(c.id, c.title)}
              >
                <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 6h18M9 6V4h6v2M5 6l1 14h12l1-14M10 10v6M14 10v6" /></svg>
              </button>
            </div>
          )}
        </For>
      </nav>
    </aside>
  );
}

function Dots(props: { theme: ThemeInfo }) {
  return (
    <span class="dots">
      <For each={props.theme.dots}>{(color) => <i style={{ background: color }} />}</For>
    </span>
  );
}

function ThemePicker() {
  const [open, setOpen, root] = createPopover();
  const groups = () => {
    const names = [...new Set(BUILTIN_THEMES.map((t) => t.group))];
    const list = names.map((g) => [g, BUILTIN_THEMES.filter((t) => t.group === g)] as const);
    return customThemes().length > 0 ? [...list, [CUSTOM_GROUP, customThemes()] as const] : list;
  };
  const current = () => allThemes().find((t) => t.id === themeId());
  return (
    <div class="picker" ref={root}>
      <button type="button" class="pill theme-button" aria-label={`切换主题，当前${current()?.group ?? ""} ${current()?.name ?? themeId()}`} aria-expanded={open()} aria-controls="theme-options" onClick={() => setOpen((o) => !o)}>
        <Show when={current()}>{(t) => <Dots theme={t()} />}</Show>
        <span>{current()?.name ?? themeId()}</span> <span class="pill-caret">▾</span>
      </button>
      <Show when={open()}>
        <div id="theme-options" class="popup popup-down theme-popup" role="group" aria-label="主题列表">
          <For each={groups()}>
            {([group, themes]) => (
              <div class="popup-group">
                <div class="popup-group-title">{group}</div>
                <For each={themes}>
                  {(t) => (
                    <button
                      type="button"
                      class="popup-item"
                      classList={{ selected: t.id === themeId() }}
                      onClick={() => {
                        applyTheme(t);
                        setOpen(false);
                      }}
                    >
                      <span>
                        <Dots theme={t} /> {t.name}
                      </span>
                    </button>
                  )}
                </For>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

export function TopBar() {
  return (
    <header class="topbar">
      <button type="button" class="menu-button" title="对话列表" aria-label="打开对话列表" aria-expanded={store.sidebarOpen()} aria-controls="conversation-sidebar" onClick={() => store.setSidebarOpen((o) => !o)}>
        <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="M4 6h16M4 12h16M4 18h16" /></svg>
      </button>
      <div class="topbar-title">{store.conv()?.title ?? "nekochat"}</div>
      <ThemePicker />
      <AccountMenu />
    </header>
  );
}

export function ConfirmDialog() {
  let returnFocus: HTMLElement | null = null;
  createEffect(() => {
    if (confirmRequest()) returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    else if (returnFocus) {
      const element = returnFocus;
      returnFocus = null;
      requestAnimationFrame(() => { if (element.isConnected) element.focus({ preventScroll: true }); });
    }
  });
  return (
    <Show when={confirmRequest()}>
      {(req) => (
        <div class="overlay" onClick={(e) => e.target === e.currentTarget && req().resolve(false)}>
          <div
            class="dialog"
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="confirmation-title"
            aria-describedby="confirmation-message"
            onKeyDown={(event) => { if (event.key === "Escape") req().resolve(false); else trapDialogFocus(event, event.currentTarget); }}
          >
            <h3 id="confirmation-title">{req().title}</h3>
            <p id="confirmation-message">{req().message}</p>
            <div class="dialog-actions">
              <button type="button" class="btn" ref={(element) => requestAnimationFrame(() => { if (element.isConnected) element.focus({ preventScroll: true }); })} onClick={() => req().resolve(false)}>
                取消
              </button>
              <button
                type="button"
                class="btn btn-danger"
                onClick={() => req().resolve(true)}
              >
                {req().confirmLabel}
              </button>
            </div>
          </div>
        </div>
      )}
    </Show>
  );
}

export function Toast() {
  return (
    <Show when={toastText()}>
      <div class="toast" role="status">
        {toastText()}
      </div>
    </Show>
  );
}

