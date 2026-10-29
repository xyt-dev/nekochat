import { createSignal, Show } from "solid-js";
import { api, ApiError } from "../api";
import { store } from "../store";
import { toast } from "../ui";
import { Avatar } from "./Files";
import { createPopover, trapDialogFocus } from "./Popover";

export function AccountMenu() {
  const [open, setOpen, root] = createPopover();
  const [editing, setEditing] = createSignal(false);
  let trigger!: HTMLButtonElement;
  return <>
    <div class="picker account-menu" ref={root}>
      <button ref={trigger} type="button" class="account-button" aria-label="用户菜单" aria-expanded={open()} aria-controls="account-options" onClick={() => setOpen((value) => !value)}>
        <Avatar name={store.user()?.username ?? ""} url={store.user()?.avatar} />
      </button>
      <Show when={open()}><div id="account-options" class="popup popup-down account-popup" role="group" aria-label="账户操作">
        <div class="popup-group-title">{store.user()?.username}</div>
        <button type="button" class="popup-item" onClick={() => { setOpen(false); setEditing(true); }}>用户信息</button>
        <button type="button" class="popup-item" onClick={() => {
          setOpen(false);
          void store.logout().catch((error) => toast(error instanceof ApiError ? error.message : "退出失败，请重试"));
        }}>退出登录</button>
      </div></Show>
    </div>
    <Show when={editing() && store.user()}><ProfileDialog close={() => { setEditing(false); if (trigger.isConnected) trigger.focus({ preventScroll: true }); }} /></Show>
  </>;
}

function ProfileDialog(props: { close: () => void }) {
  const account = store.user()!;
  const version = store.sessionVersion();
  const [username, setUsername] = createSignal(account.username);
  const [avatar, setAvatar] = createSignal(account.avatar);
  const [currentPassword, setCurrentPassword] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [uploading, setUploading] = createSignal(false);
  const [error, setError] = createSignal("");
  const submit = async (event: SubmitEvent) => {
    event.preventDefault();
    if (busy() || uploading()) return;
    if (password()) {
      const bytes = new TextEncoder().encode(password()).byteLength;
      if (bytes < 8 || bytes > 1024) { setError("密码须为 8–1024 个 UTF-8 字节"); return; }
    }
    if (document.activeElement instanceof HTMLButtonElement) {
      (event.currentTarget as HTMLFormElement).focus({ preventScroll: true });
    }
    setBusy(true); setError("");
    try {
      await store.saveProfile({ username: username().trim(), avatar: avatar(), ...(password() ? { password: password(), current_password: currentPassword() } : {}) });
      if (version !== store.sessionVersion()) return;
      props.close();
    } catch (failure) {
      if (version === store.sessionVersion()) setError(failure instanceof ApiError ? failure.message : "保存资料失败");
    } finally { setBusy(false); }
  };
  const upload = async (file: File | undefined) => {
    if (!file) return;
    if (!["image/png", "image/jpeg", "image/webp"].includes(file.type) || !file.size || file.size > 2 * 1024 * 1024) {
      setError("头像须为 PNG、JPEG 或 WebP，最大 2 MiB"); return;
    }
    setUploading(true); setError("");
    try {
      const result = await api.uploadAvatar(file);
      if (version === store.sessionVersion()) setAvatar(result.url);
    } catch (failure) {
      if (version === store.sessionVersion()) setError(failure instanceof ApiError ? failure.message : "头像上传失败");
    } finally { setUploading(false); }
  };
  return <div class="overlay" onClick={(event) => { if (event.target === event.currentTarget && !busy() && !uploading()) props.close(); }}>
    <form class="dialog profile-dialog" role="dialog" aria-modal="true" aria-labelledby="profile-title" tabIndex={-1} ref={(element) => requestAnimationFrame(() => { if (element.isConnected) element.focus({ preventScroll: true }); })} onSubmit={submit} onKeyDown={(event) => { if (event.key === "Escape" && !busy() && !uploading()) props.close(); else trapDialogFocus(event, event.currentTarget); }}>
      <h3 id="profile-title">用户信息</h3>
      <div class="profile-avatar"><Avatar name={username()} url={avatar()} /><label class="btn" role="button" tabIndex={busy() || uploading() ? -1 : 0} aria-disabled={busy() || uploading()} onKeyDown={(event) => { if ((event.key === "Enter" || event.key === " ") && !busy() && !uploading()) { event.preventDefault(); event.currentTarget.querySelector<HTMLInputElement>("input")?.click(); } }}>上传头像<input type="file" hidden accept="image/png,image/jpeg,image/webp" disabled={uploading() || busy()} onChange={(event) => { void upload(event.currentTarget.files?.[0]); event.currentTarget.value = ""; }} /></label>
        <button type="button" class="btn" disabled={busy() || uploading() || !avatar()} onClick={() => setAvatar(null)}>移除头像</button>
      </div>
      <small>PNG / JPEG / WebP，最大 2 MiB{uploading() ? " · 上传中…" : ""}</small>
      <label>用户名<input name="username" autocomplete="username" required readOnly={busy()} value={username()} onInput={(event) => setUsername(event.currentTarget.value)} /></label>
      <label>新密码<input name="new-password" type="password" autocomplete="new-password" placeholder="留空则不修改；8–1024 UTF-8 字节" readOnly={busy()} value={password()} onInput={(event) => setPassword(event.currentTarget.value)} /></label>
      <Show when={password()}><label>当前密码<input name="current-password" type="password" autocomplete="current-password" required readOnly={busy()} value={currentPassword()} onInput={(event) => setCurrentPassword(event.currentTarget.value)} /></label></Show>
      <Show when={error()}><div class="login-error" role="alert">{error()}</div></Show>
      <div class="dialog-actions"><button type="button" class="btn" disabled={busy() || uploading()} onClick={props.close}>取消</button><button type="submit" class="btn btn-primary" disabled={busy() || uploading() || !username().trim()}>{busy() ? "保存中…" : "保存"}</button></div>
    </form>
  </div>;
}

export function LoginPage() {
  const [mode, setMode] = createSignal<"login" | "register">("login");
  const [username, setUsername] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [confirmation, setConfirmation] = createSignal("");
  const [error, setError] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const submit = async (event: SubmitEvent) => {
    event.preventDefault();
    if (busy()) return;
    if (mode() === "register" && password() !== confirmation()) { setError("两次输入的密码不一致"); return; }
    if (mode() === "register") {
      const bytes = new TextEncoder().encode(password()).byteLength;
      if (bytes < 8 || bytes > 1024) { setError("密码须为 8–1024 个 UTF-8 字节"); return; }
    }
    setBusy(true); setError("");
    try {
      await store.authenticate(mode(), username().trim(), password());
      setPassword(""); setConfirmation("");
    } catch (failure) { setError(failure instanceof ApiError ? failure.message : "登录失败"); }
    finally { setBusy(false); }
  };
  return <div class="login"><form class="login-card" onSubmit={submit}>
    <div class="login-heading"><span class="login-kicker">NEKOCHAT</span><h2>{mode() === "login" ? "欢迎回来" : "创建账号"}</h2><p>{mode() === "login" ? "登录后，继续你的对话。" : "角色、对话和资料，保存在你的账号中。"}</p></div>
    <label>用户名<input name="username" autocomplete="username" required autofocus disabled={busy()} value={username()} onInput={(event) => setUsername(event.currentTarget.value)} /></label>
    <label>密码<input name="password" type="password" autocomplete={mode() === "login" ? "current-password" : "new-password"} required disabled={busy()} value={password()} onInput={(event) => setPassword(event.currentTarget.value)} /></label>
    <Show when={mode() === "register"}><label>确认密码<input name="confirm-password" type="password" autocomplete="new-password" required disabled={busy()} value={confirmation()} onInput={(event) => setConfirmation(event.currentTarget.value)} /></label></Show>
    <Show when={mode() === "register"}><small>密码须为 8–1024 个 UTF-8 字节（英文字母或数字每个为 1 字节）。</small></Show>
    <Show when={error()}><div class="login-error" role="alert">{error()}</div></Show>
    <button type="submit" class="btn btn-primary" disabled={busy() || !username().trim() || !password()}>{busy() ? "请稍候…" : mode() === "login" ? "登录" : "注册"}</button>
    <button type="button" class="btn" disabled={busy()} onClick={() => { setMode((value) => value === "login" ? "register" : "login"); setError(""); setPassword(""); setConfirmation(""); }}>{mode() === "login" ? "没有账号？注册" : "已有账号？登录"}</button>
  </form></div>;
}
