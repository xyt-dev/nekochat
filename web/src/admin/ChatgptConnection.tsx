import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { errorMessage, type AdminApi, type ChatgptStatus } from "./api";

export function ChatgptConnection(props: { api: AdminApi; onChanged: () => Promise<void> }) {
  const [status, setStatus] = createSignal<ChatgptStatus | null>(null);
  const [selected, setSelected] = createSignal("");
  const [authorizationUrl, setAuthorizationUrl] = createSignal("");
  const [redirect, setRedirect] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const [welcome, setWelcome] = createSignal(false);
  const activeAccount = () => status()?.accounts.find((account) => account.client_id === status()?.active_client_id);
  const selectedIsActive = () => !!status()?.connected && selected() === status()?.active_client_id;
  const selectedAccount = () => status()?.accounts.find((account) => account.client_id === selected());
  let disposed = false;
  let polling = false;
  let statusVersion = 0;
  async function update(next: ChatgptStatus) {
    if (disposed) return;
    const previous = status();
    const changed = previous?.connected !== next.connected || previous?.active_client_id !== next.active_client_id;
    if (!previous || previous.active_client_id !== next.active_client_id || selected() && !next.accounts.some((account) => account.client_id === selected())) {
      setSelected(next.active_client_id ?? "");
    }
    setStatus(next); setWelcome(next.welcome_required);
    if (!next.pending) setAuthorizationUrl("");
    if (changed) await props.onChanged();
  }
  async function refresh(force = false) {
    if (disposed || !force && (polling || busy())) return;
    const version = ++statusVersion;
    polling = true;
    try { const next = await props.api.chatgptStatus(); if (!disposed && version === statusVersion) await update(next); }
    catch (failure) { if (!disposed && version === statusVersion) setError(errorMessage(failure)); }
    finally { if (version === statusVersion) polling = false; }
  }
  async function action(run: () => Promise<ChatgptStatus | void>) {
    if (busy() || disposed) return;
    statusVersion++;
    setBusy(true); setError("");
    try { const next = await run(); if (disposed) { await props.onChanged(); return; } if (next) await update(next); else await refresh(true); }
    catch (failure) { if (!disposed) setError(errorMessage(failure)); }
    finally { polling = false; setBusy(false); }
  }
  async function begin() {
    if (busy() || disposed || !status() || status()?.pending) return;
    // Open synchronously in the click handler; the fallback link remains available if blocked.
    const popup = window.open("about:blank", "_blank");
    if (popup) popup.opener = null;
    statusVersion++;
    setBusy(true); setError("");
    try {
      const authorization = await props.api.chatgptStart(selected() || null);
      if (disposed) { popup?.close(); return; }
      const url = new URL(authorization.authorization_url);
      if (url.origin !== "https://auth.openai.com" || url.pathname !== "/api/accounts/authorize") throw new Error("服务器返回了不受信任的授权地址");
      setAuthorizationUrl(url.href);
      if (popup) popup.location.replace(url.href);
      await refresh(true);
    } catch (failure) { popup?.close(); setError(errorMessage(failure)); }
    finally { polling = false; setBusy(false); }
  }
  onMount(() => {
    void refresh();
    const timer = window.setInterval(() => { if (status()?.pending || authorizationUrl()) void refresh(); }, 1500);
    onCleanup(() => { disposed = true; window.clearInterval(timer); });
  });
  return <section class="admin-card" aria-labelledby="chatgpt-connection-title" aria-busy={busy()}>
    <h3 id="chatgpt-connection-title">连接官方ChatGPT账号（订阅）</h3>
    <p class="admin-note">通过 OpenAI 官方 Sign in with ChatGPT 授权，使用账号允许的 ChatGPT 订阅或 credits；与 API key 计费分开。只有管理员可以连接，凭证仅保存在本服务器，不发送给浏览器。不自动修改现有角色或默认角色设置。</p>
    <p role="status">{status() ? status()!.connected ? "已连接 · 使用 ChatGPT 订阅" : "尚未连接可用的 ChatGPT 账号" : "正在读取连接状态…"} <a href="https://chatgpt.com/settings/usage" target="_blank" rel="noopener noreferrer">Manage usage · 管理 ChatGPT 用量</a></p>
    <p class="admin-note" aria-live="polite">当前使用账号：<strong>{status() ? status()!.connected ? activeAccount()?.label ?? status()!.active_client_id ?? "账号信息暂不可用" : "无（尚未连接）" : "正在读取…"}</strong></p>
    <label>ChatGPT 账号 / 工作区<select name="chatgpt-account" value={selected()} disabled={busy() || !status()} onChange={(event) => setSelected(event.currentTarget.value)}><option value="" selected={selected() === ""}>添加新的账号 / 工作区</option><For each={status()?.accounts ?? []}>{(account) => <option value={account.client_id} selected={selected() === account.client_id}>{account.label}{account.client_id === status()?.active_client_id && status()?.connected ? "（正在使用）" : account.connected ? "（已保存会话）" : "（需重新授权）"}</option>}</For></select></label>
    <p class="admin-note">订阅请求固定使用当前账号。选择其他已保存账号后，点击「切换至此账号」才会用于后续请求。</p>
    <div class="admin-actions"><button type="button" class="primary" disabled={busy() || !status() || !!status()?.pending} onClick={() => void begin()}>Continue with ChatGPT</button><Show when={selected() && status()?.accounts.some((account) => account.client_id === selected() && account.connected)}><button type="button" disabled={busy() || selectedIsActive()} onClick={() => void action(() => props.api.chatgptSelect(selected()))}>{selectedIsActive() ? "正在使用此账号" : "切换至此账号"}</button></Show><Show when={status()?.connected}><button type="button" disabled={busy()} onClick={() => void action(() => props.api.chatgptLogout())}>退出当前账号</button></Show><button type="button" disabled={busy()} onClick={() => void refresh()}>刷新状态</button></div>
    <Show when={selectedAccount() && !selectedAccount()!.connected}>
      <div class="admin-actions"><button type="button" class="admin-account-remove" disabled={busy() || !!status()?.pending} onClick={() => void action(() => props.api.chatgptRemove(selected()))}>移除本地记录</button></div>
      <p class="admin-note">仅移除所选已退出账号的本地记录，保留其他账号。OpenAI 侧的应用不会随之删除，请到 <a href="https://chatgpt.com/settings/usage" target="_blank" rel="noopener noreferrer">ChatGPT 设置</a>管理授权；以后重新添加此账号可能创建新的应用注册。</p>
    </Show>
    <Show when={status()?.pending || authorizationUrl()}><div class="admin-oauth-pending">
      <p>请在 OpenAI 页面完成授权。本次授权十分钟后失效；返回地址仅绑定本服务器的安全环回监听器。</p>
      <Show when={authorizationUrl()}><p><a href={authorizationUrl()} target="_blank" rel="noopener noreferrer">重新打开本次官方授权页面</a></p></Show>
      <p class="admin-note">远程服务器：浏览器的 127.0.0.1 回调可能无法打开。请复制地址栏中完整的最终回调 URL（包含 code、state、client_id），粘贴到下方；不要分享或截图此 URL。也可使用 SSH 转发本次回调端口。</p>
      <form onSubmit={(event) => { event.preventDefault(); const value = redirect(); void action(async () => { const next = await props.api.chatgptCallback(value); setRedirect(""); return next; }); }}><label>完整回调 URL<input name="chatgpt-redirect-url" type="url" autocomplete="off" spellcheck={false} value={redirect()} onInput={(event) => setRedirect(event.currentTarget.value)} required placeholder="http://127.0.0.1:…/auth/callback?…" disabled={busy()} /></label><div class="admin-actions"><button type="submit" disabled={busy() || !redirect().trim()}>完成授权</button><button type="button" disabled={busy()} onClick={() => void action(async () => { await props.api.chatgptCancel(); setRedirect(""); setAuthorizationUrl(""); })}>取消授权</button></div></form>
    </div></Show>
    <Show when={error() || status()?.error}><p class="admin-error" role="alert">{error() || status()?.error} <a href="https://chatgpt.com/settings/usage" target="_blank" rel="noopener noreferrer">管理 ChatGPT 用量和应用权限</a></p></Show>
    <p class="admin-note">订阅余额、剩余额度与金钱成本未提供时为未知，不代表 0 或免费。连接后请刷新实际账号模型目录，再上架和绑定角色。达到订阅限额时不会自动改用 API key。</p>
    <Show when={welcome()}><dialog class="admin-chatgpt-welcome" ref={(dialog) => queueMicrotask(() => { if (dialog.isConnected && !dialog.open) dialog.showModal(); })} aria-labelledby="chatgpt-welcome-title" onCancel={(event) => { event.preventDefault(); void action(async () => { await props.api.chatgptAcknowledge(); setWelcome(false); }); }}><div class="admin-card"><h3 id="chatgpt-welcome-title">You're using your ChatGPT plan</h3><p>此服务器中绑定 ChatGPT 订阅的角色会使用您账号的 ChatGPT plan 或 credits，与应用自身或 API key 的费用分开。您可以在 ChatGPT 设置中管理用量和应用权限。</p><p><a href="https://chatgpt.com/settings/usage" target="_blank" rel="noopener noreferrer">Manage usage</a></p><button type="button" class="primary" autofocus disabled={busy()} onClick={() => void action(async () => { await props.api.chatgptAcknowledge(); setWelcome(false); })}>Got it · 知道了</button></div></dialog></Show>
  </section>;
}
