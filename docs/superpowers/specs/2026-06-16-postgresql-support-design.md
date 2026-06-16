# PostgreSQL support — Design

Date: 2026-06-16
Status: Approved

## Goal

Add PostgreSQL as a runtime-selectable database backend alongside the existing
SQLite support, from a single binary. The backend is chosen by the
`DATABASE_URL` scheme: `sqlite:<path>` or `postgres://user:pass@host:port/db`.
SQLite stays — it backs the fast in-memory test suite and simple/local
deployments — and PostgreSQL becomes an alternative for production use.

## Approach: sqlx `Any` driver

The app already uses runtime queries (`sqlx::query` / `query_as` with `?`
placeholders) and no `query!` compile-time macros, so the `Any` driver fits the
existing style with essentially no query rewrites. Every column type in use
(`i64`, `String`, `Option<String>`) is within Any's supported type subset.

The chief risk — whether Any reliably rewrites `?`→`$1` for Postgres — is
de-risked by making the first implementation step a spike that proves the
behavior against a real Postgres before any handler is touched. If the spike
fails, we fall back to an enum/repository abstraction (per-backend native SQL),
having rewritten nothing.

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

All 8 queries keep their `?` placeholders unchanged — Any rewrites them to `$1`
for Postgres. The only change is the slug-existence check in
`create_share`:

- Today: `sqlx::query_scalar::<_, i64>("SELECT 1 FROM shares WHERE slug = ?")`.
- Change to row-presence: `sqlx::query("SELECT 1 FROM shares WHERE slug = ?")
  .bind(&slug).fetch_optional(&state.db).await?.is_some()`.

Reason: Postgres types the literal `1` as `int4`, which would fail to decode as
`i64` under Any. Checking row presence (without decoding the scalar) is
backend-neutral.

No other handler logic changes. Structs deriving `sqlx::FromRow` (`User`,
`Share`) decode from `AnyRow` because their field types are in Any's supported
set.

## 4. Spike-first + gated Postgres tests (`tests/`)

- **Task 1 — spike (committed as the first gated test):** connect an `AnyPool`
  to `TEST_DATABASE_URL` (a Postgres URL), run the Postgres DDL, then exercise a
  representative `?`-placeholder `INSERT` + `SELECT` round-trip. This proves
  Any's placeholder rewriting and type decoding against real Postgres before the
  handlers are touched. If it fails: stop and escalate (fall back to the
  enum/repository approach).
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
