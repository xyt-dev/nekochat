# 0.7.0 用户、头像、编辑器与统一附件

**Goal:** 完成用户本轮逐项要求，保留角色/真实官方GPT授权/旧会话；所有新聊天必须登录且按用户ID隔离。
**User decisions:** 必须登录。密码存储有切换按钮、默认明文；哈希模式不可逆，切回明文后旧哈希在成功登录或管理员重置时转换。模型没有默认项，只有默认角色；用户记忆最后角色和对话。
**Spec:** 既有 `docs/superpowers/specs/2026-10-26-nekochat-design.md` 的角色、资源本地化、附件和性能边界，按本轮账号需求扩展。

## 公共契约与所有权
- 所有执行者先读 Agent.md。跳过中途 build/test/lint/formatter；主代理集成后一次必要检查与实际UI/API验证。执行者不提交。CHANGELOG由主代理在每次提交前更新。
- A（AccountCore）拥有账号持久化/会话/归属/头像Rust后端、store/lib.rs及schema迁移、tree/catalog、server/auth.rs、api/user.rs、api/admin.rs、测试公共fixture。初次集成为schema v4；最终增加schema v5凭据版本迁移，阻止密码重置/停用与旧验证并发时恢复会话。
- B（DocumentCore）拥有store/attachments.rs、server/attachments.rs、server/chat.rs及文档上下文（完成后把chat交主代理集成）、相关测试；向A提交新增附件列SQL。不得自行改schema版本。
- C（ChatSurface）拥有所有web公开聊天App/api/store/components/styles/themes及bench；不改admin目录。
- D（AdminSurface）拥有web/admin目录用户管理、存储模式、角色头像及移除默认模型控件；不改公开聊天文件。
- 主代理拥有server/lib.rs路由集成、Cargo/package版本和新增依赖、CHANGELOG/Agent文档、最终验证与部署。其他代理通过消息报告依赖和精确路由，不同时修改这些文件。

## 数据与API
- UserPublic: `{id, username, avatar: string|null, last_character_id: string|null, last_conversation_id: string|null}`，无密码。
- 会话token为随机不透明值，服务端保存指纹；cookie HttpOnly。注册/login/退出、改名/密码/头像、停用/删除与失效会话全部真实实现。
- 建议路由：GET `/api/account/session` -> UserPublic|null；POST `/api/account/register`、POST `/api/account/login` -> UserPublic并设cookie；DELETE `/api/account/session`；PATCH `/api/account`；PUT `/api/account/preferences`；POST `/api/account/avatar` -> `{url}`。
- 管理端：GET/POST `/api/admin/users`；PATCH/DELETE `/api/admin/users/{id}`；POST `/api/admin/users/{id}/password`显式查看可恢复密码；POST `/api/admin/users/{id}/legacy-conversations`分配未归属旧会话；GET/PUT `/api/admin/user-settings` -> `{password_storage:'plain'|'hash',legacy_conversations:number}`；POST `/api/admin/avatars` -> `{url}`；GET `/api/avatars/{id}`安全读取。
- A定义 `accounts::CurrentUser`（至少公开id/username/avatar）并由认证middleware插入Extension；定义异步 `accounts::require_conversation(&AppState,&CurrentUser,&str)->Result<(),ApiError>` 给附件handler调用。
- Rust通道起轮必须接收可信actor user_id并校验归属，不能只依赖Web页面。A和B约定精确 `start_turn` 参数与Store归属方法。
- conversations.owner_user_id 归属，旧记录保持NULL，仅管理员可分配。用户只列出自己的数据；越权ID返回404。禁止第一个注册者自动取得管理员/全部旧历史。
- 用户偏好服务端保存，写入前校验角色可用、会话归属；恢复优先自己的有效last选择，退回默认角色。删除会话/角色清掉对应last引用。移除公开SettingKey/Settings中的default_model_id；历史v2迁移只保留必要的原始SQL兼容，不保留运行时旧设置路径。
- CharacterSpec/公开角色新增 `avatar: string|null`；NodeView/Live start携带 `character_avatar: string|null`供历史/流式头像，用户消息使用当前登录者头像。头像本站上传、PNG/JPEG/WebP≤2MiB，默认首字占位，不依赖外链。
- Attachment统一 `{id,name,kind:'image'|'document',media_type,size,url}`。已有图片自动迁移为image/合理默认名。同一个POST `/api/conversations/{id}/attachments`收原始bytes，header `x-file-name`为encodeURIComponent(file.name)，Content-Type仅辅助；必须仍校验归属及内容。
- 图片保持PNG/JPEG/WebP/GIF≤8MiB；文档≤16MiB，UTF8文本/Markdown/常见代码/JSON/CSV等以及PDF，提取文本上限2MiB，超限或损坏明确报错不截断。总附件每消息≤5；非图片不能进入image_url/input_image。PDF没有可提取文本时明确提示，不假装已读取。
- 文档原始bytes和提取文本本地SQLite保存；上下文将文本作为所属用户消息的一部分，非系统指令；历史、编辑、重新生成保留；清空/删除含未发送上传全部删掉。

## UI验收
- 聊天端删除Using ChatGPT plan/Manage usage，管理端保留真实来源和用量入口；默认Claude Dark但保留已有主动保存的偏好。
- 轻量contenteditable编辑器内展示图片，可粘贴/拖放/选择、删除、只图发送、编辑恢复；发送后图片和文字留在同一气泡，不溢出。文档卡片排编辑框上方，名称/大小/移除/下载清楚。
- 主题选择器右侧为头像菜单：用户信息编辑、退出。注册/登录、修改用户名/头像/密码、退出后不保留上一人的聊天状态。
- 管理端用户列表/创建/编辑/停用/删除、显式密码查看、明文/哈希切换及不可逆提醒；旧对话分配入口。角色编辑器头像上传/预览/清除。
- 模型页移除默认模型；角色页默认角色保留。每个登录账号恢复自己的上次会话和角色，不靠全站localStorage混用。

## 验证与版本
- [x] A：用户/模式迁移、登录会话、资料、归属、偏好、头像及旧数据迁移。
- [x] B：统一上传、PDF/文本提取、图片/文档区分及完整上下文、删除边界。
- [x] C：聊天登录、头像菜单、角色恢复、编辑器/消息布局、默认主题及去掉套餐条。
- [x] D：用户管理、密码模式、角色头像、默认角色与无默认模型。
- [x] 主代理：集成路由/签名/依赖/0.7.0版本；必要回归与真实多用户UI/API场景。
- [x] 按实际提交回填历史功能版本，完成CHANGELOG与Agent要求；经用户确认安全启用10000，保留官方凭证和现有数据。
- [x] 用户补充：默认宿主机监听0.0.0.0:10000，保留Host白名单；管理密码在配置中自行修改，不替换现有值。

## 最终证据
- 相关Rust回归与TypeScript/WASM/Vite、release构建通过；新增凭据竞态和公开角色隐私回归先失败后通过。
- 实际浏览器验证：注册/登录、资料/头像、角色头像、管理端用户创建/重置/停用/启用/删除、明文/哈希切换、用户隔离与偏好恢复、图片/文档布局。
- 实际Go GPT调用识别图片并读取Markdown/PDF中的独立校验码；停止信号的真实HTTP流验证从继续接收15段修复为首段后中断。
- 浏览器复现并验证修复：错误原密码保持登录、新建请求等待期间保留草稿、选中文字替换为图片后只发送可见内容。
- 10000已更新并持久运行；实际访问 `http://192.168.31.40:10000`，默认Claude Dark登录页正常；未知Host返回421，匿名对话访问返回401。
- 迁移前私密备份：`/home/xy6/.local/state/nekochat/backups/pre-0.7.0-20261028-276932`。原11张表的数据（除按要求移除旧默认模型设置）、4条未归属会话与官方GPT凭证完整保留；未给真实账号自动分配旧历史。
