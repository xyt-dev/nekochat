import { createEffect, on, onCleanup, onMount, Show } from "solid-js";
import { ConfirmDialog, Sidebar, Toast, TopBar } from "./components/Chrome";
import { LoginPage } from "./components/Account";
import { Composer } from "./components/Composer";
import { MessageList } from "./components/MessageList";
import { TreePanel } from "./components/TreePanel";
import { CharacterBackground } from "./components/CharacterBackground";
import { store } from "./store";
import { loadCustomThemes } from "./themes";

export function App(props: { offline?: boolean }) {
  onMount(() => {
    if (props.offline) return;
    const expired = () => store.resetSession();
    window.addEventListener("account-expired", expired);
    onCleanup(() => window.removeEventListener("account-expired", expired));
    void store.bootstrapSession();
  });
  createEffect(on(() => store.user()?.id, (id) => {
    if (id && !props.offline) void loadCustomThemes();
  }));
  return (
    <div class="app" classList={{ "sidebar-open": store.sidebarOpen() }}>
      <Show when={props.offline || store.sessionReady()} fallback={<div class="login" role="status">加载登录状态…</div>}>
        <Show when={props.offline || store.user()} fallback={<LoginPage />}>
          <Sidebar />
          <button type="button" class="backdrop" aria-label="关闭对话列表" tabIndex={store.sidebarOpen() ? 0 : -1} onClick={() => store.setSidebarOpen(false)} />
          <main class="main">
            <CharacterBackground />
            <TopBar />
            <MessageList />
            <Composer />
          </main>
          <Show when={store.treeOpen()}><TreePanel /></Show>
        </Show>
      </Show>
      <ConfirmDialog />
      <Toast />
    </div>
  );
}
