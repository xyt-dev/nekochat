import { onMount, Show } from "solid-js";
import { loginNeeded } from "./api";
import { ConfirmDialog, LoginPage, Sidebar, Toast, TopBar } from "./components/Chrome";
import { Composer } from "./components/Composer";
import { MessageList } from "./components/MessageList";
import { TreePanel } from "./components/TreePanel";
import { store } from "./store";
import { loadCustomThemes } from "./themes";

function conversationFromHash(): string | null {
  const m = location.hash.match(/^#\/c\/(.+)$/);
  return m ? decodeURIComponent(m[1]) : null;
}

export function App(props: { offline?: boolean }) {
  onMount(() => {
    if (props.offline) return;
    void store.loadModels();
    void store.loadConversations();
    void loadCustomThemes();
    const id = conversationFromHash();
    if (id) void store.openConversation(id);
  });
  return (
    <div class="app" classList={{ "sidebar-open": store.sidebarOpen() }}>
      <Sidebar />
      <div class="backdrop" onClick={() => store.setSidebarOpen(false)} />
      <main class="main">
        <TopBar />
        <MessageList />
        <Composer />
      </main>
      <Show when={store.treeOpen()}>
        <TreePanel />
      </Show>
      <ConfirmDialog />
      <Toast />
      <Show when={loginNeeded()}>
        <LoginPage />
      </Show>
    </div>
  );
}
