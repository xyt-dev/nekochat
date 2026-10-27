# 图片输入与官方 ChatGPT 账号接入

> 执行：按已有设计补漏；用户要求快速实现，不重复全套测试。两条独立链路并行，主代理负责共享路由与最终集成。

**Goal:** 修复 PNG 粘贴缺失，并像 pi 一样在管理端接入官方 ChatGPT/Codex OAuth，而不是用 Go API 代替订阅。
**Spec:** `docs/superpowers/specs/2026-10-26-nekochat-design.md` §4.4、§10.3、§11。
**Architecture:** 沿用 Rust axum/SQLite/SolidJS。图片作为对话内本地附件，保存历史与树分支，转换到真实多模态消息；订阅使用单独 Codex Responses 协议和服务端 OAuth/刷新凭证。

## 约束与接口
- 不读取、输出或提交用户其他应用的凭证；官方授权必须由用户本人操作。API key 配置不受影响。
- 不更换用户已有角色/默认绑定。ChatGPT 登录入口仅管理端，普通用户无账号登录。
- 当前10000是用户使用中的 `nekochat-live-10000-final`，不在实现过程中停止它；先在隔离实例验收。
- 主代理集成 `server/src/lib.rs` 路由和 `server/src/chat.rs` 中 Codex 认证/分发；图片执行者拥有其余 chat 改动，完成后交接。
- 图片执行者拥有 gateway `ChatMessage` 图片类型与已有 CC/Responses body，提供 `responses::input_messages(&[ChatMessage]) -> Vec<Value>`（crate可见）供 Codex 复用；OAuth 执行者不编辑 gateway/lib.rs，由主代理接入新模块导出。
- 图片初定 SQLite BLOB（本地数据目录内），对话范围鉴权、节点关联和级联清除。PNG/JPEG/WebP/GIF 魔数检查，单图上限8MiB、每轮最多5张；浏览器不把 base64 放响应式历史。
- `Protocol::ChatgptResponses` 区分官方订阅的严格参数边界（当前只允许 effort），复用原生 Responses 流。按当前官方 SIWC 动态注册、稳定 host ID、nonce/PKCE、JWKS ID-token/作用域验证接入公开 `api.openai.com/v1`，不使用旧 backend-api 或借用其他客户端身份。凭证0600、原子保存、单飞刷新、登出删除。
- 不显示精确模型/思考档位；按官方要求提供 ChatGPT 订阅使用说明与管理用量入口。未知费用/额度不伪造为0。

## Task 1: 图片全链路
- [x] 追踪现有输入、节点和清除逻辑；确认缺失的剪贴板/附件/多模态边界。
- [x] 增加受对话范围约束的上传/读取、节点附件关联；历史/编辑/重新生成都保留图片，清空/删除彻底清理。
- [x] 前端粘贴、拖放、选择、预览/移除及仅图片发送；失败恢复草稿；角色能力提示。
- [x] CC 使用 image_url，Responses 使用 input_image；文本保持原样。不支持视觉的角色在调用前明确拒绝。
- [x] 实际 Chromium 原生剪贴板 PNG → 本地预览 → Go gpt-6-luna 真实识别紫色圆形与橙色三角形；刷新后图片和回复仍在历史中。

## Task 2: 官方 ChatGPT OAuth
- [x] 读取当前 pi 源码与官方 SIWC 说明，确认动态注册与公开 API 契约。
- [x] Rust 实现授权开始/状态/回调或粘贴回调/取消/登出、私密持久化与自动刷新；管理端显示连接状态。
- [x] 实现独立 ChatGPT 订阅提供商和真实账号模型目录请求，管理端可上架并绑定角色；不自动改现有角色绑定。
- [x] 官方订阅 Responses 复用标准流事件与多模态输入；严格完成/错误/用量处理，以 OAuth bearer 调用公开 API，无旧 Codex 后台端点或账号专属伪造请求头。
- [x] 实际管理页发起官方 PKCE/nonce/state/direct-scope 授权、拒绝错误回调400、取消授权；未认证管理请求401，浏览器无错误。
- [x] 本人完成 OpenAI 授权；已取得实际账号模型目录，并保留用户配置的 `GPT酱 → chatgpt-subscription / gpt-6.1-sol / high`，完成一次真实官方订阅图片推理。

## 集成验收
- [x] 集成路由/协议分发/认证，迁移所有构造器和调用方；必要回归107例通过、workspace/tests类型检查及生产构建通过，未重复全套测试。
- [x] 隔离服务真实UI验收完成，并经用户明确选择「现在启用」切换10000；原配置、密钥环境、默认角色和对话保留。

## 实际交付状态
- 10000最终release服务已启用并保持运行；本次隔离10001实例、临时配置与浏览器验证页已清理。
- 公开角色仍为用户配置的 `test`、`大肥鱼`、`GPT酱`，未覆盖用户对角色名称等配置的修改。
- Go `gpt-6-luna` 视觉能力已在隔离实例真实验证，启用对应视觉元数据；未把其他模型的未知能力伪造成已验证。
- 官方订阅现已真实连通：浏览器原生粘贴 PNG，经 `GPT酱` 的官方 `gpt-6.1-sol` 正确识别紫色圆形/橙色三角形，状态 complete，4730无缓存输入/23输出。界面显示 Using ChatGPT plan 与 Manage usage，控制台无错误。
- 官方自建验证对话和图片已删除；正式实例用量记录保留。保留用户自行选择的模型、角色名称、人设、资源包和 high 参数，没有自动覆盖为其他模型。

Sources: https://github.com/earendil-works/pi/tree/main/packages/ai/src/auth ; https://developers.openai.com/siwc/token-sharing-open-source ; https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference
