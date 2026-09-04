-- §20.7. The first phase-2 migration: §21's `sync_budget` references `account(id)`, so accounts
-- come before every other phase-2 table.
--
-- The database stores `token_ref` and never a token; the OS keychain stores the secret (§20.6).
-- Every `*_at` is nullable-means-unknown per §1.12 — a NULL is "never observed", never a zero.
CREATE TABLE account (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  -- No CHECK, deliberately: a STRICT table's constraint cannot be altered in place, so a CHECK
  -- here would make adding a second forge a create-copy-drop-rename of live credential state.
  provider       TEXT NOT NULL,
  host           TEXT NOT NULL,
  login          TEXT NOT NULL,
  display_name   TEXT,
  auth_kind      TEXT NOT NULL CHECK (auth_kind IN ('device', 'pat')),
  scope_tier     TEXT NOT NULL CHECK (scope_tier IN ('public', 'private')),
  -- Verbatim from the server. NEVER a source literal: a hard-coded grant is how two scope names
  -- that do not exist came to be drawn on a settings screen.
  granted_scopes TEXT NOT NULL,
  scopes_observed_at INTEGER,
  -- The keychain entry name, `<provider>:<host>:<login>`. NEVER a token.
  token_ref      TEXT NOT NULL,
  connected_at   INTEGER NOT NULL,
  last_verified_at INTEGER,
  last_error_kind  TEXT,
  last_error_at    INTEGER,
  is_enabled     INTEGER NOT NULL DEFAULT 1 CHECK (is_enabled IN (0, 1)),
  -- §21.7's listing validator. It gets no clock: an ETag is never rendered as a time, and one
  -- read of one resource writes the validator and the listing together. Declared at creation
  -- because adding it later would be a rebuild of live credential state.
  listing_etag   TEXT,
  UNIQUE (provider, host, login)
) STRICT;

CREATE TABLE account_org (
  account_id      INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  login           TEXT NOT NULL,
  -- OFF by default: one membership in a large organisation would otherwise admit thousands of
  -- tiles the user never asked for.
  is_enabled      INTEGER NOT NULL DEFAULT 0 CHECK (is_enabled IN (0, 1)),
  -- NULL = not counted, rendered as unknown. Never as 0.
  repo_count_seen INTEGER,
  sso_state       TEXT CHECK (sso_state IS NULL OR sso_state IN
                    ('none', 'authorized', 'unauthorized', 'unknown')),
  observed_at     INTEGER,
  PRIMARY KEY (account_id, login)
) STRICT;

CREATE TABLE project_account (
  project_id  INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  account_id  INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  affiliation TEXT NOT NULL CHECK (affiliation IN ('owner', 'organization_member', 'collaborator')),
  can_push    INTEGER NOT NULL CHECK (can_push IN (0, 1)),
  observed_at INTEGER NOT NULL,
  -- Absence from a listing marks unseen; it never deletes a row.
  last_seen_generation INTEGER,
  PRIMARY KEY (project_id, account_id)
) STRICT;
