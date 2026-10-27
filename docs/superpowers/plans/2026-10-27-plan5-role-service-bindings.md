# 大肥鱼与 GPT 娘真实服务接入

**Goal:** 在现有实例创建两张角色卡，分别使用真实 DeepSeek 与 GPT 服务；补齐必要的 OpenAI Responses 协议，不把 GPT 娘绑到 DeepSeek 冒充。
**Spec:** docs/superpowers/specs/2026-10-26-nekochat-design.md §4.4/§8/§10；用户要求现在建立两张角色卡。

## 已核对
- 上游 src/index.ts PERSONA.self.skill 是大肥鱼身份原文；GPT 娘仅有白发龙娘、龙角、OpenAI 发饰及双人方向说明，没有独立完整性格人设。两卡以这些事实适配，额外语气规则属于本项目，不冒称原作者全文。
- 现有本地实例10000已运行新版角色API，保留用户 test 角色/默认设置/背景/对话。
- 配置有 DeepSeek 与 OpenCode Go；目前仅上架 DeepSeek V4.1 Flash。Go实时目录有 gpt-6-luna，原生服务使用 /responses。

## 实现边界
- 不显示/写出 API key、管理密码；配置仅用于私密认证，不改配置文件或重置数据。通过管理API创建角色，而非直接写运行中数据库。
- 不未经同意停止用户10000进程。先构建与协议回归，再在用户明确选择「现在切换并完成两张卡」后安全启用；保留原进程环境、配置与工作目录。
- 保留本地资源、角色专有参数与客户端不能覆盖模型的边界；不接 Claude/工具/OAuth/自动重试。
- 不反复全套测试，仅相关回归和真实短聊天验证。

## Task 1: 原生 Responses
- [x] core Protocol::Responses，支持与限制按协议校验；Responses不接受seed、frequency_penalty、presence_penalty、stop或DeepSeek思考模式。
- [x] gateway 新 responses.rs：store:false、stream:true，原生 input/system、reasoning.effort、max_output_tokens、可选temperature/top_p；发自身User-Agent和稳定会话头，不带tools。解析文字、拒绝文字、思考增量及 response.completed 用量；failed/incomplete/EOF明确报错，不把中途断流当完成。复用标准化Usage/StreamEvent，Go计费/额度优先其原始字段。
- [x] server 依model.protocol分发，目录允许Go GPT且预填Responses；上架时拒绝协议族误配。管理端协议编辑器真实开放Responses，角色编辑器不误导设置该协议不支持的参数。

## Task 2: 两卡
- [x] 大肥鱼：DeepSeek鲸鱼娘身份，自身表情+双人方向；绑定真实DeepSeek模型，self/often，两本地包，无背景默认。
- [x] GPT 娘：GPT白发龙娘身份，双人方向反过来以GPT为本人；绑定Go gpt-6-luna原生Responses，self/often，双人包（可用大肥鱼图只作他者，不冒认单人形象），默认无背景。
- [x] 不覆盖用户既有角色或默认设置。名字冲突先核对是否本次建立，不能按显示名盲覆盖。

## Task 3: 核验
- [x] Responses正常/失败/中断/计费/拒绝文字回归、既有CC不退化、前端类型检查与构建，以及实际10000服务SSE实跑。
- [x] 安全启用新服务后，通过管理API落地两卡并验证公开列表。每卡只做一次简短真实服务调用，保留计费记录，不假装模拟上游是真模型。
- [x] 更新README协议说明和结果记录，清理自有验证资源；用户服务不擅自停止。

## 落地结果
- 实例：`http://127.0.0.1:10000`，最终 release 服务保持运行；原有 `test` 角色仍为默认，未改用户配置文件或清空用户数据。
- 大肥鱼：OpenCode Go / `deepseek-v4.1-flash` / Chat Completions；本地 `nekowhale` + `nekowhale-gpt` 包。
- GPT娘：OpenCode Go / `gpt-6-luna` / 原生 Responses；本地 `nekowhale-gpt` 包。两卡默认不设背景。
- 真实调用分别返回鲸鱼娘、白发龙娘身份介绍并完成计量；两次自建验证对话均已删除，监控用量记录保留。
- 实际浏览器确认角色选择器存在两卡，不显示底层模型/提供商或思考控件，控制台无错误；最终服务公开列表仍可访问。
- 验证通过：`bun run build`、`cargo check --workspace --tests`、core parameters 3例、gateway Chat Completions 19例 + Responses 5例、server admin/role_turns 8例；最终 release 构建成功。只跑相关回归，未重复全套测试。

Sources: https://github.com/Loping151/dsh-nekowhale-stickers/blob/main/src/index.ts ; https://opencode.ai/v2/docs/console/api/inference/ ; https://developers.openai.com/api/reference/resources/responses/methods/create
