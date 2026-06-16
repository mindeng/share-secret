# PostgreSQL support — Design

Date: 2026-06-16
Status: Approved

## Goal

Add PostgreSQL as a runtime-selectable database backend alongside the existing
SQLite support, from a single binary. The backend is chosen by the
`DATABASE_URL` scheme: `sqlite:<path>` or `postgres://user:pass@host:port/db`.
SQLite stays — it backs the fast in-memory test suite and simple/local
deployments — and PostgreSQL becomes an alternative for production use.

## Approach: sqlx `Any` driver with `$1` placeholders

The app uses runtime queries (`sqlx::query` / `query_as`) and no `query!`
compile-time macros, so the `Any` driver fits the existing style. Every column
type in use (`i64`, `String`, `Option<String>`) is within Any's supported type
subset, and the backend is selected at runtime from the `DATABASE_URL` scheme.

### Spike finding (2026-06-16) — placeholder convention

A spike run against a real Postgres (Task 1) established two facts that shape
the approach:

- **Any does NOT translate `?` placeholders.** It passes them through to the
  native driver. Postgres rejects `?` with a syntax error; SQLite accepts `?`
  only because it is SQLite's native syntax. So a single query written with `?`
  cannot run on both backends.
- **Postgres-style `$1` placeholders work on BOTH backends via Any** — SQLite
  natively supports `$NNN` named parameters and sqlx binds them positionally.
  Any's type decoding on Postgres was also verified (`BIGINT`→`i64`,
  `TEXT`→`String`, `NULL`→`Option::None`).

**Decision:** write every query with `$1 … $n` placeholders. One query string
then runs on both backends through a single `AnyPool` — no runtime placeholder
translation, no per-backend SQL duplication, no backend flag. This supersedes
the original assumption that queries could keep `?`. The fallback options
considered (a runtime `?`→`$n` translator, or a full enum/repository abstraction
with native pools) proved unnecessary because the unified `$1` form works
directly.

## 1. Architecture & dependencies

- `AppState.db` changes from `sqlx::SqlitePool` to `sqlx::AnyPool`.
- `Cargo.toml` sqlx features become:
  `["runtime-tokio", "tls-rustls", "sqlite", "postgres", "any", "migrate"]`.
  `tls-rustls` lets Postgres-over-TLS (managed providers) work with no system
  OpenSSL, keeping the Debian-slim runtime image unchanged.
- `sqlx::any::install_default_drivers()` is called exactly once at process
  startup, guarded by a `std::sync::Once` (a second call errors, and the test
  suite builds many pools).

## 2. Schema / DDL & init (`src/db.rs`)

`init_db()` reads `DATABASE_URL` (default `sqlite:share_secret.db`), installs
drivers once, connects an `AnyPool`, then runs **backend-specific DDL** selected
by the URL scheme — the only place that branches:

- **SQLite**: unchanged from today — `INTEGER PRIMARY KEY AUTOINCREMENT`,
  `TEXT`, `kdf_salt TEXT`, `created_at DATETIME DEFAULT CURRENT_TIMESTAMP`, plus
  the existing legacy `ALTER TABLE shares ADD COLUMN kdf_salt TEXT` migration
  (ignored if the column already exists).
- **Postgres**:

  ```sql
  CREATE TABLE IF NOT EXISTS users (
      id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
      username TEXT UNIQUE NOT NULL,
      password_hash TEXT NOT NULL,
      created_at TEXT NOT NULL DEFAULT (now())::text
  );
  CREATE TABLE IF NOT EXISTS shares (
      id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
      user_id BIGINT NOT NULL REFERENCES users(id),
      slug TEXT UNIQUE NOT NULL,
      encrypted_payload TEXT NOT NULL,
      kdf_salt TEXT,
      created_at TEXT NOT NULL DEFAULT (now())::text
  );
  ```

  `id`/`user_id` are `BIGINT` so they decode to `i64` (matching the models).
  `kdf_salt` is present from the start, so no ALTER is needed on Postgres.
  `created_at` is `TEXT` (not `TIMESTAMP`) so it keeps decoding to
  `Share.created_at: String` under Any on both backends — no model change.

`init_db_memory()` (tests) stays SQLite in-memory but returns an `AnyPool`, and
also goes through the `Once`-guarded driver install.

Backend detection is by URL scheme: a `postgres://` or `postgresql://` prefix
means Postgres; anything else (i.e. `sqlite:`) means SQLite.

## 3. Query portability (`src/handlers/*`, `src/auth.rs`)

Every query's placeholders change from `?` to `$1 … $n` (numbered in bind
order). This single form runs on both backends through the `AnyPool`. The
affected query sites:

- `src/auth.rs` — `SELECT … FROM users WHERE id = $1`.
- `src/handlers/auth.rs` — `INSERT INTO users … VALUES ($1, $2)` and
  `SELECT … FROM users WHERE username = $1`.
- `src/handlers/share.rs` — the slug-existence check, the share `INSERT`
  (`$1..$4`), the `UPDATE` (`$1..$4`), the `DELETE` (`$1, $2`), and the
  payload `SELECT … WHERE slug = $1`.
- `src/handlers/dashboard.rs` — `SELECT … WHERE user_id = $1 ORDER BY …`.

Additionally, the slug-existence check in `create_share` changes from a typed
scalar to a row-presence check:

- Today: `sqlx::query_scalar::<_, i64>("SELECT 1 FROM shares WHERE slug = ?")`.
- Change to: `sqlx::query("SELECT 1 FROM shares WHERE slug = $1")
  .bind(&slug).fetch_optional(&state.db).await?.is_some()`.

Reason: Postgres types the literal `1` as `int4`, which would fail to decode as
`i64` under Any. Checking row presence (without decoding the scalar) is
backend-neutral.

No other handler logic changes. Structs deriving `sqlx::FromRow` (`User`,
`Share`) decode from `AnyRow` because their field types are in Any's supported
set (verified by the spike).

The naive textual nature of the placeholders is safe here: none of the queries
contain a literal `?` or `$` inside a string literal.

## 4. Spike-first + gated Postgres tests (`tests/`)

- **Task 1 — spike (committed as the first gated test):** connect an `AnyPool`
  to `TEST_DATABASE_URL` (a Postgres URL) and exercise a representative
  `$1`-placeholder `INSERT` + `SELECT` round-trip, decoding `i64` / `String` /
  `Option<String>`. This proved the placeholder/type behavior against real
  Postgres before the handlers were touched (see the Spike finding above).
- **Gated integration suite:** a test that reads `TEST_DATABASE_URL`; if unset it
  returns early (skips), so the default `cargo test` stays green with zero
  Postgres infra. When set, it drops + recreates the schema, then runs the core
  flows (register → login → create → fetch → update → delete) through the
  existing `oneshot` harness pointed at the Postgres pool, via a new
  `make_app_with_url(url)` test helper.
- The existing in-memory **SQLite suite is unchanged** and remains the primary
  ongoing coverage (the same SQL runs on both backends).

## 5. Documentation

Update the README / `k8s/README.md` to document:
- `DATABASE_URL` accepts `sqlite:<path>` or
  `postgres://user:pass@host:port/db` (the database must already exist for
  Postgres).
- `created_at` is stored as text.
- `TEST_DATABASE_URL` enables the gated Postgres integration tests.

K8s manifests stay on SQLite + PVC, as agreed.

## Error handling

Any errors remain `sqlx::Error`, so the existing `From<sqlx::Error>` in
`error.rs` covers them — no error-type changes.

## Out of scope

- In-cluster Postgres manifests (StatefulSet/Service/Secret).
- Data migration from an existing SQLite database to Postgres.
- Connection-pool tuning beyond the current `max_connections(5)`.
