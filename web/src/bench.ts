// Dev-only benchmark (`?bench=1` on the Vite dev server): a 500-message conversation with math
// and code, a reply streamed at 100 tok/s through the real streaming path, and a scroll sweep.
// Reports the longest main-thread long task and frame timing; the result is also on
// `window.__benchResult`.
import type { NodeView } from "./api";
import { store } from "./store";

const MESSAGES = 500;
const TOKENS_PER_SEC = 100;
const STREAM_SECONDS = 20;

const REPLY = String.raw`下面推导一下二次方程的求根公式。对 $ax^2+bx+c=0$ 配方：

$$x = \frac{-b \pm \sqrt{b^2-4ac}}{2a}$$

判别式 $\Delta = b^2 - 4ac$ 决定根的个数，欧拉公式 \(e^{i\pi}+1=0\) 也顺带提一下。

\[
\int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}
\]

| 情况 | 根 |
|---|---|
| $\Delta>0$ | 两个实根 |
| $\Delta=0$ | 一个重根 |

用 Rust 写出来：

${"```"}rust
fn roots(a: f64, b: f64, c: f64) -> Option<(f64, f64)> {
    let d = b * b - 4.0 * a * c;
    if d < 0.0 {
        return None;
    }
    let s = d.sqrt();
    Some(((-b + s) / (2.0 * a), (-b - s) / (2.0 * a)))
}
${"```"}

- **要点一**：先判断 $a \ne 0$；
- **要点二**：浮点误差下用 $q = -\tfrac12(b + \operatorname{sgn}(b)\sqrt{\Delta})$ 更稳定。

`;

function fakeConversation(): { nodes: NodeView[]; leaf: string } {
  const nodes: NodeView[] = [];
  let parent: string | null = null;
  const t0 = Date.now() - MESSAGES * 60_000;
  for (let i = 0; i < MESSAGES; i++) {
    const user = i % 2 === 0;
    const id = `bench-${i}`;
    nodes.push({
      id,
      parent_id: parent,
      role: user ? "user" : "assistant",
      attachments: [],
      content: user ? `第 ${i / 2 + 1} 个问题：求根公式怎么推？顺便给段代码。` : REPLY + (i % 10 === 1 ? "\n![猫猫搓手](/api/resources/nekowhale/4a44767ab58b.gif)\n" : i % 10 === 3 ? "\n![喝药](/api/resources/nekowhale-gpt/222876e49285.gif)\n" : ""),
      reasoning: user ? null : "先配方，再讨论判别式。",
      character_id: user ? null : "bench-character",
      character_name: user ? null : "本地角色",
      character_avatar: null,
      status: "complete",
      metrics: user
        ? null
        : {
            ttft_ms: 420,
            total_ms: 9000,
            tokens_per_sec: 98.5,
            reasoning_ms: 1200,
            usage: { cached_in: 1024, uncached_in: 87, reasoning: 120, output: 640 },
          },
      created_at: t0 + i * 60_000,
    });
    parent = id;
  }
  return { nodes, leaf: parent! };
}

function report(text: string) {
  let el = document.getElementById("bench-report");
  if (!el) {
    el = document.createElement("pre");
    el.id = "bench-report";
    el.style.cssText =
      "position:fixed;right:8px;top:48px;z-index:99;margin:0;padding:8px 10px;font:12px monospace;" +
      "background:var(--nk-bg-deep);color:var(--nk-text);border:1px solid var(--nk-border);border-radius:8px";
    document.body.appendChild(el);
  }
  el.textContent = text;
}

/** Frame intervals (ms) while `run` lasts. */
async function frames(run: Promise<void>): Promise<number[]> {
  const gaps: number[] = [];
  let last = performance.now();
  let running = true;
  const tick = (t: number) => {
    gaps.push(t - last);
    last = t;
    if (running) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
  await run;
  running = false;
  return gaps;
}

function frameStats(gaps: number[]) {
  const sorted = [...gaps].sort((a, b) => a - b);
  const total = gaps.reduce((a, b) => a + b, 0);
  return {
    fps: Math.round((gaps.length / total) * 1000 * 10) / 10,
    p95: Math.round(sorted[Math.floor(sorted.length * 0.95)] * 10) / 10,
    worst: Math.round(sorted[sorted.length - 1] * 10) / 10,
    over33: gaps.filter((g) => g > 33.4).length,
  };
}

declare global {
  interface Window {
    __benchResult?: unknown;
  }
}

function sleep(ms: number): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  setTimeout(resolve, ms);
  return promise;
}

function nextFrame(): Promise<number> {
  const { promise, resolve } = Promise.withResolvers<number>();
  requestAnimationFrame(resolve);
  return promise;
}

export async function runBench(): Promise<void> {
  const { nodes, leaf } = fakeConversation();
  const convId = "bench";
  store.setConv({ id: convId, title: "压测：500 条消息", character_id: "bench-character", leaf_id: leaf, nodes });
  report("预热中…");
  await sleep(2500);

  const longTasks: number[] = [];
  const observer = new PerformanceObserver((list) => {
    for (const e of list.getEntries()) longTasks.push(e.duration);
  });
  observer.observe({ type: "longtask" });

  // Stream a reply at 100 tok/s (about 2 characters per token for mixed Chinese/TeX/code).
  const userNode: NodeView = {
    ...nodes[0],
    id: "bench-user",
    parent_id: leaf,
    content: "再来一遍，长一点。",
    created_at: Date.now(),
  };
  const turn = store.createLiveTurn(convId, null);
  store.setLive((l) => ({ ...l, [convId]: turn }));
  store.handleTurnEvent(turn, {
    type: "start",
    data: { user_node: userNode, assistant_node_id: "bench-live", character_id: "bench-character", character_name: "本地角色", character_avatar: null },
  });
  const source = "![猫猫搓手](/api/resources/nekowhale/4a44767ab58b.gif)\n\n" + REPLY.repeat(40);
  let pos = 0;
  const streaming = (async () => {
    const end = performance.now() + STREAM_SECONDS * 1000;
    let next = performance.now();
    while (performance.now() < end) {
      next += 1000 / TOKENS_PER_SEC;
      const t = source.slice(pos, pos + 2);
      pos += 2;
      store.handleTurnEvent(turn, { type: "text", data: { t } });
      await sleep(Math.max(0, next - performance.now()));
    }
  })();

  // First half: follow the bottom while streaming. Second half: sweep scrolling up the history.
  const followGaps = await frames(sleep((STREAM_SECONDS / 2) * 1000));
  const scroller = document.querySelector<HTMLElement>(".messages")!;
  const sweep = (async () => {
    const end = performance.now() + (STREAM_SECONDS / 2 - 1) * 1000;
    // Mark the scroll as the user's so following stops, like a real wheel scroll.
    scroller.dispatchEvent(new WheelEvent("wheel", { deltaY: -100, bubbles: true }));
    while (performance.now() < end) {
      scroller.scrollTop -= 60;
      await nextFrame();
    }
  })();
  const scrollGaps = await frames(sweep);
  await streaming;
  observer.disconnect();

  const result = {
    messages: MESSAGES,
    streamedChars: pos,
    longTasks: longTasks.length,
    maxLongTaskMs: Math.round(Math.max(0, ...longTasks)),
    follow: frameStats(followGaps),
    scroll: frameStats(scrollGaps),
  };
  window.__benchResult = result;
  report(JSON.stringify(result, null, 2));
}
