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
CREATE INDEX IF NOT EXISTS usage_by_model_time ON usage(provider, upstream_id, at_ms, id);
