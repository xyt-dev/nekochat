import { type Accessor, ErrorBoundary, Index, lazy, Match, Show, Suspense, Switch } from "solid-js";
import { createMarkdown } from "../md/client";
import type { MdBlock } from "../md/protocol";

const GraphicBlock = lazy(() => import("../graphics/GraphicBlock"));

/** The Index keeps message block elements stable while the last block streams. */
export function Markdown(props: { id: string; text: Accessor<string>; final: Accessor<boolean> }) {
  const blocks = createMarkdown(() => props.id, props.text, props.final);
  return (
    <div class="md">
      <Show when={blocks()} fallback={<div class="md-plain">{props.text()}</div>}>
        {(ready) => <Index each={ready()}>{(block, index) => <div class="md-block">
          <Switch>
            <Match when={block().kind === "html"}>
              <div innerHTML={(block() as Extract<MdBlock, { kind: "html" }>).html} />
            </Match>
            <Match when={block().kind === "graphic-pending"}>
              <div class="md-graphic-pending" role="status">正在生成图形…</div>
            </Match>
            <Match when={block().kind === "graphic-error"}>
              <div class="md-graphic-error" role="alert">图形无法显示：{(block() as Extract<MdBlock, { kind: "graphic-error" }>).error}</div>
              <details><summary>查看图形源码</summary><pre><code>{(block() as Extract<MdBlock, { kind: "graphic-error" }>).source}</code></pre></details>
            </Match>
            <Match when={block().kind === "graphic"}>
              <ErrorBoundary fallback={(err) => <div class="md-graphic-error" role="alert">图形显示失败：{String(err)}</div>}>
                <Suspense fallback={<div class="md-graphic-pending" role="status">正在载入图形…</div>}>
                  <GraphicBlock id={`${props.id}:${index}`} spec={(block() as Extract<MdBlock, { kind: "graphic" }>).spec} source={(block() as Extract<MdBlock, { kind: "graphic" }>).source} />
                </Suspense>
              </ErrorBoundary>
            </Match>
          </Switch>
        </div>}</Index>}
      </Show>
    </div>
  );
}
