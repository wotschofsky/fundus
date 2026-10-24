CREATE TABLE oauth_clients (
 id TEXT PRIMARY KEY, name TEXT NOT NULL, redirect_uris TEXT NOT NULL,
 auth_method TEXT NOT NULL, secret_hash TEXT, created_at INTEGER NOT NULL
);
CREATE TABLE oauth_pending (
 hash TEXT PRIMARY KEY, client_id TEXT NOT NULL REFERENCES oauth_clients(id),
 redirect_uri TEXT NOT NULL, challenge TEXT NOT NULL, scope TEXT NOT NULL,
 resource TEXT NOT NULL, state TEXT, expires_at INTEGER NOT NULL
);
CREATE TABLE oauth_codes (
 hash TEXT PRIMARY KEY, client_id TEXT NOT NULL REFERENCES oauth_clients(id),
 redirect_uri TEXT NOT NULL, challenge TEXT NOT NULL, scope TEXT NOT NULL,
 resource TEXT NOT NULL, expires_at INTEGER NOT NULL
);
CREATE TABLE oauth_grants (
 id TEXT PRIMARY KEY, client_id TEXT NOT NULL REFERENCES oauth_clients(id),
 scope TEXT NOT NULL, resource TEXT NOT NULL, created_at INTEGER NOT NULL,
 expires_at INTEGER NOT NULL, revoked_at INTEGER
);
CREATE TABLE oauth_access (
 hash TEXT PRIMARY KEY, grant_id TEXT NOT NULL REFERENCES oauth_grants(id) ON DELETE CASCADE,
 expires_at INTEGER NOT NULL
);
CREATE TABLE oauth_refresh (
 hash TEXT PRIMARY KEY, grant_id TEXT NOT NULL REFERENCES oauth_grants(id) ON DELETE CASCADE,
 expires_at INTEGER NOT NULL, used_at INTEGER
);
CREATE INDEX oauth_grants_client ON oauth_grants(client_id);
CREATE INDEX oauth_clients_created ON oauth_clients(created_at);
