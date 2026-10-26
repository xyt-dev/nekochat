// App state: conversations, the open conversation tree, model choice and live (streaming) turns.
import { type Accessor, createMemo, createRoot, createSignal, type Setter } from "solid-js";
import {
  ApiError,
  api,
  type ConversationSummary,
  type ConversationView,
  type ModelsView,
  type NodeView,
  type PickerModel,
  postTurn,
  type TurnAction,
  type TurnEvent,
} from "./api";
import { inNextFrame } from "./frame";
import { indexTree, pathTo, type TreeIndex } from "./tree";
import { toast } from "./ui";

/** A reply being generated; its text is buffered and copied to signals once per frame. */
export interface LiveTurn {
  convId: string;
  started: Accessor<boolean>;
  assistantId: Accessor<string>;
  /** User message the reply answers. */
  parentId: Accessor<string | null>;
  modelLabel: Accessor<string>;
  content: Accessor<string>;
  reasoning: Accessor<string>;
  /** `performance.now()` of the first reasoning delta / first text delta after it. */
  reasoningStart: Accessor<number | null>;
  reasoningEnd: Accessor<number | null>;
  /** Set when the turn failed without a stored reply. */
  error: Accessor<string | null>;
  abort: AbortController;
  // Write side, used by the event handler only.
  buf: { content: string; reasoning: string; dirty: boolean };
  set: {
    started: Setter<boolean>;
    assistantId: Setter<string>;
    parentId: Setter<string | null>;
    modelLabel: Setter<string>;
    content: Setter<string>;
    reasoning: Setter<string>;
    reasoningStart: Setter<number | null>;
    reasoningEnd: Setter<number | null>;
    error: Setter<string | null>;
  };
}

export interface Editing {
  nodeId: string;
}

function createLiveTurn(convId: string, parentId: string | null): LiveTurn {
  const [started, setStarted] = createSignal(false);
  const [assistantId, setAssistantId] = createSignal("");
  const [parent, setParent] = createSignal(parentId);
  const [modelLabel, setModelLabel] = createSignal("");
  const [content, setContent] = createSignal("");
  const [reasoning, setReasoning] = createSignal("");
  const [reasoningStart, setReasoningStart] = createSignal<number | null>(null);
  const [reasoningEnd, setReasoningEnd] = createSignal<number | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  return {
    convId,
    started,
    assistantId,
    parentId: parent,
    modelLabel,
    content,
    reasoning,
    reasoningStart,
    reasoningEnd,
    error,
    abort: new AbortController(),
    buf: { content: "", reasoning: "", dirty: false },
    set: {
      started: setStarted,
      assistantId: setAssistantId,
      parentId: setParent,
      modelLabel: setModelLabel,
      content: setContent,
      reasoning: setReasoning,
      reasoningStart: setReasoningStart,
      reasoningEnd: setReasoningEnd,
      error: setError,
    },
  };
}

const MODEL_KEY = "nk.model";
const EFFORT_KEY = "nk.effort";

function createAppStore() {
  const [conversations, setConversations] = createSignal<ConversationSummary[]>([]);
  const [conv, setConv] = createSignal<ConversationView | null>(null, { equals: false });
  const [loadingConv, setLoadingConv] = createSignal(false);
  const [live, setLive] = createSignal<Record<string, LiveTurn>>({});
  const [nodeErrors, setNodeErrors] = createSignal<Record<string, string>>({});
  const [models, setModels] = createSignal<ModelsView>({ default_model_id: null, models: [] });
  const [modelId, setModelIdRaw] = createSignal<string | null>(localStorage.getItem(MODEL_KEY));
  const [effort, setEffortRaw] = createSignal<string | null>(localStorage.getItem(EFFORT_KEY));
  const [composerText, setComposerText] = createSignal("");
  const [editing, setEditing] = createSignal<Editing | null>(null);
  const [treeOpen, setTreeOpen] = createSignal(false);
  const [sidebarOpen, setSidebarOpen] = createSignal(false);

  const index = createMemo<TreeIndex>(() => indexTree(conv()?.nodes ?? []));
  const path = createMemo(() => pathTo(index(), conv()?.leaf_id ?? null));
  const currentLive = createMemo(() => {
    const c = conv();
    return c ? live()[c.id] : undefined;
  });

  /** The model the next turn uses: the saved choice if still listed, else the default. */
  const model = createMemo<PickerModel | undefined>(() => {
    const list = models().models;
    return (
      list.find((m) => m.id === modelId()) ??
      list.find((m) => m.id === models().default_model_id) ??
      list[0]
    );
  });
  const effectiveEffort = createMemo<string | null>(() => {
    const m = model();
    if (!m || m.reasoning.levels.length === 0) return null;
    const e = effort();
    return e && m.reasoning.levels.includes(e) ? e : (m.reasoning.default ?? m.reasoning.levels[0]);
  });

  function chooseModel(m: PickerModel) {
    setModelIdRaw(m.id);
    localStorage.setItem(MODEL_KEY, m.id);
    // A new model starts at its own default level.
    chooseEffort(m.reasoning.default);
  }

  function chooseEffort(e: string | null) {
    setEffortRaw(e);
    if (e === null) localStorage.removeItem(EFFORT_KEY);
    else localStorage.setItem(EFFORT_KEY, e);
  }

  async function loadModels() {
    try {
      setModels(await api.models());
    } catch (e) {
      toast(e instanceof ApiError ? e.message : "加载模型列表失败");
    }
  }

  async function loadConversations() {
    try {
      const list = await api.conversations();
      setConversations(list);
      const c = conv();
      const title = c && list.find((s) => s.id === c.id)?.title;
      if (c && title && title !== c.title) updateConv(c.id, (cv) => (cv.title = title));
    } catch (e) {
      toast(e instanceof ApiError ? e.message : "加载对话列表失败");
    }
  }

  let loadSeq = 0;
  async function openConversation(id: string | null) {
    const seq = ++loadSeq;
    setEditing(null);
    setTreeOpen(false);
    setSidebarOpen(false);
    if (id === null) {
      setConv(null);
      history.replaceState(null, "", location.pathname + location.search);
      return;
    }
    history.replaceState(null, "", `${location.pathname}${location.search}#/c/${encodeURIComponent(id)}`);
    setLoadingConv(true);
    try {
      const view = await api.conversation(id);
      if (seq === loadSeq) setConv(view);
    } catch (e) {
      if (seq !== loadSeq) return;
      setConv(null);
      if (e instanceof ApiError && e.status === 404) {
        toast("对话不存在");
        history.replaceState(null, "", location.pathname + location.search);
      } else toast(e instanceof ApiError ? e.message : "加载对话失败");
    } finally {
      if (seq === loadSeq) setLoadingConv(false);
    }
  }

  async function reloadConversation(id: string) {
    if (conv()?.id !== id) return;
    try {
      const view = await api.conversation(id);
      if (conv()?.id === id) setConv(view);
    } catch {
      // The next explicit action reports problems.
    }
  }

  /** Applies a change to the open conversation if it is `id`. */
  function updateConv(id: string, f: (c: ConversationView) => void) {
    const c = conv();
    if (!c || c.id !== id) return;
    f(c);
    setConv(c);
  }

  function addNode(c: ConversationView, node: NodeView) {
    const i = c.nodes.findIndex((n) => n.id === node.id);
    if (i >= 0) c.nodes[i] = node;
    else c.nodes = [...c.nodes, node];
  }

  async function newConversation(): Promise<string | null> {
    try {
      const summary = await api.createConversation();
      setConversations((list) => [summary, ...list.filter((c) => c.id !== summary.id)]);
      await openConversation(summary.id);
      return summary.id;
    } catch (e) {
      toast(e instanceof ApiError ? e.message : "新建对话失败");
      return null;
    }
  }

  async function deleteConversation(id: string) {
    try {
      live()[id]?.abort.abort();
      await api.deleteConversation(id);
      setLive((l) => {
        const { [id]: _, ...rest } = l;
        return rest;
      });
      setConversations((list) => list.filter((c) => c.id !== id));
      if (conv()?.id === id) await openConversation(null);
    } catch (e) {
      toast(e instanceof ApiError ? e.message : "删除失败");
    }
  }

  async function clearConversation(id: string) {
    try {
      live()[id]?.abort.abort();
      await api.clearConversation(id);
      setLive((l) => {
        const { [id]: _, ...rest } = l;
        return rest;
      });
      setEditing(null);
      await reloadConversation(id);
      await loadConversations();
    } catch (e) {
      toast(e instanceof ApiError ? e.message : "清空失败");
    }
  }

  /** Moves the leaf; `descend` continues down the newest branch below `nodeId`. */
  async function moveLeaf(nodeId: string, descend: boolean) {
    const c = conv();
    if (!c) return;
    if (live()[c.id] && !live()[c.id].error()) {
      toast("正在生成回复，稍后再切换");
      return;
    }
    try {
      const { leaf_id } = await api.setLeaf(c.id, nodeId, descend);
      setLive((l) => {
        const { [c.id]: _, ...rest } = l;
        return rest;
      });
      updateConv(c.id, (cv) => (cv.leaf_id = leaf_id));
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) toast("正在生成回复，稍后再切换");
      else toast(e instanceof ApiError ? e.message : "切换失败");
    }
  }

  function flushLive(turn: LiveTurn) {
    if (!turn.buf.dirty) return;
    turn.buf.dirty = false;
    turn.set.content(turn.buf.content);
    turn.set.reasoning(turn.buf.reasoning);
  }

  /** Feeds one SSE event into a live turn and the open conversation. */
  function handleTurnEvent(turn: LiveTurn, ev: TurnEvent) {
    switch (ev.type) {
      case "start": {
        const { user_node, assistant_node_id, model_label } = ev.data;
        turn.set.assistantId(assistant_node_id);
        turn.set.modelLabel(model_label);
        if (user_node) turn.set.parentId(user_node.id);
        const parent = turn.parentId();
        updateConv(turn.convId, (c) => {
          if (user_node) addNode(c, user_node);
          c.leaf_id = parent;
        });
        turn.set.started(true);
        break;
      }
      case "reasoning":
        if (turn.reasoningStart() === null) turn.set.reasoningStart(performance.now());
        turn.buf.reasoning += ev.data.t;
        turn.buf.dirty = true;
        inNextFrame(() => flushLive(turn));
        break;
      case "text":
        if (turn.reasoningStart() !== null && turn.reasoningEnd() === null) {
          turn.set.reasoningEnd(performance.now());
        }
        turn.buf.content += ev.data.t;
        turn.buf.dirty = true;
        inNextFrame(() => flushLive(turn));
        break;
      case "done":
        finishTurn(turn, ev.data.node, null);
        break;
      case "error":
        finishTurn(turn, ev.data.node, ev.data.message);
        break;
    }
  }

  function finishTurn(turn: LiveTurn, node: NodeView | null, error: string | null) {
    flushLive(turn);
    if (node) {
      if (error) setNodeErrors((m) => ({ ...m, [node.id]: error }));
      updateConv(turn.convId, (c) => {
        addNode(c, node);
        c.leaf_id = node.id;
      });
      dropLive(turn);
    } else {
      turn.set.error(error ?? "回复中断");
    }
  }

  function dropLive(turn: LiveTurn) {
    setLive((l) => {
      if (l[turn.convId] !== turn) return l;
      const { [turn.convId]: _, ...rest } = l;
      return rest;
    });
  }

  /** Starts a turn; returns `false` when it was refused before streaming began. */
  async function runTurn(action: TurnAction): Promise<boolean> {
    const m = model();
    if (!m) {
      toast("还没有可用的模型，请先在管理端上架");
      return false;
    }
    let c = conv();
    if (!c) {
      const id = await newConversation();
      c = conv();
      if (!id || !c) return false;
    }
    const existing = live()[c.id];
    if (existing && !existing.error()) {
      toast("该对话正在生成回复");
      return false;
    }
    const convId = c.id;
    const turn = createLiveTurn(convId, action.action === "regenerate" ? action.node_id : null);
    setLive((l) => ({ ...l, [convId]: turn }));
    setEditing(null);
    let settled = false;
    try {
      await postTurn(convId, { ...action, model_id: m.id, effort: effectiveEffort() }, turn.abort.signal, (ev) => {
        if (ev.type === "done" || ev.type === "error") settled = true;
        handleTurnEvent(turn, ev);
      });
      if (!settled) {
        finishTurn(turn, null, "连接中断，回复未完成");
        setTimeout(() => void reloadConversation(convId), 1500);
      }
    } catch (e) {
      if (turn.abort.signal.aborted) {
        stopped(turn);
        return true;
      }
      if (!turn.started()) {
        dropLive(turn);
        toast(e instanceof ApiError ? e.message : "发送失败");
        return false;
      }
      finishTurn(turn, null, "连接中断，回复未完成");
      setTimeout(() => void reloadConversation(convId), 1500);
    }
    void loadConversations();
    return true;
  }

  /** The user stopped generating: keep what arrived locally; the server stores it as interrupted. */
  function stopped(turn: LiveTurn) {
    flushLive(turn);
    if (turn.started() && (turn.buf.content || turn.buf.reasoning)) {
      const node: NodeView = {
        id: turn.assistantId(),
        parent_id: turn.parentId(),
        role: "assistant",
        content: turn.buf.content,
        reasoning: turn.buf.reasoning || null,
        model_label: turn.modelLabel(),
        effort: null,
        status: "interrupted",
        metrics: null,
        created_at: Date.now(),
      };
      updateConv(turn.convId, (c) => {
        addNode(c, node);
        c.leaf_id = node.id;
      });
    }
    dropLive(turn);
    // Pick up the stored version (with metrics) once the server has saved it.
    setTimeout(() => {
      if (!live()[turn.convId]) void reloadConversation(turn.convId);
    }, 1500);
  }

  return {
    conversations,
    conv,
    setConv,
    loadingConv,
    live,
    setLive,
    currentLive,
    nodeErrors,
    models,
    model,
    effort: effectiveEffort,
    chooseModel,
    chooseEffort,
    composerText,
    setComposerText,
    editing,
    setEditing,
    treeOpen,
    setTreeOpen,
    sidebarOpen,
    setSidebarOpen,
    index,
    path,
    loadModels,
    loadConversations,
    openConversation,
    newConversation,
    deleteConversation,
    clearConversation,
    moveLeaf,
    runTurn,
    createLiveTurn,
    handleTurnEvent,
  };
}

export const store = createRoot(createAppStore);
