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

3. 启动：

   ```sh
   cargo run -p nekochat-server -- serve --config config.toml
   ```

默认监听 `127.0.0.1:8787`。监听非环回地址时请设置 `access_password`，并放在 HTTPS 反向代理之后（配置 `trusted_proxies`）。

## curl 冒烟

以下流程走一遍管理端与对话接口，需要 `curl` 与 `jq`，并在 `config.toml` 里设置了 `admin_password`（未设置时管理接口一律 404）。示例假定默认地址与上面配置里的 `deepseek` 提供商；请把 `你的管理密码` 换成实际值。

```sh
BASE=http://127.0.0.1:8787
JAR=cookies.txt

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
