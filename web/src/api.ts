// HTTP client for account-scoped JSON calls and SSE turns.

export type Role = "user" | "assistant";
export type NodeStatus = "complete" | "interrupted" | "error";

export interface Usage {
  cached_in: number;
  uncached_in: number;
  reasoning: number;
  output: number;
}

export interface Metrics {
  ttft_ms: number | null;
  total_ms: number;
  tokens_per_sec: number | null;
  reasoning_ms?: number | null;
  usage: Usage;
}

export interface Attachment {
  id: string;
  name: string;
  kind: "image" | "document";
  media_type: string;
  size: number;
  url: string;
}

export interface NodeView {
  id: string;
  parent_id: string | null;
  role: Role;
  content: string;
  attachments: Attachment[];
  reasoning: string | null;
  character_id: string | null;
  character_name: string | null;
  character_avatar: string | null;
  status: NodeStatus;
  metrics: Metrics | null;
  created_at: number;
}

export interface ConversationSummary {
  id: string;
  title: string;
  character_id: string | null;
  updated_at: number;
}

export interface ConversationView {
  id: string;
  title: string;
  character_id: string | null;
  leaf_id: string | null;
  nodes: NodeView[];
}

export interface CharacterBackground {
  url: string;
  opacity: number;
}

export interface PickerCharacter {
  id: string;
  display_name: string;
  background: CharacterBackground | null;
  vision: boolean;
  avatar: string | null;
}

export interface CharactersView {
  default_character_id: string | null;
  characters: PickerCharacter[];
}

export interface CustomTheme {
  id: string;
  name: string;
  url: string;
}

export type TurnAction =
  | { action: "send"; content: string; attachment_ids?: string[] }
  | { action: "edit"; node_id: string; content: string; attachment_ids?: string[] }
  | { action: "regenerate"; node_id: string };

export type TurnRequest = TurnAction & { character_id: string };

export interface StartEvent {
  user_node: NodeView | null;
  assistant_node_id: string;
  character_id: string;
  character_name: string;
  character_avatar: string | null;
}

export type TurnEvent =
  | { type: "start"; data: StartEvent }
  | { type: "reasoning"; data: { t: string } }
  | { type: "text"; data: { t: string } }
  | { type: "done"; data: { node: NodeView } }
  | { type: "error"; data: { message: string; node: NodeView | null } };

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

export interface UserPublic {
  id: string;
  username: string;
  avatar: string | null;
  last_character_id: string | null;
  last_conversation_id: string | null;
}

let requestEpoch = 0;
const pendingRequests = new Set<AbortController>();
export function resetAccountRequests(): void {
  requestEpoch++;
  for (const controller of pendingRequests) controller.abort();
  pendingRequests.clear();
}

async function toError(r: Response): Promise<ApiError> {
  let message = `请求失败（HTTP ${r.status}）`;
  try {
    const body = await r.json();
    if (body && typeof body.error === "string") message = body.error;
  } catch {
    // Not JSON: keep the generic message.
  }
  return new ApiError(r.status, message);
}

/** Never retry an old account's request after a different account logs in. */
async function request<T>(path: string, init: RequestInit, consume: (response: Response) => Promise<T>): Promise<T> {
  const epoch = requestEpoch;
  const controller = new AbortController();
  const abort = () => controller.abort();
  init.signal?.addEventListener("abort", abort, { once: true });
  if (init.signal?.aborted) controller.abort();
  pendingRequests.add(controller);
  try {
    const r = await fetch(path, { ...init, signal: controller.signal });
    if (epoch !== requestEpoch) throw new DOMException("Account changed", "AbortError");
    if (r.status === 401 && !["/api/account/login", "/api/account/register", "/api/account/session"].includes(path)) {
      window.dispatchEvent(new Event("account-expired"));
    }
    if (!r.ok) throw await toError(r);
    const result = await consume(r);
    if (epoch !== requestEpoch) throw new DOMException("Account changed", "AbortError");
    return result;
  } catch (error) {
    if (controller.signal.aborted || error instanceof ApiError) throw error;
    throw new ApiError(0, "网络连接失败");
  } finally {
    pendingRequests.delete(controller);
    init.signal?.removeEventListener("abort", abort);
  }
}

async function json<T>(path: string, method = "GET", body?: unknown): Promise<T> {
  const init: RequestInit = { method };
  if (body !== undefined) {
    init.headers = { "content-type": "application/json" };
    init.body = JSON.stringify(body);
  }
  return request(path, init, async (r) => {
    if (r.status === 204) {
      await r.arrayBuffer();
      return undefined as T;
    }
    return (await r.json()) as T;
  });
}

const conv = (id: string) => `/api/conversations/${encodeURIComponent(id)}`;

export const api = {
  session: () => json<UserPublic | null>("/api/account/session"),
  authenticate: (mode: "login" | "register", username: string, password: string) =>
    json<UserPublic>(`/api/account/${mode}`, "POST", { username, password }),
  logout: () => json<void>("/api/account/session", "DELETE"),
  profile: (body: { username?: string; password?: string; current_password?: string; avatar?: string | null }) =>
    json<UserPublic>("/api/account", "PATCH", body),
  preferences: (body: { last_character_id: string | null; last_conversation_id: string | null }) =>
    json<UserPublic>("/api/account/preferences", "PUT", body),
  uploadAvatar: async (file: File): Promise<{ url: string }> => {
    return request("/api/account/avatar", {
      method: "POST", headers: { "content-type": file.type || "application/octet-stream", "x-file-name": encodeURIComponent(file.name) }, body: file,
    }, (response) => response.json());
  },
  characters: () => json<CharactersView>("/api/characters"),
  conversations: () => json<ConversationSummary[]>("/api/conversations"),
  createConversation: (character_id?: string) =>
    json<ConversationSummary>("/api/conversations", "POST", character_id ? { character_id } : {}),
  conversation: (id: string) => json<ConversationView>(conv(id)),
  uploadAttachment: async (id: string, file: File): Promise<Attachment> => {
    return request(`${conv(id)}/attachments`, {
      method: "POST", headers: { "content-type": file.type || "application/octet-stream", "x-file-name": encodeURIComponent(file.name) }, body: file,
    }, (response) => response.json());
  },
  deleteConversation: (id: string) => json<void>(conv(id), "DELETE"),
  clearConversation: (id: string) => json<void>(`${conv(id)}/clear`, "POST"),
  setLeaf: (id: string, node_id: string, descend: boolean) =>
    json<{ leaf_id: string }>(`${conv(id)}/leaf`, "PUT", { node_id, descend }),
  themes: () => json<CustomTheme[]>("/api/themes"),
};

// ---- SSE ----

/** Parses a `text/event-stream` body into `{event, data}` records; comments are skipped. */
export async function* sseRecords(
  body: ReadableStream<Uint8Array>,
): AsyncGenerator<{ event: string; data: string }> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  let event = "";
  let data: string[] = [];
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buf += decoder.decode(value, { stream: true });
    let nl: number;
    while ((nl = buf.indexOf("\n")) >= 0) {
      let line = buf.slice(0, nl);
      buf = buf.slice(nl + 1);
      if (line.endsWith("\r")) line = line.slice(0, -1);
      if (line === "") {
        if (data.length > 0) yield { event: event || "message", data: data.join("\n") };
        event = "";
        data = [];
        continue;
      }
      if (line.startsWith(":")) continue;
      const colon = line.indexOf(":");
      const field = colon < 0 ? line : line.slice(0, colon);
      let value = colon < 0 ? "" : line.slice(colon + 1);
      if (value.startsWith(" ")) value = value.slice(1);
      if (field === "event") event = value;
      else if (field === "data") data.push(value);
    }
  }
}

/** Consume SSE to completion, keeping stop/account aborts connected to the response body. */
export async function postTurn(
  convId: string,
  body: TurnRequest,
  signal: AbortSignal,
  onEvent: (ev: TurnEvent) => void,
): Promise<void> {
  await request(`${conv(convId)}/turns`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    signal,
  }, async (r) => {
    if (!r.body) throw new ApiError(0, "服务器没有返回内容");
    for await (const rec of sseRecords(r.body)) {
      const type = rec.event as TurnEvent["type"];
      if (!["start", "reasoning", "text", "done", "error"].includes(type)) continue;
      onEvent({ type, data: JSON.parse(rec.data) } as TurnEvent);
    }
  });
}
