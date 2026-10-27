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

- **角色**：新增、修改系统提示词，并设置新对话的默认角色。已有对话保持原角色；修改该角色后，下一轮使用更新后的提示词。角色信息不会出现在聊天端。
- **模型**：按提供商刷新上游模型目录，选择上架哪些模型，设置显示名、排序、思考档位与默认档、图片能力、上下文窗口，以及每百万 token 的美元价格。API key 仍只在服务端环境变量中管理，网页不能查看或修改。当前网关只支持 Chat Completions，其他协议模型不能直接上架使用。
- **统计**：按时间、提供商和模型查看费用与 token 用量；图表颜色区分「提供商 + 模型」，可分别查看缓存输入、无缓存输入、思考和输出。输出数不含思考，图中的总计包含这四类。费用保留调用当时的记录，改价或下架不会改写历史。
- **未知费用**：未设置价格且上游未返回费用的请求标记为未定价，不当作免费。部分已知时显示已知费用和未定价请求数。
- **额度**：DeepSeek 查询官方余额，按原币种显示；OpenCode Go 根据本机记录的 `opencode.allowance` 估算最近 5 小时、7 天、30 天的用量。它不是账户的权威余额，不包含其他客户端的调用，滚动窗口也不代表服务商实际重置时刻。缺失额度计数或限额时显示未知，不伪造剩余数。

官方 `api.deepseek.com` / `opencode.ai` 地址可自动识别监控类型；自定义代理地址需要在提供商的 `monitoring` 中指定。OpenCode 默认按 Go 计划，可配置 `plan = "go_plus"`，或用 `monthly_limits` 按上游模型 ID 覆盖月额度；5 小时与每周额度分别按月额度的 20% / 50% 估算。示例见 `config.example.toml`。

额度与余额接口来源：[OpenCode Go](https://opencode.ai/v2/docs/console/go/)、[DeepSeek 余额](https://api-docs.deepseek.com/api/get-user-balance)。价格和计划可能变化，上架前请核对服务商当期信息。

## 自定义主题

把 CSS 文件放进 `<data_dir>/themes/<名字>.css`（名字只能用字母、数字、`_`、`-`），它会出现在右上角主题菜单的「自定义」分组里。变量清单与写法见 `web/THEMES.md`。

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

# 上架 deepseek-v4-flash（价格单位：美元/百万 token，未知可填 null）
MODEL_ID=$(curl -s -b $JAR -H 'Content-Type: application/json' -d '{
    "provider": "deepseek",
    "upstream_id": "deepseek-v4-flash",
    "display_name": "DeepSeek V4 Flash",
    "sort": 0,
    "protocol": "chat_completions",
    "reasoning": {"style": "deepseek", "levels": ["none", "low", "high", "max"], "default": "high"},
    "vision": false,
    "context_window": null,
    "price": {"cached_in": null, "uncached_in": null, "output": null}
  }' $BASE/api/admin/models | jq -r .id)

# 建角色
CHAR_ID=$(curl -s -b $JAR -H 'Content-Type: application/json' \
  -d '{"name":"猫娘","system_prompt":"你是一只友好的猫娘。"}' \
  $BASE/api/admin/characters | jq -r .id)

# 设为默认模型与默认角色
curl -s -b $JAR -X PUT -H 'Content-Type: application/json' \
  -d "{\"default_model_id\":\"$MODEL_ID\",\"default_character_id\":\"$CHAR_ID\"}" \
  $BASE/api/admin/settings

# 建对话（使用默认角色）
CONV_ID=$(curl -s -b $JAR -X POST $BASE/api/conversations | jq -r .id)

# 发一轮，-N 让 SSE 逐条输出；effort 为 null 时用模型的默认档位
curl -s -N -b $JAR -H 'Content-Type: application/json' \
  -d "{\"action\":\"send\",\"content\":\"你好\",\"model_id\":\"$MODEL_ID\",\"effort\":\"high\"}" \
  $BASE/api/conversations/$CONV_ID/turns
```

最后一步依次输出 `start`（新用户消息与回复节点 id）、若干 `reasoning`/`text` 增量，最后是 `done`（完整的回复节点，含 `metrics.usage`）；上游出错时以 `error` 结束。形如：

```text
event: start
data: {"assistant_node_id":"…","model_label":"DeepSeek V4 Flash · DeepSeek 官方","user_node":{"role":"user","content":"你好",…}}

event: reasoning
data: {"t":"…"}

event: text
data: {"t":"…"}

event: done
data: {"node":{"role":"assistant","status":"complete","content":"…","reasoning":"…","effort":"high","metrics":{"ttft_ms":…,"total_ms":…,"tokens_per_sec":…,"usage":{"cached_in":…,"uncached_in":…,"reasoning":…,"output":…}},…}}
```

`GET $BASE/api/conversations/$CONV_ID` 可查看整棵消息树。
