# Share Export / Import — Design

**Date:** 2026-06-16
**Status:** Approved (pre-implementation)

## Summary

Let a logged-in user export all of their own shares to a JSON file with one
click, and import such a file back, re-creating the shares under their account.
Slugs are preserved so original `/s/<slug>` links keep working.

This is a transport feature only: it moves the **ciphertext exactly as stored**
(`slug`, `encrypted_payload`, `kdf_salt`, `created_at`). It never decrypts.
The zero-knowledge model is unchanged — the server still never sees plaintext or
keys, and an exported file remains undecryptable without the original link-key
or password, exactly as today.

## Scope

- **Actor:** the logged-in user, via the dashboard.
- **Coverage:** all shares owned by that user (`WHERE user_id = current user`).
- **Import target:** the importing user's own account (`user_id` taken from the
  session, never from the file).

### Non-goals (YAGNI)

- No operator/whole-database dump, no users table in the export.
- No cross-account key recovery — ciphertext stays undecryptable without the
  original link-key/password.
- No merge/update of existing shares — slug collisions are **skipped**, never
  overwritten.

## Data format

Versioned JSON envelope:

```json
{
  "version": 1,
  "exported_at": "2026-06-16 12:00:00",
  "shares": [
    {
      "slug": "Ab3xY7zQ9pLm",
      "encrypted_payload": "<base64 iv+ciphertext>",
      "kdf_salt": "<base64 | null>",
      "created_at": "2026-06-10 09:30:00"
    }
  ]
}
```

- Exactly the four stored columns per share. No `id`, no `user_id` — those are
  instance/account-local.
- `version` lets the format evolve; import rejects any version other than `1`.
- `created_at` uses the same text form the app already stores
  (`YYYY-MM-DD HH:MM:SS`).

## Semantics

### Export — `GET /api/shares/export` (auth required)

- Select `slug, encrypted_payload, kdf_salt, created_at` for the current user,
  ordered by `created_at`.
- Return the envelope with `Content-Type: application/json` and
  `Content-Disposition: attachment; filename="share-secret-export.json"` so the
  browser saves a file.
- Empty account → a valid envelope with `"shares": []` (HTTP 200, not an error).

### Import — `POST /api/shares/import` (auth required, JSON body = envelope)

- Reject `version != 1` → `400`.
- Per share, validate `encrypted_payload` non-empty and `slug` present/non-empty.
  Malformed entries are counted as `errors`, never fatal.
- **Slug collision → skip.** `SELECT 1 FROM shares WHERE slug = $1`; if present,
  skip (leave the existing row untouched). This makes re-importing one's own
  export idempotent.
- Insert surviving rows under the **current** `user_id`, **preserving the
  original `slug` and `created_at`** (original links keep working, dashboard
  ordering stays faithful). The INSERT sets `created_at` explicitly rather than
  relying on the DB default.
- Return `{ "imported": N, "skipped": M, "errors": K }`; the dashboard shows it
  as a message.

## Components & files

### Backend — `src/handlers/share.rs`

- Structs: `ExportEnvelope { version, exported_at, shares: Vec<ExportShare> }`,
  `ExportShare { slug, encrypted_payload, kdf_salt, created_at }`,
  `ImportSummary { imported, skipped, errors }`.
- `export_shares(State, CurrentUser) -> impl IntoResponse` — sets the attachment
  headers (e.g. `(HeaderMap, String)`); reuses the dashboard's `query_as` shape.
- `import_shares(State, CurrentUser, Json<ExportEnvelope>) -> Json<ImportSummary>`
  — per-row collision check + insert.
- New insert statement includes `created_at` explicitly. Uses `$1..$n`
  placeholders (AnyPool convention: works on both SQLite and Postgres).

### Routes — `src/lib.rs`

- `GET  /api/shares/export`
- `POST /api/shares/import`

### Frontend

- `templates/dashboard.html`: an **Export** link (navigates to
  `/api/shares/export`) and an **Import** button that triggers a hidden
  `<input type="file">`.
- `static/crypto.js` (or small inline script): on file selection,
  `FileReader` → `JSON.parse` → `fetch('/api/shares/import', { JSON })` → show
  the returned summary. No decryption — pure ciphertext transport.

## Error handling

Reuse `AppError`: `BadRequest` for bad version / malformed body; auth via the
existing `CurrentUser` extractor. Per-row validation failures are tallied into
`errors` and never abort the batch.

## Testing — `tests/integration_test.rs` (in-memory SQLite)

1. Export returns the user's shares and excludes other users' shares.
2. Round-trip: create → export → wipe → import → same slugs/payloads/`created_at`.
3. Idempotent re-import: importing the same file twice → second run reports all
   skipped, no duplicates.
4. Import only affects the importing user (imported shares owned by current user).
5. Bad `version` → 400; a malformed entry is counted in `errors` without
   aborting the valid ones.
