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
