// App state: conversations, the open conversation tree, role choice and live (streaming) turns.
import { type Accessor, batch, createMemo, createRoot, createSignal, type Setter } from "solid-js";
import {
  ApiError,
  api,
  type Attachment,
  type ConversationSummary,
  type ConversationView,
  type CharactersView,
  type NodeView,
  type PickerCharacter,
  type UserPublic,
  resetAccountRequests,
  postTurn,
  type TurnAction,
  type TurnEvent,
} from "./api";
import { inNextFrame } from "./frame";
import { indexTree, pathTo, type TreeIndex } from "./tree";
import { resetAccountUi, toast } from "./ui";
import { resetMarkdownCache } from "./md/client";

/** A reply being generated; its text is buffered and copied to signals once per frame. */
export interface LiveTurn {
  convId: string;
  started: Accessor<boolean>;
  assistantId: Accessor<string>;
  /** User message the reply answers. */
  parentId: Accessor<string | null>;
  characterId: Accessor<string>;
  characterName: Accessor<string>;
  characterAvatar: Accessor<string | null>;
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
    characterId: Setter<string>;
    characterName: Setter<string>;
    characterAvatar: Setter<string | null>;
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
  const [characterId, setCharacterId] = createSignal("");
  const [characterName, setCharacterName] = createSignal("");
  const [characterAvatar, setCharacterAvatar] = createSignal<string | null>(null);
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
    characterId,
    characterName,
    content,
    characterAvatar,
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
      characterId: setCharacterId,
      characterName: setCharacterName,
      content: setContent,
      characterAvatar: setCharacterAvatar,
      reasoning: setReasoning,
      reasoningStart: setReasoningStart,
      reasoningEnd: setReasoningEnd,
      error: setError,
    },
  };
}

function createAppStore() {
  const [user, setUser] = createSignal<UserPublic | null>(null);
  const [sessionReady, setSessionReady] = createSignal(false);
  const [sessionVersion, setSessionVersion] = createSignal(0);
  let sessionEpoch = 0;
  let preferenceQueue = Promise.resolve();
  const [conversations, setConversations] = createSignal<ConversationSummary[]>([]);
  const [conv, setConv] = createSignal<ConversationView | null>(null, { equals: false });
  const [loadingConv, setLoadingConv] = createSignal(false);
  const [live, setLive] = createSignal<Record<string, LiveTurn>>({});
  const [nodeErrors, setNodeErrors] = createSignal<Record<string, string>>({});
  const [characters, setCharacters] = createSignal<CharactersView>({ default_character_id: null, characters: [] });
  const [characterId, setCharacterId] = createSignal<string | null>(null);
  const [composerText, setComposerText] = createSignal("");
  const [composerAttachments, setComposerAttachments] = createSignal<Attachment[]>([]);
  const [uploadingAttachments, setUploadingAttachments] = createSignal(0);
  const [editing, setEditing] = createSignal<Editing | null>(null);
  const [treeOpen, setTreeOpen] = createSignal(false);
  const [sidebarOpen, setSidebarOpen] = createSignal(false);
  let uploadDraftVersion = 0;

  const index = createMemo<TreeIndex>(() => indexTree(conv()?.nodes ?? []));
  const path = createMemo(() => pathTo(index(), conv()?.leaf_id ?? null));
  const currentLive = createMemo(() => {
    const c = conv();
    return c ? live()[c.id] : undefined;
  });

  /** A role switch only affects subsequent turns; the message tree stays untouched. */
  const character = createMemo<PickerCharacter | undefined>(() => {
    const list = characters().characters;
    return list.find((item) => item.id === characterId()) ??
      list.find((item) => item.id === characters().default_character_id);
  });

  function resetSession() {
    sessionEpoch++;
    setSessionVersion(sessionEpoch);
    resetAccountRequests();
    for (const turn of Object.values(live())) turn.abort.abort();
    resetAccountUi();
    resetMarkdownCache();
    loadSeq++;
    uploadDraftVersion++;
    creatingConversation = null;
    batch(() => {
      setUser(null);
      setConversations([]);
      setConv(null);
      setLive({});
      setNodeErrors({});
      setCharacters({ default_character_id: null, characters: [] });
      setCharacterId(null);
      setComposerText("");
      setComposerAttachments([]);
      setUploadingAttachments(0);
      setEditing(null);
      setLoadingConv(false);
      setSidebarOpen(false);
      setTreeOpen(false);
    });
    history.replaceState(null, "", location.pathname + location.search);
  }

  function persistPreferences() {
    if (!user()) return;
    const epoch = sessionEpoch;
    const preferences = { last_character_id: character()?.id ?? null, last_conversation_id: conv()?.id ?? null };
    preferenceQueue = preferenceQueue.catch(() => {}).then(async () => {
      if (epoch !== sessionEpoch || !user()) return;
      try {
        const saved = await api.preferences(preferences);
        if (epoch === sessionEpoch) setUser((current) => current ? { ...current, last_character_id: saved.last_character_id, last_conversation_id: saved.last_conversation_id } : null);
      } catch (error) {
        if (epoch === sessionEpoch) toast(error instanceof ApiError ? error.message : "保存偏好失败");
      }
    });
  }

  async function enterSession(account: UserPublic, requestedConversation: string | null = null) {
    resetSession();
    setUser(account);
    const epoch = sessionEpoch;
    await Promise.all([loadCharacters(), loadConversations()]);
    if (epoch !== sessionEpoch) return;
    const available = characters().characters;
    setCharacterId(available.some((item) => item.id === account.last_character_id) ? account.last_character_id : characters().default_character_id);
    const id = requestedConversation && conversations().some((item) => item.id === requestedConversation)
      ? requestedConversation : account.last_conversation_id;
    if (id && conversations().some((item) => item.id === id)) await openConversation(id, false);
  }

  async function bootstrapSession() {
    const epoch = sessionEpoch;
    const match = location.hash.match(/^#\/c\/(.+)$/);
    let requested: string | null = null;
    try { requested = match ? decodeURIComponent(match[1]) : null; } catch { /* Invalid location is not an account preference. */ }
    try {
      const account = await api.session();
      if (epoch !== sessionEpoch) return;
      if (account) await enterSession(account, requested);
      else resetSession();
    } catch (error) {
      if (epoch === sessionEpoch) toast(error instanceof ApiError ? error.message : "加载登录状态失败");
    } finally {
      setSessionReady(true);
    }
  }

  async function authenticate(mode: "login" | "register", username: string, password: string) {
    resetSession();
    const epoch = sessionEpoch;
    const account = await api.authenticate(mode, username, password);
    if (epoch === sessionEpoch) await enterSession(account);
  }

  async function logout() {
    resetSession();
    setSessionReady(false);
    try { await api.logout(); }
    finally { setSessionReady(true); }
  }

  function chooseCharacter(item: PickerCharacter) {
    setCharacterId(item.id);
    persistPreferences();
  }

  async function loadCharacters() {
    const epoch = sessionEpoch;
    try {
      const list = await api.characters();
      if (epoch === sessionEpoch) setCharacters(list);
    } catch (e) {
      if (epoch === sessionEpoch) toast(e instanceof ApiError ? e.message : "加载角色列表失败");
    }
  }

  async function loadConversations() {
    const epoch = sessionEpoch;
    try {
      const list = await api.conversations();
      if (epoch !== sessionEpoch) return;
      setConversations(list);
      const c = conv();
      const title = c && list.find((s) => s.id === c.id)?.title;
      if (c && title && title !== c.title) updateConv(c.id, (cv) => (cv.title = title));
    } catch (e) {
      if (epoch === sessionEpoch) toast(e instanceof ApiError ? e.message : "加载对话列表失败");
    }
  }

  let loadSeq = 0;
  async function openConversation(id: string | null, persist = true, preserveDraft = false) {
    if (!user()) return;
    const seq = ++loadSeq;
    uploadDraftVersion++;
    if (!preserveDraft) {
      setComposerAttachments([]);
      setComposerText("");
    }
    setEditing(null);
    setTreeOpen(false);
    setSidebarOpen(false);
    if (id === null) {
      setConv(null);
      history.replaceState(null, "", location.pathname + location.search);
      if (persist) persistPreferences();
      return;
    }
    history.replaceState(null, "", `${location.pathname}${location.search}#/c/${encodeURIComponent(id)}`);
    setLoadingConv(true);
    try {
      const view = await api.conversation(id);
      if (seq === loadSeq) {
        setConv(view);
        if (persist) persistPreferences();
      }
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
    const epoch = sessionEpoch;
    try {
      const view = await api.conversation(id);
      if (epoch === sessionEpoch && conv()?.id === id) setConv(view);
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

  let creatingConversation: Promise<string | null> | null = null;
  function newConversation(preserveDraft = false): Promise<string | null> {
    if (!user()) return Promise.resolve(null);
    const epoch = sessionEpoch;
    if (creatingConversation) return creatingConversation;
    // Detach from the old chat before the request; immediate sends await this same creation.
    if (!preserveDraft) void openConversation(null);
    creatingConversation = (async () => {
      try {
        const summary = await api.createConversation(character()?.id);
        if (epoch !== sessionEpoch) return null;
        setConversations((list) => [summary, ...list.filter((c) => c.id !== summary.id)]);
        await openConversation(summary.id, true, true);
        return summary.id;
      } catch (e) {
        if (epoch === sessionEpoch) toast(e instanceof ApiError ? e.message : "新建对话失败");
        return null;
      }
    })().finally(() => { if (epoch === sessionEpoch) creatingConversation = null; });
    return creatingConversation;
  }

  async function deleteConversation(id: string) {
    const epoch = sessionEpoch;
    try {
      live()[id]?.abort.abort();
      await api.deleteConversation(id);
      if (epoch !== sessionEpoch) return;
      setLive((l) => {
        const { [id]: _, ...rest } = l;
        return rest;
      });
      setConversations((list) => list.filter((c) => c.id !== id));
      if (conv()?.id === id) await openConversation(null);
    } catch (e) {
      if (epoch === sessionEpoch) toast(e instanceof ApiError ? e.message : "删除失败");
    }
  }

  async function clearConversation(id: string) {
    const epoch = sessionEpoch;
    try {
      live()[id]?.abort.abort();
      await api.clearConversation(id);
      if (epoch !== sessionEpoch) return;
      setLive((l) => {
        const { [id]: _, ...rest } = l;
        return rest;
      });
      setEditing(null);
      if (conv()?.id === id) setComposerAttachments([]);
      uploadDraftVersion++;
      await reloadConversation(id);
      await loadConversations();
    } catch (e) {
      if (epoch === sessionEpoch) toast(e instanceof ApiError ? e.message : "清空失败");
    }
  }

  /** Moves the leaf; `descend` continues down the newest branch below `nodeId`. */
  async function moveLeaf(nodeId: string, descend: boolean) {
    const c = conv();
    const epoch = sessionEpoch;
    if (!c) return;
    if (live()[c.id] && !live()[c.id].error()) {
      toast("正在生成回复，稍后再切换");
      return;
    }
    try {
      const { leaf_id } = await api.setLeaf(c.id, nodeId, descend);
      if (epoch !== sessionEpoch) return;
      setLive((l) => {
        const { [c.id]: _, ...rest } = l;
        return rest;
      });
      updateConv(c.id, (cv) => (cv.leaf_id = leaf_id));
    } catch (e) {
      if (epoch !== sessionEpoch) return;
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
        const { user_node, assistant_node_id, character_id, character_name, character_avatar } = ev.data;
        turn.set.assistantId(assistant_node_id);
        turn.set.characterId(character_id);
        turn.set.characterName(character_name);
        turn.set.characterAvatar(character_avatar);
        if (user_node) turn.set.parentId(user_node.id);
        const parent = turn.parentId();
        updateConv(turn.convId, (c) => {
          if (user_node) addNode(c, user_node);
          c.leaf_id = parent;
          c.character_id = character_id;
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
    batch(() => {
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
    });
  }

  async function uploadAttachments(files: File[]) {
    if (!user() || files.length === 0) return;
    if (composerAttachments().length + uploadingAttachments() + files.length > 5) {
      toast("每条消息最多 5 个附件");
      return;
    }
    if (files.some((file) => file.size === 0 || file.size > (file.type.startsWith("image/") ? 8 : 16) * 1024 * 1024)) {
      toast("图片最大 8 MiB，文档最大 16 MiB，不能上传空文件");
      return;
    }
    const epoch = sessionEpoch;
    setUploadingAttachments((count) => count + files.length);
    try {
      const id = conv()?.id ?? await newConversation(true);
      if (epoch !== sessionEpoch || !id) return;
      const version = uploadDraftVersion;
      for (const file of files) {
        if (epoch !== sessionEpoch || uploadDraftVersion !== version || conv()?.id !== id) break;
        try {
          const attachment = await api.uploadAttachment(id, file);
          if (epoch === sessionEpoch && conv()?.id === id && uploadDraftVersion === version) setComposerAttachments((list) => [...list, attachment]);
        } catch (error) {
          if (epoch === sessionEpoch) toast(error instanceof ApiError ? error.message : "附件上传失败");
        }
      }
    } finally {
      if (epoch === sessionEpoch) setUploadingAttachments((count) => count - files.length);
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
    if (!user()) return false;
    const epoch = sessionEpoch;
    const selected = character();
    if (!selected) {
      toast("还没有可用的角色，请先在管理端配置并启用");
      return false;
    }
    let c = conv();
    if (!c) {
      const id = await newConversation();
      if (epoch !== sessionEpoch) return false;
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
      await postTurn(convId, { ...action, character_id: selected.id }, turn.abort.signal, (ev) => {
        if (epoch !== sessionEpoch) return;
        if (ev.type === "done" || ev.type === "error") settled = true;
        handleTurnEvent(turn, ev);
      });
      if (epoch !== sessionEpoch) return true;
      if (!settled) {
        finishTurn(turn, null, "连接中断，回复未完成");
        setTimeout(() => void reloadConversation(convId), 1500);
      }
    } catch (e) {
      if (epoch !== sessionEpoch) return true;
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
    if (epoch === sessionEpoch) void loadConversations();
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
        attachments: [],
        content: turn.buf.content,
        reasoning: turn.buf.reasoning || null,
        character_id: turn.characterId(),
        character_name: turn.characterName(),
        character_avatar: turn.characterAvatar(),
        status: "interrupted",
        metrics: null,
        created_at: Date.now(),
      };
      batch(() => {
        updateConv(turn.convId, (c) => {
          addNode(c, node);
          c.leaf_id = node.id;
        });
        dropLive(turn);
      });
    } else {
      dropLive(turn);
    }
    // Pick up the stored version (with metrics) once the server has saved it.
    setTimeout(() => {
      if (!live()[turn.convId]) void reloadConversation(turn.convId);
    }, 1500);
  }

  return {
    user,
    setUser,
    sessionReady,
    sessionVersion,
    bootstrapSession,
    authenticate,
    logout,
    resetSession,
    conversations,
    conv,
    setConv,
    loadingConv,
    live,
    setLive,
    currentLive,
    nodeErrors,
    characters,
    character,
    chooseCharacter,
    composerText,
    setComposerText,
    composerAttachments,
    setComposerAttachments,
    uploadingAttachments,
    uploadAttachments,
    editing,
    setEditing,
    treeOpen,
    setTreeOpen,
    sidebarOpen,
    setSidebarOpen,
    index,
    path,
    loadCharacters,
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
