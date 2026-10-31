# 🐾 nekochat

[简体中文](README.md) | [English](README.en.md)

THIS IS HARMLESS TRASH FROM A LARGE SLOP MACHINE.

You used an LLM to build something and now you think you're an engineer????

**A self-hosted roleplay chat app with immersive characters, customizable character cards, and responsive streaming.**

📦 Source: [xyt-dev/nekochat](https://github.com/xyt-dev/nekochat) (default branch: `main`) · 🏷️ [Releases](https://github.com/xyt-dev/nekochat/releases) · 📖 [Operations and API guide](docs/guide.md) (Chinese) · 📝 [Changelog](CHANGELOG.md) (Chinese)

`nekochat` separates **configuration in the admin console from conversation in the chat interface**. In `/admin`, administrators combine a model, reasoning level, system prompt, search service, character artwork, and local sticker packs into a character card. The chat interface keeps model names, providers, parameter controls, and billing sources out of the conversation: choose a character and start chatting.

---

## ✨ Chat screenshots (Claude Dark)

Two bundled packs, **Big Fat Fish** and **Big Fat Fish & GPT Girl**, provide **322 local GIF stickers**. Characters can include stickers that match the conversation. Character artwork and responsive message layouts work on both desktop and mobile; the app also supports time-to-first-token and generation-rate metrics.

<table>
  <tr>
    <td width="68%" valign="top">
      <img src="docs/screenshots/nekowhale-chat-desktop.webp" alt="Big Fat Fish chat with stickers on desktop in Claude Dark" />
      <p align="center"><sub>🖥️ Desktop, Claude Dark, demo account Lina: character selection in the sidebar, character artwork, and local stickers in a teasing conversation.</sub></p>
    </td>
    <td width="32%" valign="top">
      <img src="docs/screenshots/nekowhale-chat-mobile.webp" alt="Big Fat Fish chat with stickers on mobile in Claude Dark" />
      <p align="center"><sub>📱 Mobile, 412px viewport, demo account Lina: full character artwork with responsive message bubbles and stickers.</sub></p>
    </td>
  </tr>
</table>

---

## 🎭 Character cards and 🔍 search: easy to configure, plenty to customize

Create and customize characters in `/admin` using visual controls and live previews. Character configuration does not require hand-editing configuration files: select the capabilities you want and save them as a reusable card.

### 🛠️ Character card editor

- **Choose the model and reasoning level per character.** Bind a card to a listed DeepSeek, OpenCode Go, or officially connected ChatGPT subscription model through Sign in with ChatGPT (SIWC). Configure the reasoning level, such as `none`, `low`, `high`, or `max`, and sampling settings supported by that model.
- **Upload artwork and preview it live.** Backgrounds support PNG, JPEG, WebP, GIF, and AVIF up to 5 MiB. Adjust opacity in the editor and preview the result. Images retain their proportions and adapt to desktop and mobile layouts.
- **Attach local sticker packs and custom skills.** Enable the bundled **Big Fat Fish (286 images)** and **Big Fat Fish & GPT Girl (36 images)** packs, or your own `skills/` resource packs. Set whether stickers represent the character (`self`) or a neutral reference (`neutral`), and choose their frequency: `rare`, `normal`, or `often`.

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/screenshots/character-editor-dark.webp" alt="Character editor with model, search service, avatar, system prompt, and reasoning settings" />
      <p align="center"><sub>🎛️ Upper editor: model selection, optional search service, avatar, system prompt, and server-side reasoning settings.</sub></p>
    </td>
    <td width="50%" valign="top">
      <img src="docs/screenshots/character-assets-dark.webp" alt="Live character artwork preview and sticker pack configuration" />
      <p align="center"><sub>🖼️ Lower editor: artwork and opacity preview, sticker pack selection, and sticker frequency controls.</sub></p>
    </td>
  </tr>
</table>

### 🌐 Connect web search

- **Two supported search backends.** The standalone `nekochat-search` service supports **DeepSeek Search and Fetch**, as well as **GPT native Web Search with a restricted local reader for web page text**.
- **Reuse the site's connected ChatGPT subscription.** After connecting an official ChatGPT account in Model Management (`模型管理`), a GPT search service can use that same connection without another sign-in. Subscription mode does not automatically switch to API-key billing.
- **Authorize search per character and track usage separately.** Assign an optional search service to a character card. The server can run a `model → web_search / web_fetch → model` loop within one reply. Separate search charts show calls and token usage in 15-minute buckets, independently of conversation billing.

<p align="center">
  <img src="docs/screenshots/search-services-dark.webp" width="92%" alt="Search service directory and independent search usage chart" />
  <br />
  <sub>🔍 Search management: DeepSeek API and the site's ChatGPT subscription, connection checks without upstream charges, real search tests, and usage charts by service.</sub>
</p>

---

## ⚡ Architecture: keeping streaming and long conversations responsive

`nekochat` separates generation, browser rendering, resource delivery, storage, and search so that slow clients and external requests do not unnecessarily hold up the chat interface. Its performance design focuses on moving expensive work off the browser's main thread and reusing completed rendering work.

```mermaid
flowchart LR
  subgraph Browser["🖥️ Browser · SolidJS"]
    UI["Virtual message list<br/>@tanstack/solid-virtual"]
    Worker["Incremental rendering in a Web Worker<br/>nekochat-mdwasm + KaTeX + hljs"]
    UI <-->|"Incremental block updates"| Worker
  end

  subgraph Server["⚙️ Main process · nekochat (Rust / Axum)"]
    Gen["Generation state machine<br/>Independent Tokio task · Snapshot + Broadcast"]
    Res["nekochat-resources<br/>322 bundled GIFs · Shared Bytes buffers"]
    Store["nekochat-store<br/>SQLite WAL · spawn_blocking"]
    Gw["nekochat-gateway<br/>Chat Completions / Responses / SIWC"]
    Gen --> Res
    Gen --> Store
    Gen --> Gw
  end

  subgraph Search["🔍 Separate search process · nekochat-search"]
    SProc["Private-pipe readiness handshake<br/>Reader with pinned public DNS results"]
  end

  UI <-->|"SSE observation / Snapshot reconnection"| Gen
  Gen <-->|"Tool loop · Short-lived token delegation"| SProc
  Gw <-->|"Streaming inference / OAuth PKCE"| Upstream["DeepSeek / OpenCode / OpenAI"]
```

### 1. 🧵 Web Worker and Rust WASM rendering

Reparsing a growing reply, typesetting formulas, and highlighting code on every token can compete with typing and scrolling. `nekochat` moves this work into a Web Worker (`crates/mdwasm` and `web/src/md/md.worker.ts`):

- **Block parsing in Rust WASM.** `pulldown-cmark`, compiled to WebAssembly as `nekochat_mdwasm`, splits streaming text into top-level Markdown blocks inside the worker.
- **Reuse completed blocks.** The worker tracks blocks by message `key` and avoids resending unchanged leading blocks. During ordinary streaming appends, the unfinished trailing block is rendered again while completed blocks are reused.
- **Defer expensive formatting until a block is complete.** `highlight.js` and KaTeX fill code and math placeholders when a block closes, keeping that work out of repeated updates to unfinished text.
- **Fine-grained DOM updates and virtual scrolling.** SolidJS updates the affected UI, while `@tanstack/solid-virtual` limits mounted message elements to the viewport and nearby items. Token updates do not depend on per-token DOM measurement animations.

### 2. 🔄 Server-owned generation, independent of browser connections

In `crates/server/src/generation.rs`, an independent server-side Tokio task owns the upstream model stream. Browser connections are **observers** subscribed to a `broadcast` channel:

- **Slow observers do not backpressure the upstream stream.** Bounded observation buffers let slow or disconnected clients resynchronize without holding up upstream model reads or other observers.
- **Reconnect to a snapshot instead of restarting inference.** After a refresh or a device switch, `GET /api/conversations/{id}/turns` subscribes to the existing generation. The server sends its current `snapshot`, followed by incremental updates, without issuing another model request.
- **Aggregate streamed text in memory.** Text and reasoning fragments accumulate in the generation state. Generation startup and terminal states—completion, interruption, or failure—are persisted to SQLite, rather than writing every token separately.

### 3. 🖼️ Shared image buffers and streaming sticker parsing

- **Share image bytes with `Bytes`.** `crates/resources` embeds the bundled sticker packs at build time. HTTP responses share reference-counted buffers and use `Cache-Control: private, max-age=86400`, avoiding a full image-buffer copy per response. Sticker assets stay out of the frontend JavaScript bundle and are served locally.
- **Parse sticker tags incrementally.** A server-side state machine recognizes authorized `[sticker:...]` tags as text arrives and replaces them with local resource references. It avoids full-text regex backtracking and withholds unfinished tags from the browser.

### 4. 🗄️ SQLite WAL and a separate search process

- **Offload blocking storage work.** `crates/store` uses SQLite WAL and a 5,000 ms busy timeout. Request handlers offload blocking SQLite operations with `tokio::task::spawn_blocking`, keeping them off asynchronous network workers.
- **Keep search and page parsing in their own process.** `nekochat-search` handles network search and HTML processing separately from the main chat service. A private pipe (`--ready-fd`) reports startup readiness. Its reader validates and pins public DNS results at connection time to defend against SSRF and DNS rebinding.

---

## 🚀 Quickstart

Build and run the web app and bundled search service directly with a **Rust toolchain** and **Bun**; a container is optional. The commands below use a shell with `source`, and token generation requires the OpenSSL command-line tool.

### 1. Install the WASM build tools

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129
```

### 2. Initialize configuration and search tokens

Copy the web and search service configuration examples, then generate two different persistent search tokens in `.env.local`, which is excluded by `.gitignore`. Generate them on first setup; on later starts, source the existing file so the tokens stay the same.

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

Adjust `config.toml` for your model connection:

- **Official ChatGPT subscription only (Plus / Pro):** comment out or remove the example `[[providers]]` sections and set `admin_password = "your-admin-password"`.
- **DeepSeek / OpenCode Go API keys:** retain the relevant `[[providers]]` sections and set `export DEEPSEEK_API_KEY=...` or `export OPENCODE_API_KEY=...`. Each configured provider requires its corresponding environment variable to be nonempty at startup.

### 3. Build the frontend and Rust release binaries

```sh
cd web && bun install && bun run build && cd ..
cargo build --release -p nekochat-server -p nekochat-search
```

Release builds embed `web/dist` into the `nekochat` binary, so a separate static-file deployment is not required.

### 4. Start the web app and search service together

```sh
source .env.local
./target/release/nekochat serve --config config.toml
```

`nekochat serve` first starts `nekochat-search` from the same binary directory. Once the child reports that authorization initialization and listener binding are complete through the private pipe, the web service starts. Open `http://127.0.0.1:10000`.

### 5. Configure a character and start chatting

1. Open `http://127.0.0.1:10000/admin` and sign in with `admin_password`.
2. In **Model Management (`模型管理`)**, connect an **official ChatGPT account (Sign in with ChatGPT)** or refresh an API provider's model directory. List the models you want to use with **`上架`**.
3. Optionally, in **Search Services (`搜索服务`)**, save the search connection using the two tokens from `.env.local`. Add a DeepSeek service or a GPT search service that reuses the site's ChatGPT connection.
4. In **Character Management (`角色管理`)**, create a character: bind a listed model, enable `nekowhale` / `nekowhale-gpt` sticker packs, optionally assign a search service, upload an avatar and artwork, and set the default character.
5. Return to `http://127.0.0.1:10000/`, register a chat account, and start a conversation.

---

## 🧩 More features

| Area | Features |
| :--- | :--- |
| **🌳 SQLite conversation trees** | Editing a previous user message or regenerating a reply creates a new branch. Use `/tree` or the branch controls to switch between conversation paths. |
| **📎 Images and document text extraction** | Paste, drag, or use `/upload` for PNG, JPEG, WebP, and GIF images, plus PDF, Markdown, text, and code files. The server extracts document text into the user message. Encrypted PDFs and scanned PDFs without extractable text return an error rather than sending a filename as document content. |
| **🔐 Official ChatGPT sign-in (SIWC)** | OpenAI Sign in with ChatGPT dynamic client registration and OAuth PKCE. Each account/workspace registration retains its issued `client_id` and stable `ext_agent_host_id`, supporting reauthorization after sign-out and removal of unused local registration records. |
| **🎨 Eight built-in themes and custom CSS** | Claude Dark / Light, Catppuccin Latte / Frappé / Macchiato / Mocha, and Gruvbox Dark / Light. Place custom styles in `<data_dir>/themes/<name>.css` to make them available in the theme menu; see [`web/THEMES.md`](web/THEMES.md) (Chinese). |
| **👥 User isolation and password policies** | Conversations, branches, attachments, and the last selected character are associated with the signed-in user on the server. Administrators can create or disable users, assign previously unowned conversations, and choose plaintext or Argon2id storage for newly created accounts. |
| **📊 Costs and rolling usage estimates** | View cached input, uncached input, reasoning, and output tokens by provider and model, with costs based on recorded historical prices. Supports DeepSeek's official balance API and local estimates of OpenCode Go's 5-hour, 7-day, and 30-day usage windows. |

### 📂 Repository layout

```text
nekochat/
├── crates/
│   ├── core/        # Character, conversation tree, metrics, and generation parameter types
│   ├── store/       # SQLite WAL, schema migrations, users, and conversation persistence
│   ├── gateway/     # Streaming Chat Completions, Responses, and ChatGPT subscription adapters
│   ├── resources/   # 322 bundled GIFs, custom resource validation, and sticker-tag parsing
│   ├── server/      # Axum HTTP service, generation, SIWC OAuth, and search process management
│   ├── mdwasm/      # pulldown-cmark block parsing and HTML rendering compiled to WebAssembly
│   └── search/      # Standalone search binary, DeepSeek / GPT search, restricted reader, and usage
├── web/             # SolidJS + TypeScript, Web Worker rendering, and eight built-in themes
├── resource-packs/  # Sticker manifests, artwork, and original license notices
└── docs/            # Screenshots and the operations/API guide in docs/guide.md
```

---

## 📖 Advanced configuration and API guide

For reverse proxies and the `allowed_hosts` allowlist, completing ChatGPT OAuth from a remote browser, deploying `nekochat-search` separately, authoring `skills/` resource packs, database upgrades, and the complete `curl` smoke-check examples, see:

👉 **[Operations and API guide (`docs/guide.md`, Chinese)](docs/guide.md)**

---

## 📄 License

Except for third-party content with its own notices, this project's original source code is **dual-licensed under Apache-2.0 or MIT, at your option**. The SPDX expression is `Apache-2.0 OR MIT`. See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT) for the full terms.

Third-party code and assets, including character artwork shown in screenshots, retain their own licenses and attribution requirements and are excluded from this choice of licenses. See the artwork notices below and the search service's [THIRD_PARTY_LICENSES](crates/search/THIRD_PARTY_LICENSES).

---

## 🙏 Artwork and acknowledgments

- The bundled **Big Fat Fish** and **Big Fat Fish & GPT Girl** stickers and annotations come from [Loping151/dsh-nekowhale-stickers](https://github.com/Loping151/dsh-nekowhale-stickers).
- Original character artwork and stickers: **[可以叫我云大人 / 雾雪](https://space.bilibili.com/103739)** ([kiriyuki.com](https://kiriyuki.com)). The knowledge base uses CC BY-NC 4.0. Artwork usage conditions and the upstream code license are preserved in [`resource-packs/LICENSE-ASSETS`](resource-packs/LICENSE-ASSETS), [`resource-packs/LICENSE`](resource-packs/LICENSE), and [`resource-packs/NOTICE`](resource-packs/NOTICE). Noncommercial sharing must retain the original author and website attribution; paid or other commercial use of the restricted assets is prohibited.
