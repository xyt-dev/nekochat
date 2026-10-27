# nekochat

自托管的角色扮演网页聊天。

## 运行

1. 复制示例配置：

   ```sh
   cp config.example.toml config.toml
   ```

2. 按需修改 `config.toml`，并为每个提供商设置 `api_key_env` 指定的环境变量：

   ```sh
   export DEEPSEEK_API_KEY=...
   export OPENCODE_API_KEY=...
   ```

3. 构建网页前端（需要 bun、`wasm32-unknown-unknown` 目标和 0.2.129 版的 `wasm-bindgen` CLI）：

   ```sh
   cd web && bun install && bun run build && cd ..
   ```

   产物在 `web/dist`。release 构建会把它嵌进二进制；debug 构建运行时直接读磁盘上的 `web/dist`。

4. 启动，然后用浏览器打开 `http://127.0.0.1:10000`：

   ```sh
   cargo run --release -p nekochat-server -- serve --config config.toml
   ```

默认监听 `127.0.0.1:10000`。监听非环回地址时请设置 `access_password`，并放在 HTTPS 反向代理之后（配置 `trusted_proxies`）。

为防 DNS 重绑定，服务只接受 `Host` 为 `localhost`、`127.0.0.1`、`::1` 或监听地址本身（监听具体 IP 时）的请求，其他一律返回 421。通过反向代理或 Tailscale 主机名访问时，把这些主机名（不带端口）写进 `allowed_hosts`，例如 `allowed_hosts = ["chat.example.com"]`。

## 管理端与监控

在 `config.toml` 设置 `admin_password` 后，访问 `http://127.0.0.1:10000/admin`。未设置管理密码时管理 API 不开放；管理密码独立于可选的网页访问密码。

- **角色卡**：绑定实际模型、生成参数、提示词、公开显示名、可选本地半透明背景，以及角色可用的资源包。前端只选择整套角色，不再选择或显示底层模型、提供商、思考等级。管理员修改配置后下一轮生效；用户切换角色保留消息树，旧回复保留当时的角色名称。
- **模型**：按提供商刷新上游模型目录，选择上架哪些模型，设置显示名、排序、思考档位与默认档、图片能力、上下文窗口，以及每百万 token 的美元价格。API key 仍只在服务端环境变量中管理，网页不能查看或修改。支持 Chat Completions（DeepSeek 等）和原生 Responses（OpenCode Go 的 GPT 等）；Messages / Gemini 尚未接入。Responses 不接受 seed、penalty、stop 或 DeepSeek thinking 参数，不能靠错误协议冒充支持。
- **统计**：按时间、提供商和模型查看费用与 token 用量；图表颜色区分「提供商 + 模型」，可分别查看缓存输入、无缓存输入、思考和输出。输出数不含思考，图中的总计包含这四类。费用保留调用当时的记录，改价或下架不会改写历史。
- **未知费用**：未设置价格且上游未返回费用的请求标记为未定价，不当作免费。部分已知时显示已知费用和未定价请求数。
- **额度**：DeepSeek 查询官方余额，按原币种显示；OpenCode Go 根据本机记录的 `opencode.allowance` 估算最近 5 小时、7 天、30 天的用量。它不是账户的权威余额，不包含其他客户端的调用，滚动窗口也不代表服务商实际重置时刻。缺失额度计数或限额时显示未知，不伪造剩余数。

官方 `api.deepseek.com` / `opencode.ai` 地址可自动识别监控类型；自定义代理地址需要在提供商的 `monitoring` 中指定。OpenCode 默认按 Go 计划，可配置 `plan = "go_plus"`，或用 `monthly_limits` 按上游模型 ID 覆盖月额度；5 小时与每周额度分别按月额度的 20% / 50% 估算。示例见 `config.example.toml`。

额度与余额接口来源：[OpenCode Go](https://opencode.ai/v2/docs/console/go/)、[DeepSeek 余额](https://api-docs.deepseek.com/api/get-user-balance)。价格和计划可能变化，上架前请核对服务商当期信息。

## 本地 skill 资源包

内置 [dsh-nekowhale-stickers](https://github.com/Loping151/dsh-nekowhale-stickers) 的「大肥鱼」「大肥鱼和 GPT 娘」两套标注与完整 322 张 GIF。图片随程序本地提供，浏览器不向作者站点或 CDN 拉图，服务运行时也不下载素材。

内置图片约 388 MiB，会增加 Rust 二进制体积，但不会进入网页 JS 包或模型提示词；内置资源按静态字节共享，自定义资源请求也不复制整份图片缓冲。

管理员在角色编辑器中勾选资源包。服务端仅把已勾选包的静态说明和表情目录放进该角色的提示词；模型按描述输出 `[sticker:名字]`，服务端精确解析成本站图片。未授权、未知、歧义或近轮重复的表情不会随机替代，半截标记不发到前端。身份 `self` / `neutral` 和频率由管理端设置。

大肥鱼身份提示词可参考上游 `src/index.ts` 的 `PERSONA.self.skill`；GPT 娘在原仓库中有形象与互动方向资料，但没有独立的完整性格提示词，需据此适配。GPT 娘绑定 GPT 服务时要选 Responses 协议，不要把角色名称当成模型身份或把它实际绑到 DeepSeek 后冒称 GPT。两张角色卡均可由管理员继续编辑，原有默认角色无需改动。

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

知识库采用 CC BY-NC 4.0，GIF 的使用条件见随包保留的 `LICENSE-ASSETS`；非商业分享必须保留作者与网站署名，禁止收费和其他商业用途。作者：[可以叫我云大人/雾雪](https://space.bilibili.com/103739)，[kiriyuki.com](https://kiriyuki.com)。复用代码的 Apache-2.0 许可、NOTICE 和形象设计者署名一并保留。署名链接只用于来源说明，不作为图片外链。

角色、参数与资源授权在 Rust 端解析；起轮服务和资源事件可供其他 Rust 通道复用。Web 只负责角色卡和图片呈现，未来微信等应用不需要复制这套策略。本轮没有接入微信 bot。


## 自定义主题

把 CSS 文件放进 `<data_dir>/themes/<名字>.css`（名字只能用字母、数字、`_`、`-`），它会出现在右上角主题菜单的「自定义」分组里。变量清单与写法见 `web/THEMES.md`。

## 已有数据升级

本版首次启动会事务性地把 schema 1 升级到 schema 2，保留对话树、模型和账务记录。旧角色的公开名继承原名称，实际模型继承原默认模型（若存在）；未绑定可用模型的角色不会发布到聊天列表。升级前请停止服务并备份数据目录；旧版程序不能直接读取升级后的数据库。本次开发不会自动操作你的现有数据或重启服务。

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

# 设为默认模型与默认角色
curl -s -b $JAR -X PUT -H 'Content-Type: application/json' \
  -d "{\"default_model_id\":\"$MODEL_ID\",\"default_character_id\":\"$CHAR_ID\"}" \
  $BASE/api/admin/settings

# 聊天端只能取得角色公开名称和本地背景，不返回模型/提示词/参数
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
