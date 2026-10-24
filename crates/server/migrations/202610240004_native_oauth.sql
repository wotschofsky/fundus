ALTER TABLE oauth_clients ADD COLUMN allowed_scope TEXT NOT NULL DEFAULT 'mcp:read'
 CHECK(allowed_scope IN ('mcp:read','vault:owner'));
