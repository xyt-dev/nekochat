import { createEffect, createSignal, For, Show } from "solid-js";
import { ApiError, login } from "../api";
import { store } from "../store";
import { allThemes, applyTheme, BUILTIN_THEMES, CUSTOM_GROUP, customThemes, type ThemeInfo, themeId } from "../themes";
import { confirmAction, confirmRequest, toastText } from "../ui";
import { createPopover } from "./Popover";

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
      <button type="button" class="sidebar-close" onClick={() => { store.setSidebarOpen(false); document.querySelector<HTMLButtonElement>(".menu-button")?.focus(); }}>关闭对话列表</button>
      <button type="button" class="new-chat" onClick={() => void store.newConversation()}>
        ＋ 新对话
      </button>
      <nav class="conv-list">
        <For each={store.conversations()}>
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
                🗑
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
      <button type="button" class="pill theme-button" onClick={() => setOpen((o) => !o)}>
        <Show when={current()}>{(t) => <Dots theme={t()} />}</Show>
        <span>{current()?.name ?? themeId()}</span> <span class="pill-caret">▾</span>
      </button>
      <Show when={open()}>
        <div class="popup popup-down theme-popup">
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
        ☰
      </button>
      <div class="topbar-title">{store.conv()?.title ?? "nekochat"}</div>
      <ThemePicker />
    </header>
  );
}

export function ConfirmDialog() {
  return (
    <Show when={confirmRequest()}>
      {(req) => (
        <div class="overlay" onClick={(e) => e.target === e.currentTarget && req().resolve(false)}>
          <div
            class="dialog"
            role="alertdialog"
            onKeyDown={(e) => e.key === "Escape" && req().resolve(false)}
          >
            <h3>{req().title}</h3>
            <p>{req().message}</p>
            <div class="dialog-actions">
              <button type="button" class="btn" onClick={() => req().resolve(false)}>
                取消
              </button>
              <button
                type="button"
                class="btn btn-danger"
                ref={(el) => requestAnimationFrame(() => el.focus())}
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

export function LoginPage() {
  const [password, setPassword] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const submit = async (e: SubmitEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await login(password());
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "登录失败");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div class="login">
      <form class="login-card" onSubmit={submit}>
        <h2>nekochat</h2>
        <p>请输入访问口令</p>
        <input
          type="password"
          autocomplete="current-password"
          placeholder="访问口令"
          value={password()}
          onInput={(e) => setPassword(e.currentTarget.value)}
          ref={(el) => requestAnimationFrame(() => el.focus())}
        />
        <Show when={error()}>
          <div class="login-error">{error()}</div>
        </Show>
        <button type="submit" class="btn btn-primary" disabled={busy() || !password()}>
          进入
        </button>
      </form>
    </div>
  );
}
