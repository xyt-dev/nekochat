# 管理端与统计监控实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. 用户已要求快速实现，不做逐任务测试/审查循环；两个路径隔离的实现并行，最后统一构建和实际浏览器验证。

**Goal:** 在现有聊天基础上交付可用的 /admin 管理页面，以及真实用量驱动的花费、额度与时间 × token 监控。
**Architecture:** 复用现有 axum 管理鉴权、角色/模型 CRUD 和 SQLite usage；服务端按 SQL 聚合，不把原始记录全集搬到浏览器。管理页独立懒加载，复用 CSS 主题，图表用轻量 Canvas，不影响聊天包与流式路径。
**Tech Stack:** Rust / axum / rusqlite / reqwest；SolidJS / TypeScript / Canvas。
**Spec:** docs/superpowers/specs/2026-10-26-nekochat-design.md §10、§3.3、§6.2（本计划不实现表情配置、上传、压缩或尚无网关适配的协议）。

## Global Constraints
- 默认端口仍为 10000。不可读写用户 config.toml、data/，不可停止用户的 10000 或 3000 进程。冒烟使用 /tmp/nkadmin-* 与 >=10010 端口。
- 管理端始终需要 admin_password；未配置返回 404。API key/凭证不能经任何接口或页面返回；角色配置仅管理端可见。
- 花费优先使用调用时保存的 upstream_cost_usd，否则使用当时的 cost_usd（SQL COALESCE）；不能用今天的价格重算旧调用。未知不等于零，部分已知必须标明未定价请求数。
- output 不含 reasoning；总 token = cached_in + uncached_in + reasoning + output。
- OpenCode Go 配额是本机调用记录的估算，不是账户权威余额；不包含其他客户端用量。无法取得数据明确显示未知/不可用，不造数。
- 不做权限、用户账户、预算限制、自动重试、工具或订阅授权等额外功能。
- 不运行中途 build/lint/tests/formatters；两个实现结束后控制者只统一运行一次构建、针对性账务检查与实际浏览器场景。

## 文件归属与并行边界
- RustAdmin：crates/**、Cargo.toml / Cargo.lock。负责 SQL 聚合、监控 API、余额获取、模型默认值。不得修改 web/**、README、示例配置、计划文件。
- AdminWeb：web/**。负责 /admin 独立入口与样式、登录、角色/模型/默认设置、监控图表。不得修改 Rust、README、配置、计划。
- 控制者：README.md、config.example.toml、本计划、整体验证。共享工作目录 /home/xy6/CodeSpace/nekochat，分支 plan3-admin-monitoring。不要自行合并、推送或创建子代理。

## 共享 API 契约
现有角色/模型/设置接口保持原样，以 crates/server/src/api/admin.rs、store/catalog.rs 为准。/api/admin/* 不使用前端普通访问口令队列；管理页独立处理 401。POST /api/admin/session {password} 返回204。GET /api/admin/providers 仍为 [{key,label}]。

### 模型目录扩展
GET /api/admin/providers/{key}/models 每项保留 upstream_id、listed_model_id，增加 supported:boolean、defaults: ModelDefaults|null。
ModelDefaults = {display_name,protocol:"chat_completions",reasoning:{style:"none"|"effort"|"deepseek",levels:string[],default:string|null},vision:boolean,context_window:number|null,price:{cached_in:number|null,uncached_in:number|null,output:number|null}}。
只预填已核实的常见模型；未知值留空，未知模型可手动配置。已知不受当前网关支持的 Responses/Messages/Gemini 模型标 supported:false，不准误上架为 Chat Completions。管理页协议选择只开放真实实现的 Chat Completions，明确说明其余协议尚未接入，不假装可用。

### GET /api/admin/stats
查询：from_ms、to_ms（整数毫秒，左闭右开），bucket_ms（整数，>=60000）；省略时最近7天、每小时。无效范围400；最大366天、每模型最多1500桶，超出400，页面依区间选桶。
返回：
```
{from_ms,to_ms,bucket_ms,
 totals:{requests,cached_in,uncached_in,reasoning,output,cost_usd,unpriced_requests},
 models:[{key,provider,provider_label,upstream_id,model_label,
          totals:{requests,cached_in,uncached_in,reasoning,output,cost_usd,unpriced_requests},
          points:[{at_ms,requests,cached_in,uncached_in,reasoning,output,cost_usd,unpriced_requests}]}]}
```
key = JSON.stringify([provider,upstream_id]) 的等价稳定编码。cost_usd 是已知花费之和；全未知或无请求为 null；unpriced_requests 计 cost_usd NULL 的请求。桶按 Unix epoch 对齐；只返回非空桶，前端补零。模型包括已下架、改名或删除的历史调用（按 provider + upstream_id 分组，标签取最新记录）。统计仅在管理 API 返回，无对话/内容。前端按 provider/model 过滤，选择 token 类型（总计、缓存输入、无缓存输入、思考、输出），颜色按稳定 key 分配。

### GET /api/admin/quotas
返回：
```
{as_of_ms,providers:[{key,label,kind:"deepseek"|"opencode_go"|"unknown",
 status:"ok"|"unavailable"|"error",note:string|null,
 balance:{available:boolean,balances:[{currency,total,granted,topped_up}]}|null,
 models:[{upstream_id,model_label,windows:[{name:"5h"|"7d"|"30d",limit_usd:number|null,used_usd:number|null,remaining_usd:number|null,unknown_requests:number}]}]}]}
```
余额各金额字符串，保留币种，不折算 USD。DeepSeek GET /user/balance，使用服务端 api_key，设置有限超时，错误不泄露上游原始响应/key。未知提供商返回 unavailable。
OpenCode 从 usage.allowance_usd 累计，分别最近5小时/7天/30天，明确标“本机滚动窗口估算”；remaining=max(0,limit-used)，有 allowance 缺失则 used 为已知部分并附 unknown_requests，页面不可把 remaining 当确定余额。没有额度计数且有请求时 used/remaining 为 null。覆盖仍上架模型以及最近30天曾用模型。默认 Go（不是 Go Plus），明确 note，配置可切换或覆盖限额。

### 提供商可选配置
ProviderEntry 增加可选 monitoring，旧配置保持兼容：
```
[providers.monitoring]
kind = "deepseek" # 或 opencode_go
# balance_url = "https://api.deepseek.com/user/balance" # 可选覆盖
# plan = "go" # 或 go_plus
# monthly_limits = { "my-model-id" = 60.0 } # OpenCode 可选覆盖
```
省略 monitoring 时只根据官方 base_url 主机名 api.deepseek.com / opencode.ai 识别，其他 unknown。DeepSeek 默认余额 URL 从官方 base 派生；自定义 base 需明确 kind/balance_url。OpenCode 官方说明各模型5小时20%、每周50%、每月100%。内置常见 Chat Completions 模型当前公开限额，未知模型不猜测。来源 https://opencode.ai/v2/docs/console/go/；DeepSeek https://api-docs.deepseek.com/api/get-user-balance 。

## Task 1: Rust 统计、额度与模型预填
- [x] 扩展兼容配置、SQL 聚合和管理路由，不改变聊天数据与普通用户API。
- [x] 余额适配与 OpenCode 本地滚动额度、上架目录默认值。
- [x] 保留少量确定性账务边界回归：多模型同名跨提供商、缺价/零价、左闭右开、桶边界、reasoning不重复计费、缺失额度；不写字段转发或文案测试。

## Task 2: 管理网页
- [x] /admin 独立管理密码页，404显示配置说明；鉴权失败不泄露已有页面数据。
- [x] 角色列表/新增/编辑/删除与默认角色；模型提供商目录刷新/搜索/勾选上架、编辑字段与下架，默认模型；破坏操作确认。
- [x] 统计区间（24小时/7天/30天/自定义）、提供商/模型过滤、分色 Canvas 时间×token 图与悬停数据、费用表（清楚标缓存输入/无缓存输入/思考/输出）、额度卡片、错误/无数据状态。
- [x] 复用主题切换，窄屏可用；不在聊天页添加角色/管理数据，不向 localStorage 存管理密码。

## Task 3: 集成交付
- [x] 统一构建；临时 upstream + 临时数据库通过真实聊天写用量，核对费用与未知额度语义。
- [x] 实际 Chromium 验证管理密码/角色/模型上架/默认设置/统计图/余额/主题，回到聊天验证管理变更可见。
- [x] 更新 README 与示例配置，移除临时脚本与自有进程，不碰用户数据。

## 验证记录

- WASM 构建、TypeScript 类型检查、Vite 生产打包及 Rust 服务端构建通过。首次集成编译发现 SQLite 的 u64 解码与空 Set 泛型问题，分别改为检查型 i64→u64 转换与显式 Set<string>。
- 仅运行相关账务/窗口、监控参数、配置、负账务值检查：14 项通过，未重复执行全工作区测试。
- 临时服务通过真实聊天路径写入四次调用：缓存输入 320、无缓存输入 160、思考 20、输出 60；已知费用 $0.2504，另有 1 次未定价。免费请求记为 $0；下架模型、彻底删除对话后历史统计仍保留，且不含内容。
- 模拟 DeepSeek 官方余额响应保留 CNY 123.45 与 USD 6.50 原币种精度；Go 的 0.5 allowance 在月额度60时得到 5h/7d/30d 估算剩余 11.5/29.5/59.5。未知适配器显示不可用。此处使用临时上游响应，不代表已查询真实账户。
- 实际 Chromium：错误密码、正确登录、服务重启后的会话失效与数据隐藏；角色新增/编辑/默认设置、删除取消与确认；模型勾选上架/编辑/默认设置；官方 OpenCode GPT/Claude/Gemini 协议禁选；统计预设/自定义区间/提供商与模型过滤/输出指标键盘查看；亮暗主题；390px 页面无横向溢出。修复并验证刷新统计后原过滤选择保持可见。
- 回到聊天页，新模型、max 默认档与管理端角色立即生效，公式与代码正常渲染，无控制台错误；聊天页未加载管理包、未暴露角色控件，管理密码未存 localStorage。
- 实机手机和真实付费上游未验证；未重跑既有 500 消息压测。聊天流式路径没有修改，管理页独立懒加载。
