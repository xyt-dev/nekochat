import { type Accessor, createSignal, For, type JSX, onCleanup, Show } from "solid-js";
import type { Metrics, NodeStatus, NodeView } from "../api";
import { type LiveTurn, store } from "../store";
import { siblingsOf } from "../tree";
import { Markdown } from "./Markdown";
import { Avatar, formatFileSize } from "./Files";

/** Hover actions; on touch screens a tap on the message reveals them instead. */
function MessageShell(props: { class: string; children: JSX.Element }) {
  const [tapped, setTapped] = createSignal(false);
  return (
    <div
      class={`msg ${props.class}`}
      classList={{ tapped: tapped() }}
      onClick={(e) => {
        if ((e.target as HTMLElement).closest("button, a, summary, input, select, textarea, label, [role='button']")) return;
        setTapped((t) => !t);
      }}
    >
      {props.children}
    </div>
  );
}

function SiblingNav(props: { node: NodeView }) {
  const sib = () => siblingsOf(store.index(), props.node);
  return (
    <Show when={sib().list.length > 1}>
      <span class="sibling-nav">
        <button
          type="button"
          aria-label="上一个分支"
          title="上一个分支"
          disabled={sib().index === 0}
          onClick={() => void store.moveLeaf(sib().list[sib().index - 1].id, true)}
        >
          ‹
        </button>
        <span aria-label={`分支 ${sib().index + 1}，共 ${sib().list.length} 个`}>
          {sib().index + 1}/{sib().list.length}
        </span>
        <button
          type="button"
          aria-label="下一个分支"
          title="下一个分支"
          disabled={sib().index === sib().list.length - 1}
          onClick={() => void store.moveLeaf(sib().list[sib().index + 1].id, true)}
        >
          ›
        </button>
      </span>
    </Show>
  );
}

export function UserMessage(props: { node: NodeView }) {
  const edit = () => {
    store.setEditing({ nodeId: props.node.id });
    store.setComposerText(props.node.content);
    store.setComposerAttachments(props.node.attachments);
    document.querySelector<HTMLElement>(".composer-editor")?.focus();
  };
  return (
    <MessageShell class="msg-user">
      <div class="message-avatar"><Avatar name={store.user()?.username ?? "用户"} url={store.user()?.avatar} /></div>
      <div class="message-content"><div class="bubble">
        <Show when={props.node.content}><div class="bubble-text">{props.node.content}</div></Show>
        <Show when={props.node.attachments.length > 0}>
          <div class="message-attachments"><For each={props.node.attachments}>{(attachment) => (
            <Show when={attachment.kind === "image"} fallback={
              <a class="document-card" href={attachment.url} download={attachment.name}><span>{attachment.name}</span><small>{formatFileSize(attachment.size)} · 下载</small></a>
            }>
              <a class="message-image" href={attachment.url} target="_blank" rel="noopener noreferrer" title={attachment.name}>
                <img src={attachment.url} alt={attachment.name} loading="lazy" decoding="async" />
              </a>
            </Show>
          )}</For></div>
        </Show>
      </div>
      <div class="msg-actions">
        <SiblingNav node={props.node} />
        <button type="button" class="act hover-only" onClick={edit}>
          ✎ 编辑
        </button>
      </div>
      </div>
    </MessageShell>
  );
}

function formatMs(ms: number): string {
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(2)} s`;
}

function DebugLine(props: { metrics: Metrics | null; status: NodeStatus }) {
  return (
    <div class="debug">
      <Show when={props.metrics}>
        {(m) => (
          <>
            <Show when={m().ttft_ms !== null}>
              <span>
                首字 <b class="c-green">{formatMs(m().ttft_ms!)}</b>
              </span>
            </Show>
            <Show when={m().tokens_per_sec !== null}>
              <span>
                速率 <b class="c-green">{m().tokens_per_sec!.toFixed(1)} tok/s</b>
              </span>
            </Show>
            <span>
              缓存输入 <b class="c-teal">{m().usage.cached_in}</b>
            </span>
            <span>
              无缓存输入 <b class="c-blue">{m().usage.uncached_in}</b>
            </span>
            <Show when={m().usage.reasoning > 0}>
              <span>
                思考 <b class="c-purple">{m().usage.reasoning}</b>
              </span>
            </Show>
            <span>
              输出 <b class="c-yellow">{m().usage.output}</b>
            </span>
          </>
        )}
      </Show>
      <Show when={props.status === "interrupted"}>
        <span class="tag tag-warn">已中断</span>
      </Show>
      <Show when={props.status === "error"}>
        <span class="tag tag-error">出错</span>
      </Show>
    </div>
  );
}

function seconds(ms: number): number {
  return Math.max(1, Math.round(ms / 1000));
}

/** Collapsed reasoning: "▸ 思考了 N 秒"; opens to the full text when there is any. */
function Reasoning(props: { text: Accessor<string>; label: Accessor<string> }) {
  const [open, setOpen] = createSignal(false);
  return (
    <div class="reasoning" classList={{ open: open() }}>
      <button
        type="button"
        class="reasoning-toggle"
        aria-expanded={open()}
        disabled={!props.text()}
        onClick={() => setOpen((o) => !o)}
      >
        <span class="caret">{open() ? "▾" : "▸"}</span> {props.label()}
      </button>
      <Show when={open() && props.text()}>
        <div class="reasoning-text">{props.text()}</div>
      </Show>
    </div>
  );
}

export function AssistantMessage(props: { node: NodeView }) {
  const n = () => props.node;
  const reasoningMs = () => n().metrics?.reasoning_ms ?? null;
  const label = () => {
    const ms = reasoningMs();
    return ms !== null ? `思考了 ${seconds(ms)} 秒` : "思考过程";
  };
  const regenerate = () => {
    if (n().parent_id) void store.runTurn({ action: "regenerate", node_id: n().parent_id! });
  };
  return (
    <MessageShell class="msg-assistant">
      <div class="message-avatar"><Avatar name={n().character_name ?? "助手"} url={n().character_avatar} /></div>
      <div class="message-content">
      <Show when={n().reasoning || reasoningMs() !== null}>
        <Reasoning text={() => n().reasoning ?? ""} label={label} />
      </Show>
      <Markdown id={n().id} text={() => n().content} final={() => true} />
      <Show when={store.nodeErrors()[n().id]}>
        {(msg) => <div class="msg-error" role="alert">{msg()}</div>}
      </Show>
      <DebugLine metrics={n().metrics} status={n().status} />
      <div class="msg-actions">
        <SiblingNav node={n()} />
        <button type="button" class="act hover-only" onClick={regenerate}>
          ↻ 重新生成
        </button>
      </div>
      </div>
    </MessageShell>
  );
}

export function LiveMessage(props: { turn: LiveTurn }) {
  const t = props.turn;
  // Ticks once a second while thinking so the counter moves without per-frame work.
  const [now, setNow] = createSignal(performance.now());
  const timer = setInterval(() => setNow(performance.now()), 1000);
  onCleanup(() => clearInterval(timer));
  const label = () => {
    const start = t.reasoningStart();
    if (start === null) return "";
    const end = t.reasoningEnd();
    return end === null ? `思考中 ${seconds(Math.max(now(), start) - start)} 秒` : `思考了 ${seconds(end - start)} 秒`;
  };
  const generating = () => t.error() === null;
  const status = () => {
    if (t.stopping()) return "正在停止…";
    switch (t.connectionState()) {
      case "connecting": return "正在连接…";
      case "reconnecting": return "连接已断开，正在恢复…";
      case "streaming": return "思考中…";
    }
  };
  return (
    <MessageShell class="msg-assistant">
      <div class="message-avatar"><Avatar name={t.characterName() || "助手"} url={t.characterAvatar()} /></div>
      <div class="message-content">
      <Show when={t.reasoningStart() !== null}>
        <Reasoning text={t.reasoning} label={label} />
      </Show>
      <Show when={t.assistantId()} fallback={<div class="md-plain">{t.content()}</div>}>
        <Markdown id={t.assistantId()} text={t.content} final={() => !generating()} />
      </Show>
      <Show when={t.error()}>{(msg) => <div class="msg-error" role="alert">{msg()}</div>}</Show>
      <div class="debug">
        <Show when={generating()}>
          <span class="generating" data-state={t.stopping() ? "stopping" : t.connectionState()}>{status()}</span>
        </Show>
      </div>
      </div>
    </MessageShell>
  );
}
