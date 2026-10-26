# 计划 2：网页聊天前端 —— 实现约定

上位文档：`docs/superpowers/specs/2026-10-26-nekochat-design.md`（第 6、7、9 节为主）。用户要求快速实现：不写逐任务计划，不做逐任务审查，最后整体验证一次。本文件只定两边必须对齐的接口与归属。

## 归属（两个代理并行，各自只改、只提交自己的路径）

| 归属 | 路径 |
|---|---|
| Rust 侧 | `crates/mdwasm/**`、`crates/server/**`、`crates/core/**`、根 `Cargo.toml` / `Cargo.lock`、`web/scripts/build-wasm.sh` |
| 前端 | `web/**`（`web/scripts/build-wasm.sh` 除外） |
| 控制者 | `README.md`、根 `.gitignore`、本文件 |

提交时只 `git add` 自己的路径；遇到 `index.lock` 稍等重试。不要跑别人的长任务，不要占用 10000 端口（用户自己的服务在用）；开发用 10010 及以上端口。端口 3000 属于用户的其他进程，不要碰。

## 1. Markdown WASM（Rust 侧）

crate `crates/mdwasm`，包名 `nekochat-mdwasm`，`crate-type = ["cdylib", "rlib"]`，依赖 `pulldown-cmark 0.13`、`wasm-bindgen = "=0.2.129"`（必须与本机 CLI 版本一致）。

`web/scripts/build-wasm.sh`：`cargo build -p nekochat-mdwasm --release --target wasm32-unknown-unknown`，再 `wasm-bindgen --target web --out-dir web/src/wasm target/wasm32-unknown-unknown/release/nekochat_mdwasm.wasm`。产物目录 `web/src/wasm/`（gitignored）。

导出（wasm-bindgen）：

- `split_blocks(md: &str) -> Vec<String>`：按**顶层块**切分（段落、标题、列表整体、代码围栏、表格、引用、display 数学……），返回各块的原文；块之间的空行不重要。最后一块在流式输出时可能未完成。未闭合的代码围栏视为一直延伸到结尾的一个块。
- `render_html(md: &str) -> String`：把一段 Markdown 渲染为**安全** HTML：
  - 扩展：表格、删除线、任务列表、数学（`$…$`、`$$…$$`）。渲染前把代码之外的 `\(…\)` 转成 `$…$`、`\[…\]` 转成 `$$…$$`（LLM 常用写法）。
  - 原始 HTML（块级与行内）一律按文本转义输出，绝不透传。
  - 链接只允许 `http:`、`https:`、`mailto:`、以 `#` 或 `/` 开头的相对地址；其他协议（`javascript:`、`data:`、`vbscript:` 等）的链接输出为纯文本。链接加 `target="_blank" rel="noopener noreferrer nofollow"`。
  - 图片只允许 `http:` / `https:`，否则输出 alt 文本；`<img loading="lazy" decoding="async">`。
  - 数学占位：行内 `<span class="nk-math" data-display="0">转义后的TeX</span>`，独立 `<div class="nk-math" data-display="1">转义后的TeX</div>`。
  - 代码：`<pre><code class="language-xxx">转义后的代码</code></pre>`（无语言时无 class）。
- Rust 单元测试（宿主编译即可）：脚本注入被转义、`javascript:` 链接被拒、`\[…\]` 转换（且代码块内不转换）、未闭合围栏的切块。

## 2. 服务端补充（Rust 侧）

1. `nekochat_core::Metrics` 增加 `reasoning_ms: Option<u64>`（`#[serde(default)]`，兼容旧数据）= 首个正文增量时刻 − 首个思考增量时刻；有思考但没有正文时 = 最后一个增量 − 首个思考增量；没有思考时为 `None`。`MetricsView` 暴露同名字段。
2. 静态资源：用 `rust-embed` 嵌入 `web/dist`（目录不存在时也必须能编译，例如 `allow_missing`）。非 `/api` 开头的 GET：有同名文件就返回（`mime_guess` 定 Content-Type），否则返回 `index.html`（SPA 回退）；`/assets/*` 加 `Cache-Control: public, max-age=31536000, immutable`，`index.html` 加 `Cache-Control: no-cache`。`dist` 不存在时根路径返回 404 JSON `{"error":"前端未构建：先运行 web 构建"}`。Host 校验照旧在最外层；访问密码只管 `/api/*`，所以页面本身总能打开。
3. 自定义主题：`GET /api/themes` → `[{"id","name","url"}]`，来自 `<data_dir>/themes/*.css`（`id` = `name` = 文件名去掉 `.css`，按名称排序，目录不存在返回 `[]`）；`GET /themes/{file}` 只接受 `^[A-Za-z0-9_-]+\.css$`，返回 `text/css`，其余 404 JSON。`/api/themes` 受访问密码保护。
4. 测试：SPA 回退、`/assets` 缓存头、主题列表、主题路径穿越被拒。

wasm 产物构建出来后，Rust 侧必须立刻 `write agent://Frontend` 通知前端（附上 `web/src/wasm/` 里的文件名与导出函数签名）。

## 3. 前端（前端代理）

`web/`：SolidJS + Vite + TypeScript，包管理用 bun。`bun run build` = 先跑 `scripts/build-wasm.sh` 再 `vite build`，输出 `web/dist`。开发时 Vite 代理 `/api` 与 `/themes` 到本地服务端。

后端接口以计划 1 的代码为准：`crates/server/src/lib.rs`（路由）、`api/user.rs`（`/api/models`、对话 CRUD、`PUT …/leaf`、`NodeView`/`MetricsView`）、`chat.rs`（`POST …/turns` 请求体与 SSE 事件 `start`/`reasoning`/`text`/`done`/`error`）、`auth.rs`（`POST /api/session`）。所有错误体都是 `{"error": "<中文>"}`。

必须实现（设计文档第 6、7.2、9 节）：

- 布局 A：左侧栏（「＋ 新对话」、对话列表、悬停出现 🗑 彻底删除，确认弹窗文案见 6.6）；顶栏对话标题 + 右上角主题切换（按 Catppuccin / Gruvbox / 其他 / 自定义 分组，每项三个色点）；对话区；输入框。窄屏侧栏收成抽屉，悬停操作在触屏改为点按消息后出现。
- 用户端看不到任何角色信息。
- 回复上方灰字 `model_label`；思考默认折叠为「▸ 思考了 N 秒」（N 取 `metrics.reasoning_ms`，流式时实时计时），点开看全文。
- debug 行：首字、速率、缓存输入、无缓存输入、思考（>0 时）、输出；等宽小字，各自颜色；`interrupted` / `error` 状态要标出。SSE `error` 的 `message` 显示在该条回复处。
- 输入框平时只显示「输入消息…」；输入 `/` 才弹出命令，Tab 补全、↑↓ 选择。本计划只提供后端已支持的 `/clear`（确认后调用 `POST …/clear`）与 `/tree`；`/compact`、`/upload` 属于计划 5，**不要显示**。
- 模型选择器（按提供商分组，`label`、档位小字）+ 思考强度选择器（`levels` 为空时不显示；换模型落到该模型默认档）；上次选择记在 localStorage，否则用 `default_model_id`。
- 对话树：根据 `nodes` 与 `leaf_id` 计算当前路径与兄弟；有兄弟的消息显示 `‹ i/n ›`（切换调用 `PUT …/leaf {node_id, descend:true}`）；用户消息「✎ 编辑」→ `action:"edit"`；回复「↻ 重新生成」→ `action:"regenerate"`（`node_id` 为其父用户节点）。`/tree` 面板：树状列出全部节点、当前位置高亮、↑↓ + Enter，也支持点选；选用户消息 → 文本放回输入框并进入编辑模式（发送即 `edit`）；选回复 → `PUT …/leaf {node_id, descend:false}`。生成中切换返回 409 时给出提示。
- 访问密码：任何 `/api` 请求得到 401 时显示口令页，`POST /api/session` 后重试。
- 主题：全部颜色走 CSS 变量；内置 Catppuccin Latte / Frappé / Macchiato / Mocha（官方色板）、Gruvbox Dark / Light（官方色板）、Claude 娘（浅底 `#faf9f5`、主色 `#d97757`、正文 `#141413` 为起点）。主题作用于 `:root[data-theme="<id>"]`；自定义主题来自 `/api/themes`，按 `url` 以 `<link>` 加载。选择记在 localStorage。前端在 `web/THEMES.md` 写清变量名清单与自定义主题写法。
- 性能（硬性）：流式 token 进缓冲，每个 `requestAnimationFrame` 最多更新一次 DOM；Markdown 在 Web Worker 里用上面的 wasm 做：已完成块渲染一次后冻结，只重渲最后一块；KaTeX（`katex.renderToString`，`throwOnError:false`）与代码高亮（highlight.js core + 常用语言）也在 Worker 里，对占位元素的内容先反转义再处理；长对话虚拟滚动（变高消息）；不做打字机动画。生成中用户上滚时不强制滚到底。
- 验收（设计 9.6）：模拟 100 tok/s 流式输出、已有 500 条消息（含公式、代码）的对话，主线程无 >50 ms 长任务，滚动 60 fps。前端提供一个仅开发用的压测入口（例如 `?bench=1`，不进生产路由也可）用于这项测量。
