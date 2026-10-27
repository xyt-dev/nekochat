# 角色卡片与本地 skill 资源包实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. 沿用用户已选择的子代理与快速实现，不重复讨论/逐项审查；路径隔离并行，集成后只做针对性验证。

**Goal:** 将聊天入口从模型改为完整角色卡，隐藏模型与思考档位；真正接入指定 dsh 大肥鱼和 GPT 娘资源，并使角色与资源核心可被微信等通道复用。
**Spec:** docs/superpowers/specs/2026-10-26-nekochat-design.md，以下最新用户要求覆盖其 §2/6 中旧的模型选择和角色不可见规则。

## 最新要求与决定
- 前端仅选角色，角色由管理员配置：内部名称 name、前端 display_name、实际 model_id、模型参数、system_prompt、可选半透明背景、勾选的资源包。聊天端不能选/显示思考等级，不能收到底层模型/参数/提示词。思考内容折叠、时长与 token debug 仍保留，等级不是时长或 token 数。
- 参数先实现实际支持的字段：effort、temperature、top_p、max_output_tokens、seed、frequency_penalty、presence_penalty、stop。严格类型与有效数值，不能用原始 JSON 覆盖 messages/model/tools/认证。
- 角色切换保留当前消息树，从下一轮开始用新角色与模型；每条助手节点记录实际所用角色及显示名快照。角色切换不重写旧回复，重新生成/编辑使用当前选择角色。管理端角色更新从下一轮生效。
- 资源全部本地：仓库预置完整两套 322 张 GIF 与原标注、授权署名。构建产物含资源；运行时没有远程拉图、外链图片、CDN 或静默替代。导入工具只供显式维护更新，不由聊天调用。上游资源地址只存在导入工具/来源说明，不用于浏览器加载。
- skill 目录仅是资源包单位，不加载上游工具执行逻辑。SKILL.md 提供静态指引；pack.json 与 stickers.json 提供结构化数据；assets/ 存本地素材。角色勾选的包才进该次系统提示词和表情解析范围。
- 沿用原设计准确选图：目录逐行名字/情绪/适用场景/方向，模型写 [sticker:名字]，精确优先，唯一模糊匹配，未知移除；每回复最多一张，近几轮去重，绝不随机同情绪挑图。缓冲跨 chunk 半截标记，不泄露半标记；代码围栏中的字面示例保持文本。
- 背景默认 null。不用滤镜/backdrop 动画；单个不响应 token 更新的独立装饰层，以 opacity 实现半透明，pointer-events:none，不影响文本可读性和虚拟列表。
- 保留单 Rust 服务、SQLite 树、现有监控与 CSS 主题。默认10000，不碰用户 config.toml、data/、10000/3000进程。验证使用 /tmp/nkrole-*、>=10030端口。无模型工具/脚本/文件系统授权。
- Web 适配与核心分离：资源 crate 不依赖 axum/URL/SSE，输出 Text/Sticker 的中立片段；聊天起轮服务可直接从 Rust 调用，HTTP/SSE只包装该服务。以后微信使用同一角色 id、参数与授权解析，不靠 Web DOM，也不复制提示词策略。本次不实现任何 bot。

## 归属
1. **LocalPacks**：crates/resources/**、resource-packs/**、scripts/import-nekowhale.*。实现资源中立接口、导入下载完整素材与许可。不得改根Cargo、现有crates或web。
2. **RoleServer**：crates/core、store、gateway、server、mdwasm；根Cargo.toml/Cargo.lock。实现角色存储迁移、模型参数、通道独立起轮入口、HTTP映射与本地素材路由。将 crates/resources 加到 workspace/服务器依赖。不得改资源crate内容或web。
3. **RoleWeb**：web/**。完成聊天角色选择/背景/隐私与管理角色编辑器/资源包管理视图。不得改Rust或resource-packs。
4. **控制者**：README、配置例子、设计/计划、集成与实际浏览器验证。
不提交/合并/推送，不创建子代理。中途不运行 build/tests/lint/formatter，最后控制者统一运行。LSP当前无服务，导出改动要做完整调用点迁移；不要用兼容别名维持旧前端模型/等级路径。

## 数据与 API 共享契约
### 管理 Character
GET/POST/PUT /api/admin/characters 的对象：
```
{id,name,display_name,system_prompt,model_id:string|null,
 parameters:{effort?:string|null,temperature?:number|null,top_p?:number|null,max_output_tokens?:number|null,seed?:number|null,frequency_penalty?:number|null,presence_penalty?:number|null,stop?:string[]},
 enabled:boolean,sort:number,background:{url:string,opacity:number}|null,
 resource_packs:string[],sticker_identity:"self"|"neutral",sticker_frequency:"rare"|"normal"|"often",
 created_at,updated_at}
```
POST/PUT body不含id/timestamps。新建 name/display_name 必填，其他服务端默认 parameters={}、enabled=true、sort=0、background=null、resource_packs=[]、identity=neutral、frequency=often、model_id=null（未绑定模型的角色不出现在聊天列表，编辑器提示）。模型绑定必须真实上架，effort必须在模型档位内，null用服务端模型默认档。全部生成参数不向普通API暴露。
迁移至schema2保留所有旧数据：display_name=name，旧角色 model_id 取当前默认模型（存在时），参数空，资源/背景空；历史节点不再暴露 model_label/effort。没有旧角色且配置了默认模型时不伪造角色，用户到管理端配置。字段保存所有调用点/测试适配，不重置数据库。

### 聊天公开 API clean cutover
GET /api/characters → {default_character_id,characters:[{id,display_name,background:{url,opacity}|null}]}，只列enabled且绑定模型可用的角色。移除普通 /api/models 路由，不提供模型字段或档位。
POST /api/conversations JSON {character_id?:string}，缺省用可用默认角色，无默认时可选第一个可用角色；没有可用角色明确400提示管理配置。
ConversationSummary增加character_id；ConversationView增加character_id。
POST /api/conversations/{id}/turns JSON {action:"send"|"edit"|"regenerate",content?/node_id?,character_id:string}。不接受 model_id/effort/parameters 等额外字段（400而不是忽略）。只从已授权服务端角色解析实际模型/参数/资源。
NodeView删除 model_label、effort；新增character_id:string|null、character_name:string|null，角色历史显示名快照，历史未知显示助手；其余tree/metrics字段不变。SSE start 将 model_label 改为 character_name，附character_id。reasoning/text/done/error结构其余保持。
资源标记在服务端解析后变为本地Markdown图片 ![名字](/api/resources/<pack_id>/<asset_id>)，普通 text事件仍使用 {t}，done保存已经解析的正文。资源crate先发中立 ResourceChunk::Sticker，再由Web适配格式化成Markdown。future微信可直接把同一ResourceRef转为图片消息，而非理解HTML。历史上下文可保留已处理的图片Markdown，不发送思考内容。

### 资源包管理和素材
GET /api/admin/resource-packs → [{id,name,description,sticker_count,skill,attribution,assets:[{id,name,url}]}]，素材 url 只为本站 /api/resources/...；由内置包与 <data_dir>/skills/<pack_id> 本地目录组成，不通过浏览器任意写路径。可查看目录和本地素材，勾选授权在角色编辑器。额外包安装只需管理员放到本地skills目录，重启加载；不做网络导入/脚本执行/自动标注这一轮未请求的功能。
GET /api/resources/{pack_id}/{asset_id}：精确清单查找，无任意路径、目录穿越、symlink逃逸；只返回本地安全图片，访问口令gate覆盖。无外部重定向或热下载。
POST /api/admin/backgrounds：Content-Type image/png|jpeg|webp|gif 原始字节，<=5MiB，检查签名，不收SVG/HTML；存 <data_dir>/backgrounds/<uuid>.<ext>，返回 {url:"/api/backgrounds/<id.ext>"}。
GET /api/backgrounds/{file}：同样安全文件名/图片类型/访问口令边界。
角色 background.url 仅接受上述本站背景URL，或现有本地资源包assets URL。opacity默认0.15，范围0..1；外部URL/协议相对URL/..必须400。
Markdown渲染允许上述本地图片路径，拒绝外链图片（可保留alt），不放宽rawHTML/危险scheme。图片GIF原样播放，max宽高限制，loading=lazy/decoding=async。

## 资源 crate 精确接口（LocalPacks 与 RoleServer 对齐）
包名 nekochat-resources（Rust2024），导出下列类型/方法；增加辅助方法可直接相互消息：
```
pub struct ResourceCatalog; // 内置嵌入 + 本地skills目录，加载一次，不含网络
impl ResourceCatalog {
 pub fn load(custom_dir: &std::path::Path) -> anyhow::Result<Self>;
 pub fn packs(&self) -> Vec<PackView>; // PackView可Serialize，含上述列表字段（assets URL由server映射，不由crate硬编码）
 pub fn contains_pack(&self,id:&str)->bool;
 pub fn prompt(&self,allowed:&[String],identity:&str,frequency:&str)->anyhow::Result<String>; // 固定排序、字节稳定
 pub fn asset(&self,pack:&str,id:&str)->Option<ResourceAsset>; // bytes::Bytes 共享缓冲，mime，安全已验证
 pub fn filter<'a>(&'a self,allowed:&[String],recent:&[String])->anyhow::Result<StickerFilter<'a>>;
}
pub enum ResourceChunk { Text(String), Sticker(ResourceRef) }
pub struct ResourceRef {pub pack_id:String,pub asset_id:String,pub name:String} // Serialize/Clone
impl StickerFilter<'_> {
 pub fn push(&mut self,text:&str)->Vec<ResourceChunk>;
 pub fn finish(&mut self)->Vec<ResourceChunk>;
}
```
ResourceAsset {pub bytes:bytes::Bytes,pub mime:&'static str}；内置文件由静态字节零拷贝包装，自定义本地文件只在加载时转为共享缓冲，HTTP 请求克隆句柄，不复制整张图片。PackView至少id/name/description/sticker_count/skill/attribution，asset信息另用pub assets Vec<AssetView>，AssetView id/name；server补URLs，不让crate依赖http。
recent 为最近几条回复已经使用的规范 pack_id/asset_id；过滤器不允许未授权资源。支持精确上游完整id、编号_名字、名字别名，歧义不随机。内置包ID nekowhale / nekowhale-gpt。资产id用稳定ASCII（建议hash加.gif），原始知识库字段不丢。

## 并行任务与验收
- [x] LocalPacks：参考 https://github.com/Loping151/dsh-nekowhale-stickers 的 meta/nekowhale.json 与 SKILL.md、src/core/sync.ts、NOTICE/LICENSE/LICENSE-ASSETS。两套共322张，使用站点 /api/kit/nekowhale 的 manifest 准确匹配id/文件并校验hash。全部落盘本地并嵌入，运行时网络依赖为零。文件丢失/坏hash不能以假图或删减数据“完成”。保留作者B站可以叫我云大人/雾雪、kiriyuki.com、形象作者与非商用限制。
- [x] RoleServer：完整schema迁移；角色配置CRUD、parameters接上真实gateway；移除客户端模型/等级覆盖；本地资源/背景、稳定提示词、逐块过滤与近轮去重；通道独立起轮服务；监控账务仍记录真实模型，而非角色名代替计费键。资源中立策略不放在Web组件。
- [x] RoleWeb：仅角色选择（没有提供商/模型/思考等级），本地背景装饰层，消息上方角色显示名；取消遗留nk.model/nk.effort缓存与API类型；保留思考折叠/时长/debug。角色管理含绑定模型、各参数、公开名、背景上传/清除/透明度、资源包复选框/预览、身份/频率、上架开关/排序。模型管理仍为管理员维护，不改变监控。资源包视图显示本地两套计数/署名/素材预览。移动端和主题沿用。
- [x] 控制者：统一编译和少量新行为回归（安全授权/跨chunk/原有schema迁移/参数不被用户覆盖）；实际Chromium使用本地资源不发第三方图片请求，选择角色聊天、图片背景和表情显示、树切换/重新生成、管理更新生效；更新README和主设计的被覆盖规则；清理自有临时服务，不动用户进程。

## 集成验证记录

- `bun run build`（WASM、TypeScript、Vite）通过；`cargo check --workspace --tests` 检查所有 Rust 测试目标；相关 Rust 行为测试 49 项和本地图片 URL 安全测试 1 项通过，未重复跑整套测试。
- 用临时服务与模拟上游走真实聊天：仅给客户端角色 id/公开名/背景；旧 `/api/models` 404、模型/effort 覆盖 400，管理员的八类参数送达 native 请求，真实模型计费键不被角色名替代。
- 普通与双人标注分别 286/36，总322张真实 GIF；精确路径已加载，跨3字符 chunk不泄露半标记，未授权双人表情移除，近轮不重复，无资源角色不出图。模拟上游验证策略与传输，不声称已验证真实付费模型的选图质量。
- 实际 Chromium：GPT 娘与小雪切换、历史角色名快照、本地GIF显示、上传本地背景并保存22%透明度、切换无背景角色时清除装饰；资源包预览仅请求本站图片，未观测到第三方资源请求。
- 修复流式转已保存节点时短暂产生重复 key、令 ResizeObserver 跟踪已移除节点而导致图片回复重叠的问题：完成/中断状态切换使用 Solid batch 原子更新；双图片回复实测位置不重叠。
- 修复新对话请求未完成就发送消息时落入旧对话的竞态：立即脱离旧上下文并共享新建请求。人为延迟新建响应300ms后立即发送，只新建一次，新对话正确保存2节点。
- 修复资源 crate 的 `debug-embed` Cargo 特性统一导致网页 debug 资源意外固定的问题；网页 debug 显式读磁盘，release仍嵌入。修复后服务器实际返回当前 Vite 构建哈希，而非旧哈希。
- `/tree` 键盘打开、回复定位、当前角色重新生成、‹ › 回到历史角色分支均已实际执行。修复打开命令的 Enter 冒泡到新建树面板，立即触发关闭的问题。390px 页面无横向溢出，聊天无底层模型和思考等级控件。
- 4× CPU 降速、500条含公式/代码/本地表情历史、100 tok/s：单聊天标签页下，主线程 >50ms 长任务0，跟随与历史滚动约60.1fps。第一次测量在同浏览器仍打开资源预览页时出现1次57ms长任务；不把“所有多标签负载下绝无卡顿”当作已验证结论。真实手机未验证。
- Rust 直接调用 `chat::start_turn` 的回归验证了不经 HTTP/SSE 也能按相同角色/参数运行。没有实现或假装实现微信 bot。
- Release 构建通过；从项目目录之外运行 release 二进制，未配置任何模型提供商，两套内置 GIF 均可直接读取，SHA-1 前缀分别匹配 `4a44767ab58b` 和 `222876e49285`，网页也从二进制正常提供；不依赖运行时资源下载。
