# 🐾 nekochat

[简体中文](README.md) | [English](README.en.md)

THIS IS LARGE SLOP MACHINE 的无害垃圾。

拿个大模型开发还真把你当工程师了？？？？

**专注于角色沉浸感、高可玩性卡片配置与零卡顿流式体验的自托管角色扮演网页聊天。**

📦 源码仓库：[xyt-dev/nekochat](https://github.com/xyt-dev/nekochat)（默认分支 `main`） · 🏷️ 版本发布：[Releases](https://github.com/xyt-dev/nekochat/releases) · 📖 运维与接口文档：[docs/guide.md](docs/guide.md) · 📝 变更记录：[CHANGELOG.md](CHANGELOG.md)

`nekochat` 采用 **「管理端全量编排 + 聊天端纯粹交互」** 的设计：管理员在 `/admin` 将底层模型、思考档位、系统提示词、联网搜索服务、立绘背景与本地表情包组合成完整的「角色卡」；普通聊天页面完全隐藏模型名称、提供商、参数滑块与账单来源，打开即可直接与角色对话。

---

## ✨ 聊天界面实拍（Claude Dark）

内置「大肥鱼」「大肥鱼和 GPT 娘」两套共 **322 张本地 GIF 表情包**。角色会根据语境与情绪在流式回复中自然穿插表情，配合立绘背景与首字/速率指标展示，桌面端与移动端均保持干净紧凑的交互体验：

<table>
  <tr>
    <td width="68%" valign="top">
      <img src="docs/screenshots/nekowhale-chat-desktop.webp" alt="Claude Dark 桌面端大肥鱼酱带表情回复实拍" />
      <p align="center"><sub>🖥️ 桌面端（Claude Dark，演示账号 Lina）：左侧切换角色卡，搭配立绘背景，揉脸扯尾巴互动下实时渲染本地表情包</sub></p>
    </td>
    <td width="32%" valign="top">
      <img src="docs/screenshots/nekowhale-chat-mobile.webp" alt="Claude Dark 移动端大肥鱼酱带表情回复实拍" />
      <p align="center"><sub>📱 移动端（412px 窄屏，演示账号 Lina）：保留完整立绘，消息气泡与表情自适应</sub></p>
    </td>
  </tr>
</table>

---

## 🎭 角色卡片制作与 🔍 搜索对接：直观方便，可玩性拉满

在 `/admin` 管理端，创建和调教一个角色不需要写复杂的配置文件，所有能力都做成了可视化开关与实时预览，几分钟就能搭出性格鲜明、带专属技能的角色卡：

### 🛠️ 一站式角色卡片工作台

- **模型与思考深度独立绑定**：每张角色卡可自由绑定 DeepSeek、OpenCode Go 或官方 ChatGPT 订阅（SIWC）下的任意上架模型，并单独固定思考档位（`none` / `low` / `high` / `max`）与采样参数。
- **立绘背景与实时预览**：支持上传 PNG / JPEG / WebP / GIF / AVIF 背景图（至多 5 MiB），可调节透明度并实时预览；图片等比完整显示，适配桌面与手机。
- **本地表情包与自定义 Skill 挂载**：一键勾选内置的 `大肥鱼（286 张）`、`大肥鱼和 GPT 娘（36 张）` 或自定义 `skills/` 资源包，自由调节表情身份（`角色本人 self` / `中立引用 neutral`）与发送频率（`偶尔` / `适中` / `经常`）。

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/screenshots/character-editor-dark.webp" alt="角色卡片制作：模型、搜索服务、头像、提示词与思考档位" />
      <p align="center"><sub>🎛️ 角色卡片上半区：绑定底层模型、专属联网搜索服务、角色头像、系统提示词与服务端思考档位</sub></p>
    </td>
    <td width="50%" valign="top">
      <img src="docs/screenshots/character-assets-dark.webp" alt="角色卡片制作：立绘背景实时预览与表情资源包挂载" />
      <p align="center"><sub>🖼️ 角色卡片下半区：立绘背景与透明度预览、322 张表情包勾选与表情频率控制</sub></p>
    </td>
  </tr>
</table>

### 🌐 零门槛联网搜索对接

- **双引擎开箱即用**：内置独立搜索底座 `nekochat-search`，支持对接 **DeepSeek 官方搜索 + 网页获取**，以及 **GPT 原生 Web Search + 本地受限网页正文提取**。
- **直接复用本站 ChatGPT 订阅**：已在「模型管理」登录过官方 ChatGPT 账号（Plus / Pro）后，新建 GPT 搜索服务时可直接勾选复用同一份订阅，**无需二次登录，也不会偷偷消耗 API Key 余额**。
- **按角色精细授权 + 独立用量折线图**：在角色卡片上按需选择是否挂载搜索服务；服务端在单次回复内自动完成多轮 `模型 → web_search / web_fetch → 模型` 闭环，并提供独立于对话账单的 15 分钟粒度调用与 Token 折线图。

<p align="center">
  <img src="docs/screenshots/search-services-dark.webp" width="92%" alt="搜索服务目录与独立搜索用量折线图" />
  <br />
  <sub>🔍 搜索服务管理：支持 DeepSeek API 与本站 ChatGPT 订阅双通道，自带不计费连接测试、真实搜索测试与分服务折线统计</sub>
</p>

---

## ⚡ 架构设计：为什么流式生成与长对话绝不会卡

`nekochat` 从服务端状态机、浏览器渲染管线、静态资源分发到数据库与子进程隔离，核心目标只有一个：**无论流式输出多快、历史消息多长、或者网络与外部搜索多慢，主界面输入、滚动与生成恢复始终保持零卡顿。**

```mermaid
flowchart LR
  subgraph Browser["🖥️ 浏览器前端 · SolidJS"]
    UI["虚拟消息列表<br/>@tanstack/solid-virtual"]
    Worker["Web Worker 增量渲染线程<br/>nekochat-mdwasm + KaTeX + hljs"]
    UI <-->|"仅重算末尾生长块"| Worker
  end

  subgraph Server["⚙️ 主服务进程 · nekochat (Rust / Axum)"]
    Gen["Generation 核心状态机<br/>独立 Tokio 任务 · 快照 + Broadcast"]
    Res["nekochat-resources<br/>322 张内置 GIF · Bytes 零拷贝"]
    Store["nekochat-store<br/>SQLite WAL · spawn_blocking 隔离"]
    Gw["nekochat-gateway<br/>Chat Completions / Responses / SIWC"]
    Gen --> Res
    Gen --> Store
    Gen --> Gw
  end

  subgraph Search["🔍 独立搜索子进程 · nekochat-search"]
    SProc["私有管道就绪握手<br/>连接层公网 DNS 锁死 Reader"]
  end

  UI <-->|"SSE 观察流 / 修订号快照重连"| Gen
  Gen <-->|"多轮工具调用 · 短期令牌委托"| SProc
  Gw <-->|"流式推理 / OAuth PKCE"| Upstream["DeepSeek / OpenCode / OpenAI"]
```

### 1. 🧵 前端主线程零解析开销：Web Worker + Rust WASM 增量块管线

许多网页聊天在流式输出时每收到一个 token 就在主线程全量重跑 Markdown、LaTeX 公式和代码高亮，回复越长界面越卡。`nekochat` 将排版计算彻底移出主线程（`crates/mdwasm` + `web/src/md/md.worker.ts`）：

- **Rust WASM 块级切分**：`pulldown-cmark` 编译为 WebAssembly（`nekochat_mdwasm`），在独立 Web Worker 内将流式文本切分为顶层 Markdown 块。
- **已封口段落缓存复用**：Worker 按消息 `key` 缓存已完成块的文本与 HTML；流式追加期间**只重新渲染末尾正在生长的那一个块**，已完成的上下文段落零重复计算。
- **重型渲染延后封口**：代码块语法高亮（`highlight.js`）与数学公式排版（`KaTeX`）仅在段落闭合时于 Worker 内一次性填充占位符，不在逐 token 热路径中反复执行。
- **虚拟列表与细粒度 DOM 更新**：主线程使用 SolidJS 细粒度响应式配合 `@tanstack/solid-virtual` 虚拟滚动，仅挂载可视区域消息节点，也不使用逐 token DOM 测量动画，打字与滚动始终满帧。

### 2. 🔄 服务端生成与慢客户端彻底解耦：Core 独占 `Generation` 状态机

在 `crates/server/src/generation.rs` 中，上游模型流由服务端 Tokio 任务独立持有，浏览器连接只是订阅 `broadcast` 通道的**观察者（Observer）**：

- **零反向阻塞（No Backpressure Stall）**：观察通道采用有界缓冲；某个客户端网络卡顿、切到后台或断开连接时，绝不会反压阻塞上游模型读取，也不会拖慢同一会话的其他观察端。
- **F5 刷新与断线毫秒级快照恢复**：刷新网页或切换设备后，前端通过 `GET /api/conversations/{id}/turns` 订阅现有生成，服务端先返回带单调递增 `revision` 的内存快照（`snapshot`），再无缝衔接后续增量事件，不重发模型请求、不丢字。
- **流式数据内存聚合，非逐 Token 刷盘**：流式 token 与思考片段在内存状态机内聚合，仅在生成启动与结束（完成 / 中断 / 出错）时写入 SQLite，避免高频磁盘 I/O 抖动。

### 3. 🖼️ 322 张内置 GIF 零拷贝分发与流式标签状态机

- **静态字节零拷贝（`Bytes`）**：`crates/resources` 在编译期嵌入内置表情包，运行时通过引用计数 `Bytes` 直接构造 HTTP 响应并附带 `Cache-Control: private, max-age=86400`，不进前端 JS 包、不复制整块图片缓冲，也不依赖外部 CDN。
- **O(1) 流式 `[sticker:...]` 解析**：服务端状态机在文本流经过时实时匹配并替换已授权表情标签，无全文正则回溯，且半截未闭合标签不会泄漏到前端造成界面闪烁。

### 4. 🗄️ SQLite WAL 并发读写与独立搜索进程隔离

- **存储层阻塞隔离**：`crates/store` 开启 `PRAGMA journal_mode = WAL` 与 `busy_timeout = 5000ms`，所有 SQLite 操作均通过 `tokio::task::spawn_blocking` 运行在专用阻塞线程池，绝不占用 Tokio 异步网络工作线程。
- **搜索与正文解析子进程隔离**：联网搜索与 HTML 清洗运行在独立的 `nekochat-search` 子进程中，通过父子私有管道（`--ready-fd`）完成启动就绪握手；外网慢请求或大网页解析完全与主聊天进程隔离，且内置 Reader 在连接层锁定公网 DNS，杜绝 SSRF 与 DNS Rebinding。

---

## 🚀 Quickstart

无需容器，在一台装有 **Rust (1.85+)** 与 **Bun** 的机器上即可直接编译并启动 Web 与内置搜索底座。

### 1. 安装前端 WASM 构建工具

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129
```

### 2. 初始化配置文件与搜索令牌

复制 Web 与搜索底座的示例配置，并生成两枚互不相同的持久化搜索令牌写入 `.env.local`（已在 `.gitignore` 中忽略）：

```sh
cp config.example.toml config.toml
cp crates/search/config.example.toml crates/search/config.toml

cat > .env.local <<EOF
export NEKOCHAT_SEARCH_RUNTIME_TOKEN="$(openssl rand -hex 32)"
export NEKOCHAT_SEARCH_ADMIN_TOKEN="$(openssl rand -hex 32)"
EOF
chmod 600 .env.local
source .env.local
```

按你的模型接入方式调整 `config.toml`：

- **仅使用官方 ChatGPT 订阅（Plus / Pro）**：将 `config.toml` 中示例的 `[[providers]]` 段落注释或删除，设置 `admin_password = "你的管理密码"` 即可直接启动。
- **使用 DeepSeek / OpenCode Go API Key**：保留对应 `[[providers]]` 段落，并在环境中 `export DEEPSEEK_API_KEY=...` 或 `export OPENCODE_API_KEY=...`（配置中声明的提供商在启动时会校验对应环境变量非空）。

### 3. 构建前端与 Rust Release 二进制

```sh
cd web && bun install && bun run build && cd ..
cargo build --release -p nekochat-server -p nekochat-search
```

`web/dist` 会在 release 编译时嵌入 `nekochat` 二进制，无需额外部署静态文件目录。

### 4. 一键启动 Web 与搜索底座

```sh
source .env.local
./target/release/nekochat serve --config config.toml
```

`nekochat serve` 会先启动同目录的 `nekochat-search`，待其通过私有管道报告授权初始化与端口绑定完成后，再启动 Web 服务监听 `http://127.0.0.1:10000`。

### 5. 进入后台配置角色并开始对话

1. 打开 `http://127.0.0.1:10000/admin`，使用 `admin_password` 登录。
2. 在 **模型管理** 连接 **官方 ChatGPT 账号（Sign in with ChatGPT）** 或刷新 API 提供商模型目录，将需要的模型点「上架」。
3. 在 **搜索服务**（可选）填入 `.env.local` 中的两枚令牌保存底座连接，新建 DeepSeek 或复用本站 ChatGPT 订阅的搜索服务。
4. 在 **角色管理** 创建角色：绑定上架模型、勾选 `nekowhale` / `nekowhale-gpt` 表情包、按需挂载搜索服务、上传头像与立绘背景，并设为默认角色。
5. 返回 `http://127.0.0.1:10000/` 注册聊天账号即可开始对话。

---

## 🧩 其他核心特性

| 模块 | 功能说明 |
| :--- | :--- |
| **🌳 完整 SQLite 对话树** | 编辑任意历史用户消息或对同一轮回复点「重新生成」都会在树上创建新分支；支持通过 `/tree` 或分支按钮随时在平行分支间切换。 |
| **📎 图片与文档真实提取** | 支持粘贴、拖拽或 `/upload` 上传 PNG / JPEG / WebP / GIF 图片，以及 PDF、Markdown、文本与代码文件。PDF 与文本在服务端真实提取内容后送入用户消息，加密或无字扫描 PDF 明确报错，绝不拿文件名充当正文。 |
| **🔐 官方 ChatGPT 订阅 (SIWC)** | 原生实现 OpenAI 官方 Sign in with ChatGPT 动态客户端注册与 OAuth PKCE；每个账号/工作区独立保存 `client_id` 与稳定 `ext_agent_host_id`，支持退出后保留注册映射重连及清理已退出本地记录。 |
| **🎨 8 套内置主题 + 自定义 CSS** | 内置 Claude（Dark / Light）、Catppuccin（Latte / Frappé / Macchiato / Mocha）、Gruvbox（Dark / Light）8 套主题；在 `<data_dir>/themes/<name>.css` 放入自定义样式即可在主题菜单中切换（详见 [`web/THEMES.md`](web/THEMES.md)）。 |
| **👥 多用户隔离与密码策略** | 对话、分支、附件与上次所选角色均按登录用户 ID 在服务端隔离。后台支持创建/停用用户、分配旧未归属对话，并可切换新建账号默认使用明文或 Argon2id 哈希存储。 |
| **📊 费用与滚动配额监控** | 按「提供商 + 模型」展示缓存输入、无缓存输入、思考与输出四类 Token 及历史单价费用；支持查询 DeepSeek 官方实时余额及估算 OpenCode Go 的 5h / 7d / 30d 滚动窗口用量。 |

### 📂 仓库目录结构

```text
nekochat/
├── crates/
│   ├── core/        # 角色、消息树、指标与生成参数核心数据结构
│   ├── store/       # SQLite (WAL) 存储、schema 迁移、用户与对话树持久化
│   ├── gateway/     # Chat Completions、标准 Responses 与官方 ChatGPT 订阅流式网关
│   ├── resources/   # 内置 322 张 GIF 嵌入、自定义资源包校验与流式 [sticker:...] 状态机
│   ├── server/      # Axum HTTP 服务、Generation 状态机、SIWC OAuth、搜索子进程托管
│   ├── mdwasm/      # 编译为 WebAssembly 的 pulldown-cmark 分块与 HTML 渲染器
│   └── search/      # 独立 nekochat-search 二进制：DeepSeek / GPT 搜索、反 SSRF Reader 与账本
├── web/             # SolidJS + TypeScript 前端、Web Worker 渲染管线与 8 套内置主题
├── resource-packs/  # 内置大肥鱼 / GPT 娘表情包清单、图片素材与原始许可说明
└── docs/            # 实拍截图 (docs/screenshots/) 与完整运维/API 指南 (docs/guide.md)
```

---

## 📖 进阶配置与接口手册

如需了解反向代理与 `allowed_hosts` 白名单、远程浏览器完成 ChatGPT OAuth 回调、独立部署 `nekochat-search`、编写自定义 `skills/` 资源包、数据库升级说明或完整 `curl` 冒烟脚本，请参阅：

👉 **[完整使用与运维指南 (`docs/guide.md`)](docs/guide.md)**

---

## 🙏 素材来源与致谢

- 内置「大肥鱼」「大肥鱼和 GPT 娘」表情包与标注整理自 [Loping151/dsh-nekowhale-stickers](https://github.com/Loping151/dsh-nekowhale-stickers)。
- 形象与表情包原作者为 **[可以叫我云大人 / 雾雪](https://space.bilibili.com/103739)**（[kiriyuki.com](https://kiriyuki.com)）。知识库遵循 CC BY-NC 4.0，图片素材使用条件见随仓保留的 [`resource-packs/LICENSE-ASSETS`](resource-packs/LICENSE-ASSETS)、[`resource-packs/LICENSE`](resource-packs/LICENSE) 与 [`resource-packs/NOTICE`](resource-packs/NOTICE)（非商业分享请保留原作者与网站署名，禁止用于收费或其他商业用途）。
