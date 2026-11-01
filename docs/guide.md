# 使用指南

[返回项目首页](../README.md)

这里收录部署、管理、模型接入和 API 的详细用法。以下命令都从仓库根目录执行。第一次运行可以先看首页的 [Quickstart](../README.md#quickstart)。

## 运行

1. 复制 Web 和项目内搜索底座的示例配置：

   ```sh
   cp config.example.toml config.toml
   cp crates/search/config.example.toml crates/search/config.toml
   ```

2. 按需修改 `config.toml`，并为每个提供商设置 `api_key_env` 指定的环境变量：

   ```sh
   export DEEPSEEK_API_KEY=...
   export OPENCODE_API_KEY=...
   ```

   同时在运行环境中设置两个不同的高强度搜索连接令牌 `NEKOCHAT_SEARCH_RUNTIME_TOKEN`、`NEKOCHAT_SEARCH_ADMIN_TOKEN`。令牌在重启后应保持不变，并与管理端保存的搜索连接一致；不要每次启动重新生成。实际配置和数据不提交到版本库。

3. 构建网页前端（需要 bun、`wasm32-unknown-unknown` 目标和 0.2.129 版的 `wasm-bindgen` CLI）：

   ```sh
   cd web && bun install && bun run build && cd ..
   ```

   产物在 `web/dist`。release 构建会把它嵌进二进制；debug 构建运行时直接读磁盘上的 `web/dist`。

   构建 Web 与底座两个程序，产物保持在同一个目录：

   ```sh
   cargo build --release -p nekochat-server -p nekochat-search
   ```

4. 一次启动整个项目，然后用浏览器打开 `http://127.0.0.1:10000`：

   ```sh
   ./target/release/nekochat serve --config config.toml
   ```

   默认先启动同目录的 `nekochat-search`，读取项目内 `crates/search/config.toml`；底座完成授权初始化并绑定监听地址，通过只属于该子进程的管道报告就绪后才启动 Web。启动检查不向可能被抢占的本地端口发送令牌。底座仅在回环地址监听，示例数据目录为项目下的 `data/search-engine/`。停止项目会回收自己启动的底座；底座启动失败、监听冲突或运行中退出时明确失败，不继续留下不可用的 Web。

   自定义路径使用 `--search-config <path>`、`--search-program <path>`。已有独立或远程底座时，显式加 `--no-local-search` 启动 Web，并继续使用管理端保存的搜索地址；该模式不启动或终止外部底座。配置里的相对路径仍以启动时的工作目录解析，建议从项目根目录启动。

默认宿主机直接运行，监听 `0.0.0.0:10000`，无需容器。通过服务器 IP 或域名访问前，把实际地址加入 `allowed_hosts`；只允许本机访问时可显式改为 `listen = "127.0.0.1:10000"`。公网部署请设置强 `admin_password`、按需设置站点 `access_password`，并放在 HTTPS 反向代理之后（配置 `trusted_proxies`）；不要用默认弱密码对外开放管理端。

为防 DNS 重绑定，服务只接受 `Host` 为 `localhost`、`127.0.0.1`、`::1` 或监听地址本身（监听具体 IP 时）的请求，其他一律返回 421。通过反向代理或 Tailscale 主机名访问时，把这些主机名（不带端口）写进 `allowed_hosts`，例如 `allowed_hosts = ["chat.example.com"]`。

## 一键部署与清理

Linux 上可使用 Python 3.11+ 脚本，将程序与编译缓存分开：

```sh
python3 scripts/deploy.py
```

默认依次复用完整的 `target/release/` 或 `runtime/bin/` 中的两个程序；两处都没有完整程序时才运行前端和 Rust release 构建。已有程序不会按源码时间自动重编译，更新源码后使用 `python3 scripts/deploy.py --rebuild`。首次构建需要上文列出的 bun、Rust WASM 目标和 wasm-bindgen 工具。

脚本先复制并检查两个程序，再停止当前网站配置对应的主进程及它所属的搜索子进程，从 `runtime/bin/` 启动 Web 和搜索底座；确认监听端口归属新进程、真实 `/api/health` 返回 200 且首页返回 HTML 后，才删除仓库的整个 `target/`。`--no-clean` 可保留编译缓存。新程序启动失败时保留 `target/`，恢复原程序；原服务在运行时会使用原环境尝试重启，若配置本身也已失效则明确报告恢复启动失败。请勿同时运行其他 Cargo 构建或外部自动重启管理器。

配置、相对路径的工作目录和 `data/` 均继续使用原仓库，不复制正在使用的数据库，也不修改已有授权。脚本继承调用环境、读取原服务所需的环境变量，并在存在时加载 `.env.local`；成功启动后将配置声明的提供商、搜索环境变量及已有的 HTTP/HTTPS/ALL/NO_PROXY（含小写形式）保存到权限 `0600` 的 `runtime/environment.json`，用于之后启动，不打印这些值，也不保存其他无关环境变量。该文件含密钥，整个 `runtime/` 已忽略，不应上传或分享。日志为权限 `0600` 的 `runtime/service.log`。

```sh
python3 scripts/deploy.py status   # 查看当前站点主进程与搜索进程
python3 scripts/deploy.py stop     # 停止本站主进程及搜索子进程
python3 scripts/deploy.py start    # 从独立目录启动，不构建、不清理
```

停止按程序、工作目录、配置路径、父子关系和进程身份匹配，保留使用其他网站配置的隔离实例，即使它使用相同的搜索配置；等待正常退出，不直接强杀。`--runtime-dir <path>`、`--config <path>` 和 `--search-config <path>` 可指定独立目录及配置，后续命令应继续使用相同参数。已有 `bin/` 必须由本站脚本管理，不能指定含其他工具的目录；仓库锁与部署目录锁防止不同目标目录绕过并发保护。程序独立放置后仍需保留仓库里的脚本、两份配置及原数据目录。

## 用户与会话

聊天需要先注册或登录。站点访问密码是可选的外层门禁，与用户密码、管理密码分别管理；不设置站点密码时，能访问页面的人都可以自行注册，部署者须按需限制网络或设置外层门禁。

- 主题右侧的用户头像菜单提供「用户信息」与「退出登录」，可修改用户名、头像、密码。用户与角色头像都由本站上传；未设置头像时显示名字首字。新访客默认 Claude Dark，保留已有主题偏好。
- 对话、分支、上传附件和起轮权限按用户 ID 隔离，不能靠更换 URL 读取别人的数据。每个账号在服务端记忆上次选择的角色和对话，重新登录或换浏览器后恢复；不是全站共享的 localStorage 状态。
- 角色以卡片形式放在左侧「新对话」按钮上方；输入区只保留主题化附件图标和发送/停止，消息不再显示名称标题，双方头像位于对话两侧并与首行对齐。
- 升级前的旧对话不删除，也不自动分给第一个注册者。管理员在「用户管理」中明确将未归属旧对话分配给指定账号；新账号起初只看到自己的内容。
- 注册账号只提供本地数据隔离，不授予共享或转售上游订阅的权利。部署者仍需按上游条款限制可访问人员。

## 生成、刷新与停止

模型请求由 Rust core 独立维护。刷新网页、切换对话、关闭页面或 HTTP 观察流断线不会结束模型连接；回到会话会先取得完整的最新进度，再继续接收增量，不重新调用模型。无观察者或慢观察者都不会阻塞生成；历史树与附件仍按用户归属隔离。

- 手动「停止」调用当前生成 ID 对应的服务端接口，保存已生成内容为中断回复；旧页面的停止动作不能停止另一条新生成。
- 强制删除会话会取消其生成并清除数据；生成中 `/clear` 会提示先停止，不能用清空或断网偷偷替代停止。
- 退出、停用、密码重置会关闭相应 HTTP 观察权限，但 core 仍完成原账号的生成；重新登录后可恢复。服务进程重启不在网页刷新保证范围内，升级前请先结束进行中的回复。
- 输入区以固定位置显示正在连接、思考中、恢复观察和停止状态；“思考中”是界面文案，不改变模型实际的推理或输出模式。正文不靠逐 token 测量或动画维持反馈。
- `/tree` 支持键盘选择分支。选择用户消息会恢复其文本与附件进入编辑，发送后形成新分支；跳转到回复节点会清理未发送的编辑与附件，再切换上下文。
- 代码块复制优先使用浏览器 Clipboard API；普通 HTTP 环境未提供该 API 时使用真实的系统复制回退。复制保留键盘焦点与选区；权限拒绝或复制失败会明确提示手动选择，不显示假成功。

## 管理端与监控

在 `config.toml` 设置 `admin_password` 后，访问 `http://127.0.0.1:10000/admin`。未设置管理密码时管理 API 不开放；管理密码独立于可选的网页访问密码。

- **角色卡**：绑定实际模型、生成参数、提示词、公开显示名、本地头像/背景和资源包。卡片上可直接「设为默认」，不可用角色不能设为默认；用户已有可用选择仍优先。前端只在侧栏选择整套角色，不显示底层模型、提供商、思考等级或套餐来源。
- **背景图**：本地上传支持 PNG、JPEG、WebP、GIF 和 AVIF，单张至多 5 MiB，不扩展头像或聊天附件的格式范围。角色编辑器支持透明度调节和实时预览。图片等比完整展示，限制宽度并保留边距，不拉伸或裁切；背景不覆盖顶栏和输入区。设置随角色保存在服务端，普通用户不能自行修改。
- **手机背景**：沿用页面的 760px 移动布局断点，窄屏保留完整图片与边距，不改写角色卡保存的设置。
- **模型**：按提供商刷新上游模型目录，选择上架哪些模型，设置显示名、排序、思考档位与默认档、图片能力、上下文窗口，以及每百万 token 的美元价格。API key 仍只在服务端环境变量中管理，网页不能查看或修改。支持 Chat Completions、标准 Responses，以及通过 OAuth 使用官方 ChatGPT 订阅的 Responses；Messages / Gemini 尚未接入。标准 Responses 不接受 seed、penalty、stop 或 DeepSeek thinking；官方订阅当前只允许思考档位，不能设置输出上限、temperature、top_p 等不受支持参数。
- **用户**：创建、编辑、停用、删除、重置密码、设置头像和分配旧对话。密码策略仅决定此后新建账号使用明文还是 Argon2id 哈希（新安装默认明文）；既有账号不因策略切换、登录或重置而转换，也不弹转换确认。明文仅管理端显式查看。实际改密码/停用会撤销认证及观察权限，凭据版本阻止旧验证恢复会话；不会终止 core 生成。数据库、WAL/SHM 和备份仍须保护。
- **统计**：按时间、提供商和模型查看费用与 token 用量；图表颜色区分「提供商 + 模型」，可分别查看缓存输入、无缓存输入、思考和输出。输出数不含思考，图中的总计包含这四类。费用保留调用当时的记录，改价或下架不会改写历史。
- **未知费用**：未设置价格且上游未返回费用的请求标记为未定价，不当作免费。部分已知时显示已知费用和未定价请求数。
- **额度**：DeepSeek 查询官方余额，按原币种显示；OpenCode Go 根据本机记录的 `opencode.allowance` 估算最近 5 小时、7 天、30 天的用量。它不是账户的权威余额，不包含其他客户端的调用，滚动窗口也不代表服务商实际重置时刻。缺失额度计数或限额时显示未知，不伪造剩余数。

角色、模型和用户编辑器在实际保存失败时保留草稿。写入已明确成功、但后续列表刷新失败时，会关闭已提交的编辑器并区分提示「已保存，但列表刷新失败」，使用页面重试加载控件恢复列表，不自动重发写操作。切换管理页不会丢掉已完成保存对共享目录的刷新；默认角色以服务端确认的设置为准。

官方 `api.deepseek.com` / `opencode.ai` 地址可自动识别监控类型；自定义代理地址需要在提供商的 `monitoring` 中指定。OpenCode 默认按 Go 计划，可配置 `plan = "go_plus"`，或用 `monthly_limits` 按上游模型 ID 覆盖月额度；5 小时与每周额度分别按月额度的 20% / 50% 估算。示例见 `config.example.toml`。

额度与余额接口来源：[OpenCode Go](https://opencode.ai/v2/docs/console/go/)、[DeepSeek 余额](https://api-docs.deepseek.com/api/get-user-balance)。价格和计划可能变化，上架前请核对服务商当期信息。

## 图片与文件输入

输入框支持直接粘贴 PNG、拖放、点击「上传附件」或输入 `/upload`，共用 `POST /api/conversations/{id}/attachments` 接口。图片在编辑框内显示，发送后与文字保留在同一消息气泡；PDF、Markdown、文本及常见代码文件在编辑框上方排列，可以移除或下载。

- 图片支持 PNG / JPEG / WebP / GIF，单张至多 8 MiB；可以只发送图片。角色必须绑定支持视觉的模型，否则前后端明确拒绝，不会悄悄丢图。
- PDF 与 UTF-8 文本文件单个至多 16 MiB，服务端真实提取内容（至多 2 MiB 文本）并放入所属用户消息，而非系统指令；不会把文档错误地作为图片转发。损坏、加密、无可提取文字的扫描 PDF 或超限文件会明确报错，不伪装 OCR，不静默截断。
- 每条消息图片和文件合计至多 5 个。原始文件、名称和提取文本保存在本地 SQLite，随历史、编辑、重新生成及分支保留。读取需要对应用户的会话归属权限；不使用外链或公共缓存。
- 清空或彻底删除对话会删除全部关联文件，包括已经上传但尚未发送的附件。

已实际验证 Go GPT 同时读取图片、Markdown 与 PDF 的真实内容；其他上游的图片能力仍需管理员确认后启用。

## 连接官方 ChatGPT 账号

1. 打开 `/admin` → **模型管理** → **连接官方ChatGPT账号（订阅）**，点击 **Continue with ChatGPT**。
2. 本人在 OpenAI 页面登录并同意 ChatGPT plan usage。这里使用官方 SIWC 动态注册与公开 `api.openai.com/v1`，不代理网页 Cookie，也不借用 pi / Codex 保存的其他应用凭证。
3. 本机浏览器会回到临时 `127.0.0.1` 回调。远程浏览器若打不开回调，请把地址栏完整的最终回调 URL 粘贴到管理端「完整回调 URL」后完成授权，或使用 SSH 转发相应端口。不要分享、截图或记录该 URL。
4. 连接后在提供商中选 **ChatGPT 订阅（官方账号）**，刷新实际账号模型目录、选择上架，再在角色管理中绑定。不会自动替换原有 GPT娘、其他角色或默认配置。

授权凭证保存在 `<data_dir>/chatgpt-account.json`，采用私密 `0600` 权限与原子替换；支持账号/工作区选择、自动刷新、取消和退出。退出清除当前会话令牌，保留注册映射和稳定主机标识，便于下次重新授权。同一数据目录只运行一个服务实例，不把同一轮换令牌交给多个进程同时刷新。

清理不再需要的本地账号记录：在同一账号选择框中选中「需重新授权」或「未完成注册」条目，再点「移除本地记录」。已保存会话的账号必须先切换到它并退出；授权处理中也不能移除。操作只删除选中的本地注册映射，保留其他账号、凭证和主机标识；以后重新添加可能生成新的应用注册，仍要使用原账号时优先保留记录并重新授权。

OpenAI 侧的授权需在 [ChatGPT 设置](https://chatgpt.com/settings/usage) 管理：应用用量限制页关闭不再使用的应用开关并保存，可停止其使用套餐额度；不保证删除官方注册条目。本地移除不会删除官方应用，[官方会话说明](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions#end-the-renewable-session) 也明确指出撤销会话不删除客户端注册。

普通聊天页和公开角色接口不再暴露 ChatGPT 套餐来源；首次授权说明、真实来源与管理用量入口保留在管理端。token 统计按真实提供商记录，未知余额、额度与金钱成本保持未知；达到订阅限额不会自动改用 API key。OpenAI 的套餐、工作区与应用权限仍然有效，OAuth 技术接入不等于允许公开共享或转售订阅。

官方参考：[Sign in with ChatGPT](https://developers.openai.com/siwc/token-sharing-open-source)、[模型与推理](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)、[UI 要求](https://developers.openai.com/siwc/ui-ux-guidelines)。

## 独立搜索底座

搜索源码已经作为子目录放在本项目的 `crates/search/`，由独立 Rust 程序 `nekochat-search` 提供，网页和角色只通过 Nekochat 后端调用它。项目默认统一启动底座和 Web；搜索服务仍可独立构建运行，不依赖聊天数据库或 Nekochat crate，未来其他应用可复用同一 HTTP 协议。它不是 shell、裸 curl 或本地文件搜索器。

1. 按上方运行步骤设置两个不同的环境变量并复制 `crates/search/config.example.toml` 到 `crates/search/config.toml`。默认仅监听 `127.0.0.1:10011`，数据保存在项目下的 `data/search-engine/`，目录应限制访问权限。
2. 默认只需启动项目的 `nekochat serve --config config.toml`，底座会先启动。需要单独运行时使用 `cargo run -p nekochat-search -- --config crates/search/config.toml`，此时 Web 加 `--no-local-search`；单独构建时可复制整个 `crates/search` 目录，其依赖不继承主工作区。
3. 在 `/admin` → **搜索服务** 填入后端地址、两种令牌和预算，再保存。**测试已保存连接**只验证运行与管理授权，不调用收费搜索。远程部署使用 HTTPS；委托订阅访问凭据只允许 HTTPS 或明确的本机回环 HTTP，不经环境代理或重定向。
4. 新建 DeepSeek Search / Fetch 服务并填写官方 API key，或新建 GPT Web Search 服务。GPT 默认**复用本站已连接的 ChatGPT 订阅**，账号仍在「模型管理」连接、选择与退出，不需要第二次登录。填写该账号实际获准使用的搜索模型；显式选择 API key 模式才按 OpenAI API 计费，绝不自动切换账单来源。
5. 在**角色管理**为角色选择一个已启用、已连接的搜索服务。未绑定的角色没有搜索工具；普通用户不能覆盖服务、模型、密钥或绑定。已绑定服务离线时编辑器保留原选择，不会静默解除，重新分配必须明确保存。

服务 API key 保存在底座的私密 `0600` 数据文件；管理接口只返回是否配置，不回读原值。连接令牌保存在 Nekochat 的本地设置中，因此数据库、WAL/SHM、底座数据与备份都必须限制权限。GPT 订阅凭据仍只有现有 SIWC 管理器负责保存与刷新，底座只接收单次请求的短期 access token，不保存 refresh/id token，也不执行授权或刷新。浏览器不直接访问底座，不会拿到这些凭据。

### 搜索、读取与多轮回答

角色可以调用 `web_search(query)`，再通过 `web_fetch(source_id)` 读取**本轮搜索已返回的来源 ID**，不能提交任意 URL、路径或请求头。DeepSeek 的 Search / Fetch 都由其官方远端执行；GPT 使用原生 Web Search，进一步读取链接由底座内的受限 HTTP Reader 完成，不需要再接一个模型。结果统一保留来源 URL、标题、摘要、可用正文、提供商生成摘要与检索时间；生成摘要不会伪装成网页原文。

搜索不是“先给用户返回一轮，再手动接续”：Rust core 在同一条回复内部执行有限次模型→工具→模型调用，最终带真实来源链接回答。默认每轮最多 3 次工具调用、每次最多 5 条来源，管理端分别可设 1–8、1–10；请求超时可设 1–180 秒，默认 60 秒。预算耗尽后不再提供工具，不重试、不换服务；全部模型轮次与已经发生的搜索用量均记账。这不是把多轮 `max` 伪装成 `ultra`。

网页刷新、关闭或观察连接断线不会重新搜索，也不会中止 core；重新进入先同步最新进度。手动停止和强制删除对话会取消模型与当前底座请求，删除正文不会抹掉已经发生的匿名用量。

网页与搜索摘要按不可信资料处理，不获得系统指令权限。只转发模型提出的查询或获准来源，不将整段会话自动作为查询；模型仍可能将用户内容写入查询，使用第三方搜索前应避免提交敏感信息。

### 访问边界与费用

- 只接受无账号凭据的公开 HTTP(S) 来源，拒绝 `file:`、本地路径、环回/内网字面地址和本地域名。DeepSeek Fetch 只把来源作为 JSON 发送至固定官方接口，由提供商解析、获取并实施远端安全策略，不访问服务器本地文件。
- 本地 Reader 在实际连接时验证并固定公开 DNS 地址，混合公开/私网结果也拒绝；禁用代理、Cookie、认证和重定向，并限制响应大小、类型、正文和解析工作量。不读取 PDF/二进制、不运行网页 JS。Fake-IP DNS（如 `198.18.0.0/15`）会被安全拒绝，部署者需要为该进程提供真实公开 DNS，不能通过放开私网地址“修复”。
- 管理页的**真实搜索测试**需要明确确认，确实消耗上游额度。按当前官方标价，DeepSeek 成功且非空的 Search 或 Fetch 各记录 `0.005 CNY`，空搜索记录 `0 CNY`；这是按公开标价计算的成本，不是账户账单对账。GPT 未返回金钱成本时显示未知，不当作免费。不同币种分开统计、不擅自换汇；每轮模型 token 费用与搜索费用分开记。
- 官方订阅的 Web Search 仍受模型、账号、工作区及套餐政策约束；技术接入不授权公开共享或转售订阅。失败会明确返回，不伪造答案、来源或成功用量。
- 管理端「搜索服务」的用量默认显示折线图，按服务区分颜色。默认查看调用尝试次数，也可切换缓存输入、无缓存输入、思考与输出（不含思考）等 token 指标；未返回 token 的原生搜索不代表没有发生搜索。
- 图表与费用汇总覆盖完整所选时段；切到明细后只显示该时段最新 100 条，按时间降序排列。费用未知次数明确列出，CNY / USD 不混加；最长查询范围仍为 31 天，不通过限制明细来少算历史用量。

接口与政策参考：[DeepSeek Search / Fetch 与定价](https://api-docs.deepseek.com/zh-cn/guides/search/)、[SIWC 当前能力与限制](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations.md)。0.10.0 提供独立搜索底座及管理入口；连接后仍由管理员明确给角色绑定服务，不自动改变旧角色的搜索权限。


## 本地 skill 资源包

内置 [dsh-nekowhale-stickers](https://github.com/Loping151/dsh-nekowhale-stickers) 的「大肥鱼」「大肥鱼和 GPT 娘」两套标注与完整 322 张 GIF。图片随程序本地提供，浏览器不向作者站点或 CDN 拉图，服务运行时也不下载素材。

内置图片约 388 MiB，会增加 Rust 二进制体积，但不会进入网页 JS 包或模型提示词；内置资源按静态字节共享，自定义资源请求也不复制整份图片缓冲。

管理员在角色编辑器中勾选资源包。服务端仅把已勾选包的静态说明和表情目录放进该角色的提示词；模型按描述输出 `[sticker:名字]`，服务端精确解析成本站图片。未授权、未知、歧义或近轮重复的表情不会随机替代，半截标记不发到前端。身份 `self` / `neutral` 和频率由管理端设置。

大肥鱼身份提示词可参考上游 `src/index.ts` 的 `PERSONA.self.skill`；GPT 娘在原仓库中有形象与互动方向资料，但没有独立的完整性格提示词，需据此适配。GPT 娘使用 Go GPT 时选标准 Responses，使用官方 ChatGPT 账号时选官方订阅协议；不要把角色名称当成实际模型身份。两张角色卡均可由管理员继续编辑，原有默认角色无需改动。

自定义包放在 `<data_dir>/skills/<pack_id>/`，每包包含 `SKILL.md`、`pack.json`、`stickers.json` 和 `assets/`，重启后加载。这里的 skill 是**只读资源包**，不是可执行插件，不给模型命令或任意文件访问。背景图也只接受本地上传或本地包素材，默认不设背景。

一个最小自定义包示例（目录名必须与清单 id 一致，不允许符号链接或 `..` 路径）：

```text
skills/my-pack/
  pack.json
  SKILL.md
  stickers.json
  assets/wave.gif
```

`pack.json`：
```json
{"id":"my-pack","name":"我的资源包","description":"自用招呼表情","attribution":"填写实际作者与使用许可"}
```

`stickers.json`：
```json
[{"id":"wave.gif","name":"挥手","file":"assets/wave.gif","aliases":["打招呼"],"emotion":"开心","tags":["问候"],"use":"见面或友好打招呼时","dir":""}]
```

`SKILL.md` 写静态的素材使用说明；文件里的工具、命令不会被执行。复制真实图片到 `assets/` 后重启，再在角色编辑器勾选 `my-pack`。

资源包页把补充信息统一显示为「描述」，其中保留原作者、来源及许可内容；显示名称变化不取消素材的使用条件。

少量通用知识也可以写入资源包的 `SKILL.md`，例如科研方法、术语定义和写作规范；纯文本包可把 `stickers.json` 设为 `[]`，不必添加图片。管理员仍需在角色编辑器勾选该包。当前会将整份 `SKILL.md` 加入每轮模型上下文，消耗上下文与 token，并不是训练模型或按需检索知识库。

当前不会自动读取包目录中的其他 `.md` 或 `.pdf` 文件，也不解析 `SKILL.md` 中的文件链接。PDF 自动提取目前仅用于聊天附件；要在现有资源包中使用其内容，应先人工整理成 `SKILL.md`。独立文档导入、PDF 知识资源和大文献库检索尚未实现。

知识库采用 CC BY-NC 4.0，GIF 的使用条件见随包保留的 `LICENSE-ASSETS`；非商业分享必须保留作者与网站署名，禁止收费和其他商业用途。作者：[可以叫我云大人/雾雪](https://space.bilibili.com/103739)，[kiriyuki.com](https://kiriyuki.com)。复用代码的 Apache-2.0 许可、NOTICE 和形象设计者署名一并保留。署名链接只用于来源说明，不作为图片外链。

角色、参数与资源授权在 Rust 端解析；起轮服务和资源事件可供其他 Rust 通道复用。Web 只负责角色卡和图片呈现，未来微信等应用不需要复制这套策略。本轮没有接入微信 bot。

## 通用图形输出

在管理端角色编辑器的「授权资源包」勾选 **通用图形（指令包）** 并保存。此包提供 `neko-ui` 声明式协议，不含图片；纯指令包不使用表情身份、发送频率或其他素材包的许可限制。既有角色不会自动勾选，默认角色保持原有设置。

授权后，图形指引要求角色主动优先使用图形：实质回答默认至少一个合适的图形，无需用户额外请求；解释、方案和总结可用卡片布局，数值对比用图表，函数变化用本地滑块，流程与关系用节点图。用户明确要求纯文字、只要代码或固定格式时优先遵守，极简寒暄可直接用文字，不能为作图编造数据。当前这是模型指令，尚未实施服务端「必须含有效图形」的完成验收；图形协议由展示端 Rust/WASM 校验，不能将指令理解为模型必然使用图形的保证。

角色可以在普通 Markdown 中输出 `neko-ui` JSON 围栏。原生组件支持文字、指标卡、row/col/grid/card 布局、line/bar/area/donut 数据图表、参数函数图、可选择节点的关系图与静态 SVG。图形提供源码查看与复制、主题默认配色恢复、局部调色和适用的本地控件。函数表达式由 Rust/WASM 解析，在独立 Worker 采样；参数调整不调用模型。

布局背景在图形顶部的「布局」配色入口统一调整，指标和图表旁的调色图标可修改相应配色；同一时刻仅展开一个页内色板，展开时为内容腾出空间，不遮挡邻近图形。直接点击主题色块即可生效，也可输入完整 `#RRGGBB` 自定义色值；无需系统选色弹窗或确认按钮。文字节点可用 `color` 指定安全的主题语义色或十六进制色，点击「主题配色」可一并恢复文字与其他图形的主题默认值。未指定背景的行、列和网格保持透明，显式背景与卡片保留独立内边距，手机窄屏自动堆叠。

例如，要求角色输出下面的振幅演示：

````markdown
```neko-ui
{"version":1,"title":"观察振幅","items":[{"type":"plot","xMin":-6.28,"xMax":6.28,"yMin":-3,"yMax":3,"params":[{"name":"a","label":"振幅","value":1,"min":0,"max":3,"step":0.1}],"series":[{"label":"a sin(x)","expr":"a*sin(x)"}]}]}
```
````

JSON 上限 128 KiB、128 节点、8 层嵌套；图表最多 8 系列、每系列 2048 点，函数图最多 8 参数/8 系列，表达式 256 字符。SVG 上限 64 KiB、1024 元素，仅接受静态几何、文字、渐变与裁切；禁止脚本、事件、外链与动画。裸 `svg` 围栏也可使用，优先设置 viewBox 并使用 `currentColor` 或 `var(--nk-accent)` 等主题颜色。完整节点字段见 [图形包指引](../resource-packs/genui/SKILL.md) 与 [协议规范](superpowers/specs/2026-11-01-graphics-design.md)。

默认图形颜色随主题与自定义 CSS 变量变化，可明确指定十六进制颜色。图形原文沿用已有消息记录，历史显示不依赖当前是否仍授权教学包；未完成的 JSON 显示生成提示，围栏结束后无效内容显示错误并保留源码。参数、配色与视图选择在本浏览器标签页内有界保存，滚动卸载或刷新后可恢复，退出/切换账号时清除；不保存额外消息副本或凭证。图形界面当前用于 Web 通道，微信等通道仍需各自的呈现方式。


## 自定义主题

把 CSS 文件放进 `<data_dir>/themes/<名字>.css`（名字只能用字母、数字、`_`、`-`），它会出现在右上角主题菜单的「自定义」分组里。变量清单与写法见 `web/THEMES.md`。

## 已有数据升级

0.13.0 沿用 schema 6，无新增迁移；0.10.0 引入的 schema 6 兼容已有 schema 1–5，新增可空的角色搜索服务绑定和独立搜索用量表，旧角色默认不启用搜索。保留对话树、模型、计量、账号及官方 GPT 授权；早期未绑定可用模型的角色不发布，旧未归属对话须在管理端用户页明确分配，不自动交给第一个注册者。升级前先结束进行中的回复、停止服务并备份数据目录；旧程序不能直接读取更高版本 schema，回退旧程序必须恢复配套的升级前数据库，验证实例不要复用线上数据目录。已有账号、角色与 OAuth 无需重建，会话按原有效期验证；进程重启不等同于网页刷新，不能恢复旧上游连接。

## curl 冒烟

以下流程走一遍管理端与对话接口，需要 `curl` 与 `jq`，并在 `config.toml` 里设置了 `admin_password`（未设置时管理接口一律 404）。示例假定默认地址与上面配置里的 `deepseek` 提供商；请把 `你的管理密码` 换成实际值。

```sh
BASE=http://127.0.0.1:10000
JAR=$(mktemp)

# 健康检查，输出 ok
curl -s $BASE/api/health

# 管理端登录：会话 cookie（nk_admin）存进 $JAR，成功返回 204
curl -s -c $JAR -H 'Content-Type: application/json' \
  -d '{"password":"你的管理密码"}' $BASE/api/admin/session
# 若设置了 access_password，再登录一次访问端（cookie nk_access 写入同一个 $JAR）：
# curl -s -b $JAR -c $JAR -H 'Content-Type: application/json' \
#   -d '{"password":"你的访问密码"}' $BASE/api/session

# 提供商上游的模型列表；listed_model_id 非空表示已上架
curl -s -b $JAR $BASE/api/admin/providers/deepseek/models

# 上架模型（先从上面的目录核对 ID；价格单位：美元/百万 token，未知可填 null）
MODEL_ID=$(curl -s -b $JAR -H 'Content-Type: application/json' -d '{
    "provider": "deepseek",
    "upstream_id": "deepseek-flash",
    "display_name": "DeepSeek V4.1 Flash",
    "sort": 0,
    "protocol": "chat_completions",
    "reasoning": {"style": "deepseek", "levels": ["none", "low", "high", "max"], "default": "high"},
    "vision": false,
    "context_window": null,
    "price": {"cached_in": null, "uncached_in": null, "output": null}
  }' $BASE/api/admin/models | jq -r .id)

# 建立绑定模型的完整角色；思考等级在这里由管理员固定
CHAR_ID=$(curl -s -b $JAR -H 'Content-Type: application/json' \
  -d "{\"name\":\"猫娘\",\"display_name\":\"小雪\",\"system_prompt\":\"你是一只友好的猫娘。\",\"model_id\":\"$MODEL_ID\",\"parameters\":{\"effort\":\"high\"},\"resource_packs\":[\"nekowhale\",\"nekowhale-gpt\"],\"sticker_identity\":\"neutral\",\"sticker_frequency\":\"often\"}" \
  $BASE/api/admin/characters | jq -r .id)

# 仅设置默认角色，没有全局默认模型
curl -s -b $JAR -X PUT -H 'Content-Type: application/json' \
  -d "{\"default_character_id\":\"$CHAR_ID\"}" \
  $BASE/api/admin/settings

# 用户账号登录（管理会话不能代替用户会话），成功设置 nk_user：
# 首次使用先在网页注册，或将下面 login 改为 register。
curl -s -b $JAR -c $JAR -H 'Content-Type: application/json' \
  -d '{"username":"你的用户名","password":"你的用户密码"}' \
  $BASE/api/account/login

# 聊天端只取得角色公开名称、头像、背景和图片能力，不返回模型/提示词/参数/套餐来源
curl -s -b $JAR $BASE/api/characters

# 用角色建立对话
CONV_ID=$(curl -s -b $JAR -H 'Content-Type: application/json' \
  -d "{\"character_id\":\"$CHAR_ID\"}" $BASE/api/conversations | jq -r .id)

# 发一轮；不允许附带 model_id、effort 或 parameters 覆盖
curl -s -N -b $JAR -H 'Content-Type: application/json' \
  -d "{\"action\":\"send\",\"content\":\"你好\",\"character_id\":\"$CHAR_ID\"}" \
  $BASE/api/conversations/$CONV_ID/turns
```

最后一步依次输出 `start`（新用户消息与回复节点 id）、若干 `reasoning`/`text` 增量，最后是 `done`（完整的回复节点，含 `metrics.usage`）；上游出错时以 `error` 结束。形如：

```text
event: start
data: {"assistant_node_id":"…","character_id":"…","character_name":"小雪","user_node":{"role":"user","content":"你好",…}}

event: reasoning
data: {"t":"…"}

event: text
data: {"t":"…"}

event: done
data: {"node":{"role":"assistant","status":"complete","character_name":"小雪","content":"…","reasoning":"…","metrics":{"ttft_ms":…,"total_ms":…,"tokens_per_sec":…,"usage":{"cached_in":…,"uncached_in":…,"reasoning":…,"output":…}},…}}
```

`GET $BASE/api/conversations/$CONV_ID` 可查看整棵消息树。

`GET $BASE/api/conversations/$CONV_ID/turns` 只观察现有生成，先返回 `snapshot`（`node`、原始 `start` 与已耗时/思考时间），再继续增量；无活动生成时为 204。`GET` 对话包含 `active_turn_id`，用于刷新恢复，不能把重连实现成重发 `POST`。

手动停止使用 `POST $BASE/api/conversations/$CONV_ID/turns/$TURN_ID/stop`，`TURN_ID` 取 `active_turn_id` 或 `start.assistant_node_id`。成功/已结束为 204；另一个生成正在运行时旧 ID 返回 409。会话与这两个接口均校验账号归属，认证撤销会关闭观察，不取消模型。
