# nekochat 计划 1：服务端核心 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 一个可用 curl 跑通的 Rust 服务端：读配置、存对话树、经 Chat Completions 协议流式调用 DeepSeek / OpenCode Go、用 SSE 把回复和 debug 指标推给客户端、记录用量，并提供管理端 JSON 接口（模型上架、角色、默认设置）。

**Architecture:** Cargo workspace，四个 crate：`core`（共享领域类型）、`store`（rusqlite + SQLite，对话树与目录）、`gateway`（reqwest + SSE 的协议适配）、`server`（axum，lib + bin）。存储是同步 API，服务端经 `spawn_blocking` 调用。每次生成在独立 tokio 任务里跑，通过 mpsc 喂给 SSE 响应，客户端断开或对话被删除时取消上游并按规则落库。

**Tech Stack:** Rust（本机 nightly 1.100）、axum 0.8、axum-extra（`cookie` 特性）、tokio 1、tokio-util 0.7（`CancellationToken`）、reqwest 0.13（`json`、`stream`）、eventsource-stream 0.2、rusqlite 0.41（`bundled`）、serde / serde_json、toml 1、clap 4、uuid 1（`v4`、`v7`）、subtle 2、thiserror 2、anyhow 1、tracing 0.1；测试用 tempfile、reqwest（`cookies`）。

**Spec:** `docs/superpowers/specs/2026-10-26-nekochat-design.md`

### 计划系列（整份设计分 6 个计划，每个都产出可运行的软件）

1. **服务端核心**（本计划）
2. 网页聊天前端：SolidJS、Rust→WASM Markdown Worker、KaTeX、虚拟滚动、主题、对话树 UI、命令、debug 行、静态资源打进二进制
3. 管理端前端与统计：模型上架（含内置默认值表）、角色、统计图、额度
4. 表情包：导入 nekowhale、提示词拼装、流式标记拦截、标注流程
5. `/compact`（含自动压缩）与上传
6. Responses / Messages 协议适配（OpenCode Go 的 GPT、Claude、MiniMax 等）与 ChatGPT 订阅登录

## Global Constraints

- 模型没有任何工具；请求体里永远不带 `tools`。
- 日志（tracing）里不得出现任何消息内容、思考内容、系统提示词；只记 id、模型、状态、token 数、错误码。
- 所有上游请求带 `User-Agent: nekochat/<CARGO_PKG_VERSION>`；配置了 `session_header` 的提供商额外带 `<session_header>: <conversation.session_key>`（OpenCode Go 为 `x-opencode-session`）。
- Chat Completions 请求永远带 `"stream": true` 与 `"stream_options": {"include_usage": true}`。
- SQLite 每个连接开 `PRAGMA secure_delete = ON`、`PRAGMA foreign_keys = ON`、`PRAGMA journal_mode = WAL`；删除 / 清空对话后执行 `PRAGMA wal_checkpoint(TRUNCATE)`。
- 用量表不含对话 id 和任何内容。
- 未设 `admin_password` 时管理接口一律 404；判断访问来源时不信任环回地址。
- 默认监听 `127.0.0.1:8787`。
- 时间一律 Unix 毫秒 `i64`；id 一律 UUIDv7 字符串。

## Review Focus

1. 上游返回非 2xx（key 错、429、Go 额度用完）：客户端收到 `error` 事件，含上游状态码和消息；回复节点以 `error` 状态落库；进程不 panic。→ Task 4、Task 7 测试。
2. 客户端在流式输出中途断开：上游请求被取消；已收到的部分以 `interrupted` 状态落库，刷新后可见。→ Task 7 测试。
3. 一个中文字符的 UTF-8 字节被拆在两个 TCP 分片里：输出不乱码。→ Task 4 测试。
4. 生成过程中删除该对话：生成立即停止，不落任何节点，删除后数据库与 WAL 文件里都找不到内容。→ Task 7 测试。
5. 同一对话重复提交（双击发送）：第二个请求返回 409，不产生第二条回复。→ Task 7 测试。

---

## 文件结构

```
nekochat/
  Cargo.toml                         workspace，统一依赖版本
  config.example.toml                示例配置
  README.md                          运行说明
  crates/core/src/lib.rs             共享类型：Protocol、ReasoningStyle、ReasoningSpec、Price、Usage、Metrics
  crates/store/src/lib.rs            Store、StoreError、open/迁移
  crates/store/src/schema.sql        建表语句
  crates/store/src/catalog.rs        角色、设置、上架模型
  crates/store/src/tree.rs           对话、节点、对话树操作、删除
  crates/store/src/usage.rs          用量记录
  crates/store/tests/*.rs
  crates/gateway/src/lib.rs          Provider、ChatRequest、StreamEvent、GatewayError、USER_AGENT
  crates/gateway/src/chat_completions.rs   build_body、parse_usage、stream_chat、list_models
  crates/gateway/tests/chat_completions.rs
  crates/server/src/main.rs          clap：nekochat serve --config <path>
  crates/server/src/lib.rs           app(state) -> Router、serve(config)
  crates/server/src/config.rs        Config、load、startup_warnings
  crates/server/src/state.rs         AppState、RunningTurns
  crates/server/src/auth.rs          口令登录、会话 cookie、两个鉴权层
  crates/server/src/api/user.rs      用户端接口
  crates/server/src/api/admin.rs     管理端接口
  crates/server/src/chat.rs          上下文拼装、生成任务、SSE、指标、落库
  crates/server/tests/common/mod.rs  spawn_mock_upstream、spawn_app
  crates/server/tests/*.rs
```

---

### Task 1: Workspace、共享类型与配置

**Files:**
- Create: `Cargo.toml`、`crates/{core,store,gateway,server}/Cargo.toml`、`crates/core/src/lib.rs`、`crates/server/src/{main.rs,lib.rs,config.rs}`、`config.example.toml`、`README.md`、`.gitignore`（`target/`、`data/`）
- Test: `crates/server/tests/config.rs`

**Interfaces:**
- Produces（`nekochat_core`，全部 `Serialize + Deserialize + Clone + Debug + PartialEq`，枚举 `#[serde(rename_all = "snake_case")]`）：
  - `enum Protocol { ChatCompletions }`
  - `enum ReasoningStyle { None, Effort, DeepSeek }`，序列化为 `"none"` / `"effort"` / `"deepseek"`（`DeepSeek` 显式 `#[serde(rename = "deepseek")]`）
  - `struct ReasoningSpec { style: ReasoningStyle, levels: Vec<String>, default: Option<String> }`
  - `struct Price { cached_in: Option<f64>, uncached_in: Option<f64>, output: Option<f64> }`（美元 / 每百万 token）
  - `struct Usage { cached_in: u64, uncached_in: u64, reasoning: u64, output: u64, upstream_cost_usd: Option<f64>, allowance_usd: Option<f64> }`（`output` 不含思考），`impl Default`
  - `struct Metrics { ttft_ms: Option<u64>, total_ms: u64, tokens_per_sec: Option<f64>, usage: Usage }`
- Produces（`nekochat_server::config`）：
  - `struct ProviderEntry { key: String, label: String, base_url: String, models_url: Option<String>, api_key_env: String, session_header: Option<String> }`
  - `struct Config { listen: SocketAddr, data_dir: PathBuf, access_password: Option<String>, admin_password: Option<String>, trusted_proxies: Vec<IpAddr>, providers: Vec<ProviderEntry> }`；`listen` 默认 `127.0.0.1:8787`，`data_dir` 默认 `./data`，空字符串口令视为 `None`
  - `fn load(path: &Path) -> anyhow::Result<Config>`
  - `fn resolve_api_keys(cfg: &Config, env: impl Fn(&str) -> Option<String>) -> anyhow::Result<Vec<(ProviderEntry, String)>>`：缺环境变量时报错，错误信息包含提供商 key 和变量名
  - `fn startup_warnings(cfg: &Config) -> Vec<String>`

- [ ] **Step 1: 写失败的测试** `crates/server/tests/config.rs`

```rust
#[test] fn example_config_parses() {
    let cfg = load(Path::new("../../config.example.toml")).unwrap();
    assert_eq!(cfg.listen, "127.0.0.1:8787".parse().unwrap());
    assert_eq!(cfg.providers.iter().map(|p| p.key.as_str()).collect::<Vec<_>>(), ["deepseek", "opencode-go"]);
    assert_eq!(cfg.providers[1].session_header.as_deref(), Some("x-opencode-session"));
}
#[test] fn missing_api_key_names_provider_and_var() {
    let err = resolve_api_keys(&cfg_with_one_provider(), |_| None).unwrap_err().to_string();
    assert!(err.contains("deepseek") && err.contains("DEEPSEEK_API_KEY"));
}
#[test] fn loopback_without_password_has_no_warning() { assert!(startup_warnings(&cfg("127.0.0.1:8787", None, vec![])).is_empty()); }
#[test] fn public_without_password_warns_open_access() {
    let w = startup_warnings(&cfg("0.0.0.0:8787", None, vec![]));
    assert!(w.iter().any(|s| s.contains("未设置访问密码")));
}
#[test] fn public_with_password_without_proxy_warns_plaintext() {
    let w = startup_warnings(&cfg("0.0.0.0:8787", Some("pw"), vec![]));
    assert!(w.iter().any(|s| s.contains("明文")));
}
#[test] fn public_with_password_and_proxy_is_quiet() {
    assert!(startup_warnings(&cfg("0.0.0.0:8787", Some("pw"), vec!["127.0.0.1".parse().unwrap()])).is_empty());
}
```

（`cfg(...)` / `cfg_with_one_provider()` 是测试文件内的小构造函数。）

- [ ] **Step 2: 运行确认失败** — `cargo test -p nekochat-server --test config`，预期编译失败（`load` 未定义）。
- [ ] **Step 3: 实现** workspace、`core` 类型、`config.rs`；`config.example.toml` 内容：

```toml
listen = "127.0.0.1:8787"
data_dir = "./data"
access_password = ""
admin_password = ""
trusted_proxies = []

[[providers]]
key = "deepseek"
label = "DeepSeek 官方"
base_url = "https://api.deepseek.com"
api_key_env = "DEEPSEEK_API_KEY"

[[providers]]
key = "opencode-go"
label = "OpenCode Go"
base_url = "https://opencode.ai/inference/go/openai/v1"
models_url = "https://opencode.ai/inference/v1/models"
api_key_env = "OPENCODE_API_KEY"
session_header = "x-opencode-session"
```

`main.rs` 只解析 `serve --config <path>` 并调用 `nekochat_server::serve`。本 Task 的 `serve` 只做三件事：绑定监听、用 `tracing` 打印 `startup_warnings`、挂 `GET /api/health` → `200 ok`；Task 5 再把它换成完整路由。README 写清：复制示例配置、设置环境变量、`cargo run -p nekochat-server -- serve --config config.toml`。

- [ ] **Step 4: 运行确认通过** — `cargo test -p nekochat-server --test config`，6 个测试 PASS。
- [ ] **Step 5: 提交** — `git add -A && git commit -m "feat: workspace, core types, config"`

---

### Task 2: 存储——建库、角色、设置、上架模型

**Files:**
- Create: `crates/store/src/{lib.rs,schema.sql,catalog.rs}`
- Test: `crates/store/tests/catalog.rs`

**Interfaces:**
- Consumes: `nekochat_core::{Protocol, ReasoningSpec, Price}`
- Produces（`nekochat_store`）：
  - `struct Store`（内部 `Mutex<rusqlite::Connection>`，`Send + Sync`）；`fn open(path: &Path) -> Result<Store, StoreError>`：建目录、设 pragma、执行 `schema.sql`（`CREATE TABLE IF NOT EXISTS`，`PRAGMA user_version = 1`）
  - `enum StoreError { NotFound, Conflict(String), Sqlite(rusqlite::Error), Json(serde_json::Error) }`（thiserror）
  - `struct Character { id, name, system_prompt: String, created_at: i64, updated_at: i64 }`
  - `fn create_character(&self, name: &str, system_prompt: &str) -> Result<Character>`；`update_character(&self, id, name, system_prompt) -> Result<Character>`；`delete_character(&self, id) -> Result<()>`；`list_characters(&self) -> Result<Vec<Character>>`；`get_character(&self, id) -> Result<Option<Character>>`
  - `enum SettingKey { DefaultModel, DefaultCharacter }`（存为 `default_model_id` / `default_character_id`）；`get_setting(&self, SettingKey) -> Result<Option<String>>`；`set_setting(&self, SettingKey, Option<&str>) -> Result<()>`
  - `struct ModelSpec { provider: String, upstream_id: String, display_name: String, sort: i64, protocol: Protocol, reasoning: ReasoningSpec, vision: bool, context_window: Option<i64>, price: Price }`
  - `struct ListedModel { id: String, #[serde(flatten)] spec: ModelSpec }`
  - `insert_model(&self, &ModelSpec) -> Result<ListedModel>`（同一 `provider + upstream_id` 重复 → `Conflict`）；`update_model(&self, id, &ModelSpec) -> Result<ListedModel>`；`delete_model(&self, id) -> Result<()>`；`list_models(&self) -> Result<Vec<ListedModel>>`（按 `sort`、`display_name` 升序）；`get_model(&self, id) -> Result<Option<ListedModel>>`
  - 删除被 `default_*` 设置引用的角色或模型时，同时把该设置清空

`schema.sql`（本计划所有表一次建好，后续 Task 只用不改）：

```sql
CREATE TABLE IF NOT EXISTS characters (id TEXT PRIMARY KEY, name TEXT NOT NULL, system_prompt TEXT NOT NULL,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS models (id TEXT PRIMARY KEY, provider TEXT NOT NULL, upstream_id TEXT NOT NULL,
  display_name TEXT NOT NULL, sort INTEGER NOT NULL, protocol TEXT NOT NULL, reasoning TEXT NOT NULL,
  vision INTEGER NOT NULL, context_window INTEGER, price TEXT NOT NULL, UNIQUE(provider, upstream_id));
CREATE TABLE IF NOT EXISTS conversations (id TEXT PRIMARY KEY, title TEXT NOT NULL,
  character_id TEXT REFERENCES characters(id) ON DELETE SET NULL, leaf_id TEXT, session_key TEXT NOT NULL,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS nodes (id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  parent_id TEXT REFERENCES nodes(id) ON DELETE CASCADE, role TEXT NOT NULL, content TEXT NOT NULL,
  reasoning TEXT, model_id TEXT, model_label TEXT, effort TEXT, status TEXT NOT NULL, metrics TEXT,
  created_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS nodes_by_parent ON nodes(conversation_id, parent_id, created_at);
CREATE TABLE IF NOT EXISTS usage (id INTEGER PRIMARY KEY, at_ms INTEGER NOT NULL, provider TEXT NOT NULL,
  upstream_id TEXT NOT NULL, model_label TEXT NOT NULL, cached_in INTEGER NOT NULL, uncached_in INTEGER NOT NULL,
  reasoning INTEGER NOT NULL, output INTEGER NOT NULL, cost_usd REAL, upstream_cost_usd REAL,
  allowance_usd REAL, purpose TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS usage_by_time ON usage(at_ms);
```

- [ ] **Step 1: 写失败的测试** `crates/store/tests/catalog.rs`（每个测试用 `tempfile::tempdir()` 新库）

```rust
#[test] fn reopen_keeps_data() { /* create_character → drop → open 同路径 → list_characters().len() == 1 */ }
#[test] fn models_listed_in_sort_order() {
    // 插入 sort=2 "B"、sort=1 "Z"、sort=1 "A" → 显示名顺序 ["A","Z","B"]
}
#[test] fn duplicate_upstream_model_conflicts() {
    store.insert_model(&spec("deepseek", "deepseek-v4-pro")).unwrap();
    assert!(matches!(store.insert_model(&spec("deepseek", "deepseek-v4-pro")), Err(StoreError::Conflict(_))));
}
#[test] fn model_spec_round_trips() { /* reasoning.levels == ["none","low","high","max"], default Some("high"), price 全字段相等 */ }
#[test] fn deleting_default_model_clears_setting() {
    let m = store.insert_model(&spec("deepseek", "x")).unwrap();
    store.set_setting(SettingKey::DefaultModel, Some(&m.id)).unwrap();
    store.delete_model(&m.id).unwrap();
    assert_eq!(store.get_setting(SettingKey::DefaultModel).unwrap(), None);
}
#[test] fn update_missing_character_is_not_found() {
    assert!(matches!(store.update_character("nope", "n", "p"), Err(StoreError::NotFound)));
}
```

- [ ] **Step 2: 运行确认失败** — `cargo test -p nekochat-store --test catalog`
- [ ] **Step 3: 实现** `lib.rs`（open、pragma、迁移）与 `catalog.rs`。`reasoning`、`price` 存 JSON 文本。
- [ ] **Step 4: 运行确认通过** — 6 个测试 PASS
- [ ] **Step 5: 提交** — `git commit -am "feat(store): schema, characters, settings, listed models"`

---

### Task 3: 存储——对话树与彻底删除

**Files:**
- Create: `crates/store/src/{tree.rs,usage.rs}`
- Test: `crates/store/tests/tree.rs`、`crates/store/tests/usage.rs`

**Interfaces:**
- Consumes: Task 2 的 `Store`；`nekochat_core::{Metrics, Usage}`
- Produces:
  - `struct Conversation { id, title: String, character_id: Option<String>, leaf_id: Option<String>, session_key: String, created_at: i64, updated_at: i64 }`
  - `enum NodeRole { User, Assistant }`；`enum NodeStatus { Complete, Interrupted, Error }`
  - `struct Node { id, conversation_id: String, parent_id: Option<String>, role: NodeRole, content: String, reasoning: Option<String>, model_id: Option<String>, model_label: Option<String>, effort: Option<String>, status: NodeStatus, metrics: Option<Metrics>, created_at: i64 }`
  - `create_conversation(&self, character_id: Option<&str>) -> Result<Conversation>`：标题 `"新对话"`，`session_key` 为新 UUIDv4
  - `list_conversations(&self) -> Result<Vec<Conversation>>`（`updated_at` 降序）；`get_conversation(&self, id) -> Result<Option<Conversation>>`；`set_title(&self, id, title) -> Result<()>`
  - `insert_node(&self, node: &Node) -> Result<()>`：同一事务里插入、把对话 `leaf_id` 设为该节点、更新 `updated_at`；对话不存在 → `NotFound`；`parent_id` 不属于同一对话 → `Conflict`
  - `get_node(&self, conv_id, node_id) -> Result<Option<Node>>`
  - `set_leaf(&self, conv_id, node_id: Option<&str>) -> Result<()>`：节点不属于该对话 → `NotFound`
  - `newest_leaf_under(&self, conv_id, node_id) -> Result<String>`：反复取 `created_at` 最大的子节点，直到没有子节点
  - `path_to(&self, conv_id, node_id: Option<&str>) -> Result<Vec<Node>>`：根 → 该节点；`None` 返回空
  - `nodes(&self, conv_id) -> Result<Vec<Node>>`：全部节点，`created_at` 升序
  - `delete_conversation(&self, id) -> Result<()>`、`clear_conversation(&self, id) -> Result<()>`（删全部节点、`leaf_id = NULL`、标题保留）；两者都在提交后执行 `wal_checkpoint(TRUNCATE)`
  - `enum UsagePurpose { Chat }`；`struct UsageRecord { at_ms: i64, provider, upstream_id, model_label: String, usage: Usage, cost_usd: Option<f64>, purpose: UsagePurpose }`；`insert_usage(&self, &UsageRecord) -> Result<()>`；`usage_between(&self, from_ms, to_ms) -> Result<Vec<UsageRecord>>`

- [ ] **Step 1: 写失败的测试** `crates/store/tests/tree.rs`

```rust
#[test] fn path_runs_root_to_leaf() {
    // u1 → a1 → u2 → a2，path_to(leaf) 的 id 顺序 == [u1,a1,u2,a2]
}
#[test] fn regenerate_creates_sibling_and_moves_leaf() {
    // u1 → a1；再插 a1b(parent=u1)；conv.leaf_id == a1b；nodes() 里 parent==u1 的有两个
}
#[test] fn newest_leaf_under_follows_latest_children() {
    // u1 → a1 → u2 → a2，且 u1 → a1b（更晚）；newest_leaf_under(u1) == a1b；newest_leaf_under(a1) == a2
}
#[test] fn set_leaf_rejects_foreign_node() {
    // 两个对话；set_leaf(conv_a, node_of_b) → Err(NotFound)
}
#[test] fn insert_with_parent_from_other_conversation_conflicts() { /* → Err(Conflict(_)) */ }
#[test] fn clear_keeps_conversation_and_removes_nodes() {
    // clear 后 get_conversation 仍在、leaf_id None、nodes() 为空
}
#[test] fn delete_leaves_no_content_bytes() {
    let marker = format!("MARKER-{}", uuid::Uuid::new_v4());
    // 插入 content=marker 的用户节点和 reasoning=marker 的回复节点，然后 delete_conversation
    for f in ["nekochat.db", "nekochat.db-wal"] {
        let p = dir.path().join(f);
        if p.exists() { assert!(!contains(&std::fs::read(&p).unwrap(), marker.as_bytes()), "{f}"); }
    }
}
#[test] fn deleting_character_detaches_conversations() { /* conv.character_id 变 None，对话仍在 */ }
```

`crates/store/tests/usage.rs`：

```rust
#[test] fn usage_query_is_half_open_by_time() {
    // at_ms = 100, 200, 300；usage_between(100, 300) 返回 at_ms [100, 200]
}
```

- [ ] **Step 2: 运行确认失败** — `cargo test -p nekochat-store`
- [ ] **Step 3: 实现** `tree.rs`、`usage.rs`。`path_to` 用递归 CTE（`WITH RECURSIVE` 沿 `parent_id` 上溯）后反转。
- [ ] **Step 4: 运行确认通过** — `cargo test -p nekochat-store`，全部 PASS
- [ ] **Step 5: 提交** — `git commit -am "feat(store): conversation tree, secure delete, usage"`

---

### Task 4: 网关——Chat Completions 适配

**Files:**
- Create: `crates/gateway/src/{lib.rs,chat_completions.rs}`
- Test: `crates/gateway/tests/chat_completions.rs`（用 axum 起本地假上游）

**Interfaces:**
- Consumes: `nekochat_core::{ReasoningStyle, Usage}`
- Produces（`nekochat_gateway`）：
  - `const USER_AGENT: &str`（`concat!("nekochat/", env!("CARGO_PKG_VERSION"))`）
  - `struct Provider { key: String, label: String, base_url: String, models_url: Option<String>, api_key: String, session_header: Option<String> }`
  - `enum ChatRole { System, User, Assistant }`；`struct ChatMessage { role: ChatRole, content: String }`
  - `struct ChatRequest<'a> { model: &'a str, messages: &'a [ChatMessage], reasoning: ReasoningStyle, effort: Option<&'a str>, session_id: &'a str }`
  - `enum StreamEvent { Reasoning(String), Text(String), Usage(Usage) }`
  - `enum GatewayError { Http { status: u16, message: String }, Transport(String), Protocol(String) }`（thiserror，`Display` 含状态码与消息）
  - `fn build_body(req: &ChatRequest) -> serde_json::Value`
  - `fn parse_usage(v: &serde_json::Value) -> Usage`
  - `async fn stream_chat(http: &reqwest::Client, p: &Provider, req: &ChatRequest<'_>) -> Result<BoxStream<'static, Result<StreamEvent, GatewayError>>, GatewayError>`：POST `{base_url}/chat/completions`
  - `async fn list_models(http: &reqwest::Client, p: &Provider) -> Result<Vec<String>, GatewayError>`：GET `models_url` 或 `{base_url}/models`，返回 `data[].id`

规则（来自 spec 4.4，`build_body`）：
- `ReasoningStyle::None`：不加任何思考字段。
- `Effort` + `Some(l)`：加 `"reasoning_effort": l`。
- `DeepSeek` + `Some("none")`：加 `"thinking": {"type": "disabled"}`，不加 `reasoning_effort`；`DeepSeek` + `Some(l)`：加 `"thinking": {"type": "enabled"}` 与 `"reasoning_effort": l`。

规则（`parse_usage`，输入是 `usage` 对象），按顺序取第一个命中的：
1. 有 `opencode.tokens`：`cached_in = cache_read`，`uncached_in = input + cache_write_5m + cache_write_1h`，`reasoning = tokens.reasoning`，`output = tokens.output - tokens.reasoning`；`upstream_cost_usd = usage.cost`，`allowance_usd = opencode.allowance`。
2. 有 `prompt_cache_hit_tokens`（DeepSeek）：`cached_in = prompt_cache_hit_tokens`，`uncached_in = prompt_cache_miss_tokens`。
3. 否则：`cached_in = prompt_tokens_details.cached_tokens`（缺省 0），`uncached_in = prompt_tokens - cached_in`。
- 2、3 两种情况：`reasoning = completion_tokens_details.reasoning_tokens`（缺省 0），`output = completion_tokens - reasoning`。

流解析：增量文本取 `choices[0].delta.content`；思考取 `choices[0].delta.reasoning_content`，没有则取 `choices[0].delta.reasoning`；空字符串不产生事件；带 `usage` 的块产生 `Usage`；`data: [DONE]` 结束。非 2xx：读响应体，`message` 取 `error.message`，否则取原文前 500 字符。

- [ ] **Step 1: 写失败的测试**

```rust
#[test] fn deepseek_none_disables_thinking() {
    let b = build_body(&req(ReasoningStyle::DeepSeek, Some("none")));
    assert_eq!(b["thinking"], json!({"type":"disabled"}));
    assert!(b.get("reasoning_effort").is_none());
}
#[test] fn deepseek_level_enables_thinking() {
    let b = build_body(&req(ReasoningStyle::DeepSeek, Some("max")));
    assert_eq!(b["thinking"], json!({"type":"enabled"}));
    assert_eq!(b["reasoning_effort"], "max");
}
#[test] fn effort_style_sets_reasoning_effort_only() { /* thinking 缺失，reasoning_effort == "xhigh" */ }
#[test] fn body_always_streams_with_usage_and_no_tools() {
    let b = build_body(&req(ReasoningStyle::None, None));
    assert_eq!(b["stream"], true);
    assert_eq!(b["stream_options"], json!({"include_usage": true}));
    assert!(b.get("tools").is_none() && b.get("reasoning_effort").is_none());
}
#[test] fn usage_opencode() {
    let u = parse_usage(&json!({"prompt_tokens":1200,"completion_tokens":300,"cost":0.0,
      "opencode":{"allowance":0.0041,"tokens":{"input":1200,"output":300,"reasoning":120,"cache_read":8000,"cache_write_5m":10,"cache_write_1h":0}}}));
    assert_eq!((u.cached_in, u.uncached_in, u.reasoning, u.output), (8000, 1210, 120, 180));
    assert_eq!((u.upstream_cost_usd, u.allowance_usd), (Some(0.0), Some(0.0041)));
}
#[test] fn usage_deepseek() {
    let u = parse_usage(&json!({"prompt_tokens":1204,"prompt_cache_hit_tokens":896,"prompt_cache_miss_tokens":308,
      "completion_tokens":426,"completion_tokens_details":{"reasoning_tokens":140}}));
    assert_eq!((u.cached_in, u.uncached_in, u.reasoning, u.output), (896, 308, 140, 286));
}
#[test] fn usage_openai_style() {
    let u = parse_usage(&json!({"prompt_tokens":1000,"prompt_tokens_details":{"cached_tokens":600},"completion_tokens":50}));
    assert_eq!((u.cached_in, u.uncached_in, u.reasoning, u.output), (600, 400, 0, 50));
}
#[tokio::test] async fn stream_yields_reasoning_text_usage_in_order() { /* 假上游脚本：reasoning "想", text "喵", usage 块, [DONE] */ }
#[tokio::test] async fn stream_sends_user_agent_and_session_header() {
    // 假上游记录请求头：user-agent 以 "nekochat/" 开头；x-opencode-session == req.session_id；authorization == "Bearer k"
}
#[tokio::test] async fn utf8_split_across_chunks_is_intact() {
    // 假上游把 `data: {"choices":[{"delta":{"content":"喵喵"}}]}` 的字节在「喵」的第 2 字节处切开分两次写出
    // 拼接所有 Text 事件 == "喵喵"
}
#[tokio::test] async fn http_error_carries_status_and_message() {
    // 假上游 429 + {"error":{"message":"rate limited"}}
    // Err(GatewayError::Http { status: 429, message }) 且 message == "rate limited"
}
#[tokio::test] async fn list_models_uses_models_url() { /* models_url 指向假上游 /v1/models → ["a","b"] */ }
```

- [ ] **Step 2: 运行确认失败** — `cargo test -p nekochat-gateway`
- [ ] **Step 3: 实现**。SSE 用 `eventsource_stream::Eventsource` 包 `resp.bytes_stream()`；reqwest 客户端由调用方创建并复用。
- [ ] **Step 4: 运行确认通过** — `cargo test -p nekochat-gateway`，全部 PASS
- [ ] **Step 5: 提交** — `git commit -am "feat(gateway): chat completions adapter"`

---

### Task 5: 服务端——状态、鉴权与管理端接口

**Files:**
- Create: `crates/server/src/{state.rs,auth.rs,api/mod.rs,api/admin.rs}`；修改 `lib.rs`
- Create: `crates/server/tests/common/mod.rs`
- Test: `crates/server/tests/auth.rs`、`crates/server/tests/admin.rs`

**Interfaces:**
- Consumes: Task 1 `Config`、`resolve_api_keys`；Task 2/3 `Store`；Task 4 `Provider`、`list_models`
- Produces:
  - `struct AppState { store: Arc<Store>, http: reqwest::Client, providers: Arc<Vec<Provider>>, config: Arc<Config>, sessions: Sessions, running: RunningTurns }`（`Clone`；按 `Provider.key` 线性查找）
  - `type RunningTurns = Arc<Mutex<HashMap<String /*conv_id*/, CancellationToken>>>`（本 Task 只建空表，Task 7 使用）
  - `fn app(state: AppState) -> axum::Router`；`async fn serve(cfg: Config) -> anyhow::Result<()>`（`Store::open(data_dir/nekochat.db)`）
  - `Sessions`：两个内存令牌集合（访问、管理），令牌是 32 字节随机数的十六进制
  - 测试工具 `tests/common/mod.rs`：
    - `struct MockScript { status: u16, chunks: Vec<(Duration /*发送前等待*/, Vec<u8>)>, models: Vec<String> }`；`MockScript::sse(events: &[serde_json::Value])` 生成 `data: <json>\n\n` 块并以 `data: [DONE]\n\n` 结尾
    - `spawn_mock_upstream(script: MockScript) -> MockUpstream { base_url: String, requests: Arc<Mutex<Vec<RecordedRequest /*headers + body JSON*/>>>, closed_early: Arc<AtomicBool> }`：`POST /chat/completions` 按脚本输出，写失败时置 `closed_early`；`GET /models` 返回 `{"data":[{"id":..}]}`
    - `spawn_app(cfg: TestCfg) -> TestApp { base: String, client: reqwest::Client /* 开 cookie_store */, store: Arc<Store>, dir: TempDir }`

接口（全部 JSON；错误体 `{"error": "<中文说明>"}`）：

| 方法与路径 | 鉴权 | 行为 |
|---|---|---|
| `GET /api/health` | 无 | `200 "ok"` |
| `POST /api/session` `{password}` | 无 | 未设访问密码 → `204`；正确 → `204` + `Set-Cookie: nk_access=<token>; HttpOnly; SameSite=Strict; Path=/`；错误 → 等待 500ms 后 `401` |
| `POST /api/admin/session` `{password}` | 无 | 未设管理密码 → `404`；正确 → `204` + `nk_admin` cookie；错误 → 500ms 后 `401` |
| `GET /api/admin/providers` | 管理 | `[{key,label}]`（配置顺序） |
| `GET /api/admin/providers/{key}/models` | 管理 | `[{upstream_id, listed_model_id: string\|null}]`，来自 `list_models` |
| `GET /api/admin/models` | 管理 | `[ListedModel]` |
| `POST /api/admin/models` `ModelSpec` | 管理 | `201 ListedModel`；provider 不在配置 → `400`；重复 → `409` |
| `PUT /api/admin/models/{id}` `ModelSpec` | 管理 | `ListedModel` / `404` |
| `DELETE /api/admin/models/{id}` | 管理 | `204` |
| `GET/POST /api/admin/characters`，`PUT/DELETE /api/admin/characters/{id}` | 管理 | 角色增删改查 `{name, system_prompt}` |
| `GET/PUT /api/admin/settings` `{default_model_id, default_character_id}` | 管理 | 引用不存在 → `400` |

鉴权层：设了访问密码时，`/api/*` 中除 `/api/health`、`/api/session`、`/api/admin/*` 外都要 `nk_access`，否则 `401`；`/api/admin/*`（除登录）都要 `nk_admin`，否则 `401`，未设管理密码时一律 `404`。口令比较用 `subtle::ConstantTimeEq`。

- [ ] **Step 1: 写失败的测试**

`tests/auth.rs`：
```rust
#[tokio::test] async fn open_access_when_no_password() { /* GET /api/conversations → 200 */ }
#[tokio::test] async fn access_password_gates_user_api() {
    // access_password="pw"：GET /api/conversations → 401；POST /api/session {"password":"bad"} → 401；
    // {"password":"pw"} → 204；再 GET → 200
}
#[tokio::test] async fn health_is_always_open() { /* access_password="pw" 时 GET /api/health → 200 */ }
#[tokio::test] async fn admin_absent_without_admin_password() { /* POST /api/admin/session → 404；GET /api/admin/models → 404 */ }
#[tokio::test] async fn admin_requires_admin_cookie() {
    // admin_password="ap"：GET /api/admin/models → 401；登录后 → 200
}
#[tokio::test] async fn access_cookie_does_not_grant_admin() { /* 只登访问 → GET /api/admin/models == 401 */ }
```

`tests/admin.rs`（已登录管理端）：
```rust
#[tokio::test] async fn provider_models_mark_listed() {
    // 假上游 /models 返回 ["m1","m2"]；上架 m2 后 GET providers/{key}/models 中 m2.listed_model_id 非空、m1 为 null
}
#[tokio::test] async fn listing_unknown_provider_is_400() {}
#[tokio::test] async fn settings_reject_dangling_ids() { /* PUT settings default_model_id="nope" → 400 */ }
#[tokio::test] async fn character_crud_round_trip() {}
```

- [ ] **Step 2: 运行确认失败** — `cargo test -p nekochat-server --test auth --test admin`
- [ ] **Step 3: 实现** `state.rs`、`auth.rs`、`api/admin.rs`；`serve` 改为用 `app(state)`；存储调用一律 `tokio::task::spawn_blocking`。`StoreError` → HTTP：`NotFound`→404、`Conflict`→409、其他→500（不带内部细节）。
- [ ] **Step 4: 运行确认通过** — 上述测试全部 PASS
- [ ] **Step 5: 提交** — `git commit -am "feat(server): auth and admin api"`

---

### Task 6: 服务端——用户端对话接口

**Files:**
- Create: `crates/server/src/api/user.rs`
- Test: `crates/server/tests/conversations.rs`

**Interfaces:**
- Consumes: Task 3 树操作；Task 5 `AppState`、鉴权层
- Produces（返回体字段名即前端在计划 2 要用的契约）：

| 方法与路径 | 行为 |
|---|---|
| `GET /api/models` | `{"default_model_id": string\|null, "models": [{"id","display_name","provider_label","label","reasoning":{"levels":[..],"default":..},"vision"}]}`；`label` = `"<display_name> · <provider_label>"`；`reasoning.style` 为 `none` 时 `levels` 为空数组；不返回价格与上游 id |
| `GET /api/conversations` | `[{"id","title","updated_at"}]` |
| `POST /api/conversations` | 用默认角色（未设则无角色）新建 → `201 {"id","title","updated_at"}` |
| `GET /api/conversations/{id}` | `{"id","title","leaf_id","nodes":[NodeView]}`，`nodes` 为全部节点（前端自行算路径与兄弟） |
| `DELETE /api/conversations/{id}` | 彻底删除 → `204` |
| `POST /api/conversations/{id}/clear` | 清空 → `204` |
| `PUT /api/conversations/{id}/leaf` `{"node_id": string, "descend": bool}` | `descend=true` 时落到 `newest_leaf_under(node_id)`；返回 `{"leaf_id"}` |

`NodeView` = `{"id","parent_id","role","content","reasoning","model_label","effort","status","metrics","created_at"}`，不含任何角色信息与价格。

- [ ] **Step 1: 写失败的测试**

```rust
#[tokio::test] async fn models_endpoint_exposes_only_picker_fields() {
    // 管理端上架一个带价格的 deepseek 模型（display_name "DeepSeek V4 Pro"）
    // models[0] 的键集合 == {"id","display_name","provider_label","label","reasoning","vision"}
    // models[0].reasoning 的键集合 == {"levels","default"}
    // models[0].label == "DeepSeek V4 Pro · DeepSeek 官方"
}
#[tokio::test] async fn conversation_view_has_no_character_data() {
    // 设默认角色（system_prompt 含 "SECRET-PROMPT"）后新建对话；GET 详情的 JSON 文本不含 "SECRET-PROMPT" 与 "character"
}
#[tokio::test] async fn leaf_descend_picks_newest_branch() { /* 用 store 直接造树，PUT leaf descend=true 返回最新叶子 */ }
#[tokio::test] async fn leaf_to_foreign_node_is_404() {}
#[tokio::test] async fn delete_then_get_is_404() {}
```

- [ ] **Step 2: 运行确认失败** — `cargo test -p nekochat-server --test conversations`
- [ ] **Step 3: 实现** `api/user.rs`
- [ ] **Step 4: 运行确认通过**
- [ ] **Step 5: 提交** — `git commit -am "feat(server): conversation api"`

---

### Task 7: 服务端——生成一轮回复（SSE）

**Files:**
- Create: `crates/server/src/chat.rs`；修改 `api/user.rs`、`state.rs`
- Test: `crates/server/tests/turns.rs`

**Interfaces:**
- Consumes: Task 4 `stream_chat`、`StreamEvent`；Task 3 `insert_node`、`path_to`、`insert_usage`；Task 6 路由
- Produces:
  - `POST /api/conversations/{id}/turns`，请求体（`#[serde(tag = "action", rename_all = "snake_case")]`）：
    - `{"action":"send","content":string,"model_id":string,"effort":string|null}`：父节点 = 当前 `leaf_id`
    - `{"action":"edit","node_id":string,"content":string,"model_id":..,"effort":..}`：`node_id` 必须是用户节点；新用户节点的父 = 它的父
    - `{"action":"regenerate","node_id":string,"model_id":..,"effort":..}`：`node_id` 必须是用户节点；新回复挂在它下面
  - 响应 `text/event-stream`，事件（`event:` 名 + `data:` JSON）：
    - `start` `{"user_node": NodeView|null, "assistant_node_id": string, "model_label": string}`（regenerate 时 `user_node` 为 null）
    - `reasoning` `{"t": string}`、`text` `{"t": string}`
    - `done` `{"node": NodeView}`
    - `error` `{"message": string, "node": NodeView|null}`
  - 使用 Task 5 的 `RunningTurns`
  - `fn cost_usd(price: &Price, u: &Usage) -> Option<f64>`：`(cached_in*p.cached_in + uncached_in*p.uncached_in + (output+reasoning)*p.output) / 1e6`，任一所需价格缺失 → `None`

校验（都在调用上游前，失败返回 JSON 错误而非 SSE）：对话不存在 → 404；该对话已有生成在跑 → 409；`model_id` 未上架 → 400；模型 `reasoning.style != none` 时 `effort` 必须在 `levels` 里（`null` 则用 `default`），`style == none` 时 `effort` 必须为 null → 否则 400；`node_id` 不存在或不是用户节点 → 400。

上下文：`[system: 角色提示词（有角色时）] + path_to(父节点) 中的节点 + 本轮用户消息`；跳过 `status = error` 且 `content` 为空的回复节点；只发 `content`，不发 `reasoning`（spec 6.2：换模型时不传思考）。

生成任务：在 `tokio::spawn` 里跑，经 `mpsc` 推事件给 SSE；记录 `t0`（发请求前）、首个 `Reasoning`/`Text` 时刻、最后一个增量时刻。
- `ttft_ms = 首个增量 − t0`；`total_ms = 结束 − t0`；`tokens_per_sec = (output + reasoning) / (最后增量 − 首个增量)`，间隔为 0 或无 usage 时为 `None`。
- 正常结束：回复节点 `Complete` 落库，写用量（`cost_usd` 按上架价格计算），发 `done`。
- 上游报错（含中途）：已有部分以 `Error` 状态落库（若有内容或思考），发 `error`；没有 usage 时不写用量。
- mpsc 发送失败（客户端断开）：取消上游，部分以 `Interrupted` 状态落库，有 usage 才写用量。
- `CancellationToken` 被触发（对话被删除）：立即停止，**不**落任何节点、不写用量。
- 无论哪种结束，都从 `RunningTurns` 移除该对话。
- 对话标题仍为 `"新对话"` 时，用本轮用户消息前 30 个字符（按 `char` 计）作为标题。
- `DELETE /api/conversations/{id}` 与 `clear` 先触发该对话的 `CancellationToken`，再删。

- [ ] **Step 1: 写失败的测试** `tests/turns.rs`（假上游按脚本输出 SSE，可设每块之间的延时）

```rust
#[tokio::test] async fn send_streams_and_persists() {
    // 脚本：reasoning "想", text "喵~", usage(DeepSeek 格式：hit 896, miss 308, completion 426, reasoning 140)
    // SSE 事件名顺序 == ["start","reasoning","text","done"]
    // done.node.status == "complete"，content == "喵~"，reasoning == "想"
    // done.node.metrics.usage == {cached_in:896, uncached_in:308, reasoning:140, output:286}
    // GET 对话：leaf_id == done.node.id，标题 == 用户消息前 30 字
    // store.usage_between(0, i64::MAX) 有 1 条，cost_usd == Some((896*0.1 + 308*1.0 + 426*2.0)/1e6)（测试模型价格 0.1/1.0/2.0）
}
#[tokio::test] async fn upstream_request_carries_system_prompt_and_history_without_reasoning() {
    // 两轮后检查假上游收到的第二个请求：messages[0] == {"role":"system","content":角色提示词}，
    // 历史回复只有 content，整个请求 JSON 不含第一轮的思考文本
}
#[tokio::test] async fn regenerate_adds_sibling_reply() { /* 同一用户节点下两个回复，leaf 指向新的 */ }
#[tokio::test] async fn edit_adds_sibling_user_message() { /* 新用户节点 parent == 原节点 parent */ }
#[tokio::test] async fn upstream_error_is_reported_and_persisted() {
    // 假上游 429 {"error":{"message":"rate limited"}}；事件 == ["start","error"]，message 含 "429" 与 "rate limited"
    // 不写用量；进程仍能处理下一个请求
}
#[tokio::test] async fn client_disconnect_persists_interrupted_partial() {
    // 脚本：text "第一段"，延时 2s，text "第二段"；客户端读到第一个 text 事件后丢弃响应
    // 轮询 store 至多 3s：存在 status == Interrupted、content == "第一段" 的回复节点；RunningTurns 已清空
    // 假上游记录到连接被提前关闭（第二段未发出）
}
#[tokio::test] async fn delete_during_stream_stops_and_leaves_nothing() {
    // 慢脚本；收到 start 后 DELETE 对话 → 204；SSE 在 3s 内结束
    // GET 对话 → 404；数据库与 WAL 文件里找不到用户消息的标记文本
}
#[tokio::test] async fn double_submit_is_conflict() { /* 第一个慢流未结束时第二个 POST turns → 409 */ }
#[tokio::test] async fn effort_outside_levels_is_400_and_no_upstream_call() { /* 假上游请求计数 == 0 */ }
#[tokio::test] async fn unlisted_model_is_400() {}
#[test] fn cost_requires_all_prices() { /* price.output = None → cost_usd == None */ }
```

- [ ] **Step 2: 运行确认失败** — `cargo test -p nekochat-server --test turns`
- [ ] **Step 3: 实现** `chat.rs` 与路由
- [ ] **Step 4: 运行确认通过** — `cargo test -p nekochat-server --test turns`，全部 PASS
- [ ] **Step 5: 提交** — `git commit -am "feat(server): streaming turns with metrics and usage"`

---

### Task 8: 整体验证与真机冒烟

**Files:**
- Modify: `README.md`（补 curl 冒烟示例：登录管理端、上架模型、设默认、建对话、发一轮）

- [ ] **Step 1: 全量检查** — `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`，预期全部通过、无警告。
- [ ] **Step 2: 启动冒烟** — 用 `config.example.toml`（填 `admin_password`）启动 `cargo run -p nekochat-server -- serve --config config.toml`；`curl -s localhost:8787/api/health` 输出 `ok`；未设对应环境变量时启动报错信息含提供商 key 与变量名。
- [ ] **Step 3: 真实上游冒烟（有 key 时）** — 按 README 的 curl 流程对 DeepSeek 上架 `deepseek-v4-flash`（style `deepseek`，levels `["none","low","high","max"]`，default `"high"`），发一轮，确认收到 `start/reasoning/text/done`，`done.node.metrics.usage` 中输出与思考计数非零，统计表新增一条。没有 key 时在交接说明里写明未做此步。
- [ ] **Step 4: 提交** — `git commit -am "docs: server smoke instructions"`
