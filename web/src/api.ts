// HTTP client for the nekochat server: JSON calls, the access-password gate and SSE turns.
import { createSignal } from "solid-js";

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

export interface NodeView {
  id: string;
  parent_id: string | null;
  role: Role;
  content: string;
  reasoning: string | null;
  model_label: string | null;
  effort: string | null;
  status: NodeStatus;
  metrics: Metrics | null;
  created_at: number;
}

export interface ConversationSummary {
  id: string;
  title: string;
  updated_at: number;
}

export interface ConversationView {
  id: string;
  title: string;
  leaf_id: string | null;
  nodes: NodeView[];
}

export interface PickerModel {
  id: string;
  display_name: string;
  provider_label: string;
  label: string;
  reasoning: { levels: string[]; default: string | null };
  vision: boolean;
}

export interface ModelsView {
  default_model_id: string | null;
  models: PickerModel[];
}

export interface CustomTheme {
  id: string;
  name: string;
  url: string;
}

export type TurnAction =
  | { action: "send"; content: string }
  | { action: "edit"; node_id: string; content: string }
  | { action: "regenerate"; node_id: string };

export type TurnRequest = TurnAction & { model_id: string; effort: string | null };

export interface StartEvent {
  user_node: NodeView | null;
  assistant_node_id: string;
  model_label: string;
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

// ---- access password gate ----

const [loginNeeded, setLoginNeeded] = createSignal(false);
export { loginNeeded };
let loginWaiters: (() => void)[] = [];

/** `POST /api/session`; on success every request parked on a 401 is retried. */
export async function login(password: string): Promise<void> {
  const r = await fetch("/api/session", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ password }),
  });
  if (!r.ok) throw await toError(r);
  await r.arrayBuffer();
  setLoginNeeded(false);
  const waiters = loginWaiters;
  loginWaiters = [];
  for (const w of waiters) w();
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

/** fetch with the 401 → password page → retry loop; non-2xx becomes an `ApiError`. */
async function request(path: string, init: RequestInit = {}): Promise<Response> {
  for (;;) {
    let r: Response;
    try {
      r = await fetch(path, init);
    } catch (e) {
      if (init.signal?.aborted) throw e;
      throw new ApiError(0, "网络连接失败");
    }
    if (r.status === 401) {
      setLoginNeeded(true);
      const { promise, resolve } = Promise.withResolvers<void>();
      loginWaiters.push(resolve);
      await promise;
      continue;
    }
    if (!r.ok) throw await toError(r);
    return r;
  }
}

async function json<T>(path: string, method = "GET", body?: unknown): Promise<T> {
  const init: RequestInit = { method };
  if (body !== undefined) {
    init.headers = { "content-type": "application/json" };
    init.body = JSON.stringify(body);
  }
  const r = await request(path, init);
  if (r.status === 204) {
    // Drain the empty body so the request completes instead of being cancelled.
    await r.arrayBuffer();
    return undefined as T;
  }
  return (await r.json()) as T;
}

const conv = (id: string) => `/api/conversations/${encodeURIComponent(id)}`;

export const api = {
  models: () => json<ModelsView>("/api/models"),
  conversations: () => json<ConversationSummary[]>("/api/conversations"),
  createConversation: () => json<ConversationSummary>("/api/conversations", "POST"),
  conversation: (id: string) => json<ConversationView>(conv(id)),
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

/** `POST …/turns`: resolves once the SSE stream is open, then reports each event. */
export async function postTurn(
  convId: string,
  body: TurnRequest,
  signal: AbortSignal,
  onEvent: (ev: TurnEvent) => void,
): Promise<void> {
  const r = await request(`${conv(convId)}/turns`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
  if (!r.body) throw new ApiError(0, "服务器没有返回内容");
  for await (const rec of sseRecords(r.body)) {
    const type = rec.event as TurnEvent["type"];
    if (!["start", "reasoning", "text", "done", "error"].includes(type)) continue;
    onEvent({ type, data: JSON.parse(rec.data) } as TurnEvent);
  }
}
