import type { CharacterBackground } from "../api";

export interface Provider { key: string; label: string; connected?: boolean }
export interface CharacterParameters {
  effort?: string | null; temperature?: number | null; top_p?: number | null;
  max_output_tokens?: number | null; seed?: number | null;
  frequency_penalty?: number | null; presence_penalty?: number | null; stop?: string[];
}
export interface CharacterRecipe {
  name: string; display_name: string; system_prompt: string; model_id: string | null;
  search_service_id: string | null;
  parameters: CharacterParameters; enabled: boolean; sort: number;
  avatar: string | null; background: CharacterBackground | null; resource_packs: string[];
  sticker_identity: "self" | "neutral"; sticker_frequency: "rare" | "normal" | "often";
}
export interface Character extends CharacterRecipe { id: string; created_at: number; updated_at: number }
export interface ResourcePack {
  id: string; name: string; description: string; sticker_count: number; skill: string;
  attribution: string; assets: { id: string; name: string; url: string }[];
}
export interface Settings { default_character_id: string | null }
export type PasswordStorage = "plain" | "hash";
export interface UserAdmin {
  id: string; username: string; avatar: string | null;
  last_character_id: string | null; last_conversation_id: string | null;
  disabled: boolean; password_storage: PasswordStorage; created_at: number;
}
export interface UserChanges { username?: string; password?: string; avatar?: string | null; disabled?: boolean }
export interface UserSettings { password_storage: PasswordStorage; legacy_conversations: number }
export interface UserPassword { password: string | null; password_storage: PasswordStorage }
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
export interface SearchConnection {
  enabled: boolean; base_url: string; timeout_secs: number; max_calls_per_turn: number; max_results: number;
  has_access_token: boolean; has_admin_token: boolean;
}
export interface SearchConnectionWrite {
  enabled: boolean; base_url: string; timeout_secs: number; max_calls_per_turn: number; max_results: number;
  access_token?: string; admin_token?: string; clear_access_token?: boolean; clear_admin_token?: boolean;
}
export type SearchServiceKind = "deepseek" | "gpt";
export type SearchAuthMode = "api_key" | "shared_chatgpt";
export interface SearchService {
  id: string; name: string; kind: SearchServiceKind; enabled: boolean; auth_mode: SearchAuthMode;
  model: string | null; has_api_key: boolean; connected: boolean; supports_fetch: boolean;
}
export interface SearchServiceWrite {
  name: string; kind: SearchServiceKind; enabled: boolean; auth_mode: SearchAuthMode;
  model?: string | null; api_key?: string; clear_api_key?: boolean;
}
export interface SearchUsage {
  requests: number; cached_in: number; uncached_in: number; output: number; reasoning: number;
  cost: { amount: number; currency: string } | null;
}
export interface SearchSource {
  id: string; title: string; url: string; snippet: string; summary?: string | null;
  content?: string | null; published_at?: string | null;
}
export interface SearchResponse {
  query: string; sources: SearchSource[]; summary?: string | null; retrieved_at_ms: number; usage: SearchUsage;
}
export interface SearchUsageRecord {
  id: string; at_ms: number; service_id: string; status: string; usage: SearchUsage | null;
}
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
  uploadAvatar(file: File): Promise<{ url: string }>;
  users(): Promise<{ users: UserAdmin[] }>;
  createUser(body: { username: string; password: string }): Promise<UserAdmin>;
  updateUser(userId: string, body: UserChanges): Promise<UserAdmin>;
  deleteUser(userId: string): Promise<void>;
  userPassword(userId: string): Promise<UserPassword>;
  userSettings(): Promise<UserSettings>;
  saveUserSettings(passwordStorage: PasswordStorage): Promise<UserSettings>;
  assignLegacyConversations(userId: string): Promise<{ assigned: number }>;
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
  searchConnection(signal?: AbortSignal): Promise<SearchConnection>;
  saveSearchConnection(body: SearchConnectionWrite, signal?: AbortSignal): Promise<SearchConnection>;
  testSearchConnection(signal?: AbortSignal): Promise<{ ok: boolean; services: number }>;
  searchServices(signal?: AbortSignal): Promise<SearchService[]>;
  saveSearchService(serviceId: string | null, body: SearchServiceWrite, signal?: AbortSignal): Promise<SearchService>;
  deleteSearchService(serviceId: string, signal?: AbortSignal): Promise<void>;
  testSearchService(serviceId: string, query: string, signal?: AbortSignal): Promise<SearchResponse>;
  searchUsage(from: number, to: number, signal?: AbortSignal): Promise<SearchUsageRecord[]>;
}

/** Admin requests never join the chat access-password queue or retain a password. */
export function createAdminApi(onAccessError: (status: number) => void): AdminApi {
  async function request<T>(path: string, method = "GET", body?: unknown, signal?: AbortSignal): Promise<T> {
    let response: Response;
    try {
      response = await fetch(`/api/admin${path}`, {
        method, credentials: "same-origin", cache: "no-store", signal,
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
    uploadAvatar: (file: File) => request<{ url: string }>("/avatars", "POST", file),
    users: () => request<{ users: UserAdmin[] }>("/users"),
    createUser: (body: { username: string; password: string }) => request<UserAdmin>("/users", "POST", body),
    updateUser: (userId: string, body: UserChanges) => request<UserAdmin>(`/users/${id(userId)}`, "PATCH", body),
    deleteUser: (userId: string) => request<void>(`/users/${id(userId)}`, "DELETE"),
    userPassword: (userId: string) => request<UserPassword>(`/users/${id(userId)}/password`, "POST", {}),
    userSettings: () => request<UserSettings>("/user-settings"),
    saveUserSettings: (passwordStorage: PasswordStorage) => request<UserSettings>("/user-settings", "PUT", { password_storage: passwordStorage }),
    assignLegacyConversations: (userId: string) => request<{ assigned: number }>(`/users/${id(userId)}/legacy-conversations`, "POST", {}),
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
    searchConnection: (signal) => request<SearchConnection>("/search/connection", "GET", undefined, signal),
    saveSearchConnection: (body, signal) => request<SearchConnection>("/search/connection", "PUT", body, signal),
    testSearchConnection: (signal) => request<{ ok: boolean; services: number }>("/search/connection/test", "POST", {}, signal),
    searchServices: (signal) => request<SearchService[]>("/search/services", "GET", undefined, signal),
    saveSearchService: (serviceId, body, signal) => request<SearchService>(`/search/services${serviceId ? `/${id(serviceId)}` : ""}`, serviceId ? "PUT" : "POST", body, signal),
    deleteSearchService: (serviceId, signal) => request<void>(`/search/services/${id(serviceId)}`, "DELETE", undefined, signal),
    testSearchService: (serviceId, query, signal) => request<SearchResponse>(`/search/services/${id(serviceId)}/test`, "POST", { query }, signal),
    searchUsage: (from, to, signal) => request<SearchUsageRecord[]>(`/search/usage?${new URLSearchParams({ from_ms: String(from), to_ms: String(to) })}`, "GET", undefined, signal),
  };
}
export const emptyModel = (provider: string, upstream_id = "", defaults: ModelDefaults | null = null): ModelSpec => ({
  provider, upstream_id, sort: 0,
  ...(defaults ?? { display_name: upstream_id, protocol: provider === "chatgpt-subscription" ? "chatgpt_responses" : "chat_completions", reasoning: { style: "none", levels: [], default: null }, vision: false, context_window: null, price: { cached_in: null, uncached_in: null, output: null } }),
});
