import { createSignal, Show } from "solid-js";
import { localImageUrl } from "../localImages";
import { errorMessage, type AdminApi } from "./api";

export function AvatarInput(props: {
  api: AdminApi; name: string; value: string | null; disabled?: boolean;
  onChange: (value: string | null) => void; onUploading: (busy: boolean) => void;
}) {
  const [uploading, setUploading] = createSignal(false);
  const [error, setError] = createSignal("");
  async function upload(file: File) {
    setError(""); setUploading(true); props.onUploading(true);
    try {
      if (!["image/png", "image/jpeg", "image/webp"].includes(file.type)) throw new Error("头像仅支持 PNG、JPEG、WebP");
      if (!file.size || file.size > 2 * 1024 * 1024) throw new Error("头像不能为空且不能超过 2 MiB");
      const result = await props.api.uploadAvatar(file);
      const url = localImageUrl(result.url);
      if (!url?.startsWith("/api/avatars/")) throw new Error("服务器返回了无效的本站头像地址");
      props.onChange(url);
    } catch (error) { setError(errorMessage(error)); }
    finally { setUploading(false); props.onUploading(false); }
  }
  return <div class="admin-avatar-editor">
    <label>上传头像（PNG / JPEG / WebP，≤ 2 MiB）<input name={props.name} type="file" accept="image/png,image/jpeg,image/webp" disabled={props.disabled || uploading()} onChange={(event) => { const file = event.currentTarget.files?.[0]; event.currentTarget.value = ""; if (file) void upload(file); }} /></label>
    <div class="admin-avatar-preview"><Show when={props.value && localImageUrl(props.value)} fallback={<span class="admin-avatar admin-avatar-placeholder" aria-hidden="true">?</span>}>{(url) => <img class="admin-avatar" src={url()} alt="头像预览" decoding="async" />}</Show><Show when={props.value}><button type="button" disabled={props.disabled || uploading()} onClick={() => { props.onChange(null); setError(""); }}>清除头像</button></Show></div>
    <Show when={uploading()}><p role="status">正在上传头像到本站…</p></Show>
    <Show when={error()}><p class="admin-error" role="alert">{error()}</p></Show>
  </div>;
}
