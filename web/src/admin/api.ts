export interface Provider { key: string; label: string; connected?: boolean }
export interface CharacterParameters {
  effort?: string | null; temperature?: number | null; top_p?: number | null;
  max_output_tokens?: number | null; seed?: number | null;
  frequency_penalty?: number | null; presence_penalty?: number | null; stop?: string[];
}
export interface CharacterRecipe {
  name: string; display_name: string; system_prompt: string; model_id: string | null;
  parameters: CharacterParameters; enabled: boolean; sort: number;
  background: { url: string; opacity: number } | null; resource_packs: string[];
  sticker_identity: "self" | "neutral"; sticker_frequency: "rare" | "normal" | "often";
}
export interface Character extends CharacterRecipe { id: string; created_at: number; updated_at: number }
export interface ResourcePack {
  id: string; name: string; description: string; sticker_count: number; skill: string;
  attribution: string; assets: { id: string; name: string; url: string }[];
}
export interface Settings { default_model_id: string | null; default_character_id: string | null }
export interface ModelDefaults {
  display_name: string;
  protocol: "chat_completions" | "responses" | "chatgpt_responses";
  reasoning: { style: "none" | "effort" | "deepseek"; levels: string[]; default: string | null };
  vision: boolean;
  context_window: number | null;
  price: { cached_in: number | null; uncached_in: number | null; output: number | null };
}
export interface ModelSpec extends ModelDefaults { provider: string; upstream_id: string; sort: number }
export interface ListedModel extends ModelSpec { id: string }
export interface UpstreamModel { upstream_id: string; listed_model_id: string | null; supported: boolean; defaults: ModelDefaults | null }
export interface Totals {
  requests: number; cached_in: number; uncached_in: number; reasoning: number; output: number;
  cost_usd: number | null; unpriced_requests: number;
}
export interface Point extends Totals { at_ms: number }
export interface ModelStats {
  key: string; provider: string; provider_label: string; upstream_id: string; model_label: string;
  totals: Totals; points: Point[];
}
export interface Stats { from_ms: number; to_ms: number; bucket_ms: number; totals: Totals; models: ModelStats[] }
export interface QuotaWindow {
  name: "5h" | "7d" | "30d"; limit_usd: number | null; used_usd: number | null;
  remaining_usd: number | null; unknown_requests: number;
}
export interface ProviderQuota {
  key: string; label: string; kind: "deepseek" | "opencode_go" | "unknown";
  status: "ok" | "unavailable" | "error"; note: string | null;
  balance: { available: boolean; balances: { currency: string; total: string; granted: string; topped_up: string }[] } | null;
  models: { upstream_id: string; model_label: string; windows: QuotaWindow[] }[];
}
export interface Quotas { as_of_ms: number; providers: ProviderQuota[] }
export interface ChatgptStatus {
  connected: boolean; active_client_id: string | null;
  accounts: { client_id: string; label: string; connected: boolean }[];
  pending: boolean; expires_at: number | null; error: string | null;
  welcome_required: boolean; usage_url: string;
}
export interface ChatgptAuthorization { authorization_url: string; expires_at: number; loopback_available: boolean }
export class AdminError extends Error {
  constructor(readonly status: number, message: string) { super(message); }
}
export const errorMessage = (error: unknown) => error instanceof Error ? error.message : "请求失败";

export interface AdminApi {
  session(password: string): Promise<void>;
  providers(): Promise<Provider[]>;
  directory(key: string): Promise<UpstreamModel[]>;
  models(): Promise<ListedModel[]>;
  saveModel(modelId: string | null, spec: ModelSpec): Promise<ListedModel>;
  deleteModel(modelId: string): Promise<void>;
  characters(): Promise<Character[]>;
  saveCharacter(characterId: string | null, body: CharacterRecipe): Promise<Character>;
  deleteCharacter(characterId: string): Promise<void>;
  resourcePacks(): Promise<ResourcePack[]>;
  uploadBackground(file: File): Promise<{ url: string }>;
  settings(): Promise<Settings>;
  saveSettings(settings: Settings): Promise<Settings>;
  stats(from: number, to: number, bucket: number): Promise<Stats>;
  quotas(): Promise<Quotas>;
  chatgptStatus(): Promise<ChatgptStatus>;
  chatgptStart(clientId: string | null): Promise<ChatgptAuthorization>;
  chatgptCallback(redirectUrl: string): Promise<ChatgptStatus>;
  chatgptCancel(): Promise<void>;
  chatgptLogout(): Promise<ChatgptStatus>;
  chatgptSelect(clientId: string): Promise<ChatgptStatus>;
  chatgptAcknowledge(): Promise<void>;
}

/** Admin requests never join the chat access-password queue or retain a password. */
export function createAdminApi(onAccessError: (status: number) => void): AdminApi {
  async function request<T>(path: string, method = "GET", body?: unknown): Promise<T> {
    let response: Response;
    try {
      response = await fetch(`/api/admin${path}`, {
        method, credentials: "same-origin", cache: "no-store",
        ...(body === undefined ? {} : body instanceof Blob
          ? { headers: { "content-type": body.type }, body }
          : { headers: { "content-type": "application/json" }, body: JSON.stringify(body) }),
      });
    } catch { throw new AdminError(0, "网络连接失败，请检查连接后重试"); }
    if (!response.ok) {
      if (response.status === 401 || response.status === 404 && (path === "/providers" || path === "/session")) onAccessError(response.status);
      let message = `请求失败（HTTP ${response.status}）`;
      try { const data: unknown = await response.json(); if (data && typeof data === "object" && "error" in data && typeof data.error === "string") message = data.error; } catch { /* Use the status when the server did not send JSON. */ }
      throw new AdminError(response.status, message);
    }
    if (response.status === 204) { await response.arrayBuffer(); return undefined as T; }
    return await response.json() as T;
  }
  const id = encodeURIComponent;
  return {
    session: (password: string) => request<void>("/session", "POST", { password }),
    providers: () => request<Provider[]>("/providers"),
    directory: (key: string) => request<UpstreamModel[]>(`/providers/${id(key)}/models`),
    models: () => request<ListedModel[]>("/models"),
    saveModel: (modelId: string | null, spec: ModelSpec) => request<ListedModel>(`/models${modelId ? `/${id(modelId)}` : ""}`, modelId ? "PUT" : "POST", spec),
    deleteModel: (modelId: string) => request<void>(`/models/${id(modelId)}`, "DELETE"),
    characters: () => request<Character[]>("/characters"),
    saveCharacter: (characterId: string | null, body: CharacterRecipe) => request<Character>(`/characters${characterId ? `/${id(characterId)}` : ""}`, characterId ? "PUT" : "POST", body),
    deleteCharacter: (characterId: string) => request<void>(`/characters/${id(characterId)}`, "DELETE"),
    resourcePacks: () => request<ResourcePack[]>("/resource-packs"),
    uploadBackground: (file: File) => request<{ url: string }>("/backgrounds", "POST", file),
    settings: () => request<Settings>("/settings"),
    saveSettings: (settings: Settings) => request<Settings>("/settings", "PUT", settings),
    stats: (from: number, to: number, bucket: number) => request<Stats>(`/stats?${new URLSearchParams({ from_ms: String(from), to_ms: String(to), bucket_ms: String(bucket) })}`),
    quotas: () => request<Quotas>("/quotas"),
    chatgptStatus: () => request<ChatgptStatus>("/chatgpt"),
    chatgptStart: (clientId: string | null) => request<ChatgptAuthorization>("/chatgpt/start", "POST", { client_id: clientId }),
    chatgptCallback: (redirectUrl: string) => request<ChatgptStatus>("/chatgpt/callback", "POST", { redirect_url: redirectUrl }),
    chatgptCancel: () => request<void>("/chatgpt/cancel", "POST"),
    chatgptLogout: () => request<ChatgptStatus>("/chatgpt/logout", "POST"),
    chatgptSelect: (clientId: string) => request<ChatgptStatus>("/chatgpt/select", "POST", { client_id: clientId }),
    chatgptAcknowledge: () => request<void>("/chatgpt/acknowledge", "POST"),
  };
}
export const emptyModel = (provider: string, upstream_id = "", defaults: ModelDefaults | null = null): ModelSpec => ({
  provider, upstream_id, sort: 0,
  ...(defaults ?? { display_name: upstream_id, protocol: provider === "chatgpt-subscription" ? "chatgpt_responses" : "chat_completions", reasoning: { style: "none", levels: [], default: null }, vision: false, context_window: null, price: { cached_in: null, uncached_in: null, output: null } }),
});
