# Owner editing for shares — Design

Date: 2026-06-16
Status: Approved

## Goal

Allow the user who created a share to edit it: change the title, the field
name/value pairs (add/remove fields), and optionally change the view password
(switching between link mode and password mode). The slug never changes.

## Constraint: zero-knowledge encryption

The server only ever stores ciphertext (`encrypted_payload`) plus an optional
`kdf_salt`. It never sees plaintext or the encryption key. Therefore an edit
requires the share to be **decrypted first**, which is only possible where the
key/password is available:

- **Link mode**: random key lives only in the URL fragment (`#key=...`), shown
  once at creation. Neither the server nor the dashboard has it.
- **Password mode**: key is derived from the view password; the server stores
  only the salt.

This is why editing lives on the view page, not the dashboard.

## Core approach

Editing happens **inline on the view page** (`/s/:slug`), the only context where
plaintext exists (the key is already in memory after a successful decrypt). No
new page, no fragment-passing problem, no page reload.

Flow:

1. View page decrypts as it does today (fragment key or password prompt).
2. The payload API additionally returns an `is_owner` flag derived from the
   session. If the viewer is the logged-in owner **and** decryption succeeded,
   an "编辑" (Edit) button appears.
3. Clicking 编辑 swaps the read-only table for an editable form (title, field
   name/value rows, add/remove field, optional view-password) pre-filled from
   the already-decrypted payload.
4. On save, the client re-encrypts in the browser and POSTs the new ciphertext
   to an owner-only update endpoint. The slug never changes.

## Re-encryption rules on save

- **Password entered** → password mode: generate a new salt, derive the key,
  encrypt. Store the new `kdf_salt`. The link no longer carries a key.
- **Password left empty:**
  - Share was opened via fragment key (already link mode) → **reuse the same
    key**, so existing `#key=...` links stay valid and show updated content.
    `kdf_salt` = NULL.
  - Share was opened via password (was password mode) and password now cleared
    → switch to link mode: generate a fresh key, `kdf_salt` = NULL, and show the
    new link.

The client tracks the current mode and the in-memory key/cryptoKey established
during the initial decrypt so it can apply these rules without re-prompting.

## Server changes

`src/handlers/share.rs`:

- `get_share_payload`: add an optional session lookup and return
  `is_owner: bool` alongside `encrypted_payload` / `kdf_salt`. Anonymous viewers
  get `false`. The endpoint stays public (no auth required to view).
- New `update_share` handler at `POST /api/shares/:slug/update` (consistent with
  the existing `/api/shares/:id/delete` style). Requires `CurrentUser`. Body:
  `{ encrypted_payload: String, kdf_salt: Option<String> }`. Runs
  `UPDATE shares SET encrypted_payload = ?, kdf_salt = ? WHERE slug = ? AND user_id = ?`.
  Reject empty `encrypted_payload` with `BadRequest` (mirrors `create_share`).
  Zero rows affected → `AppError::Forbidden` (mirrors `delete_share`).

`src/lib.rs`:

- Register the new route.

## Client changes

`static/crypto.js`:

- Add `updateShare(slug, payload, password, existing)` where `existing` carries
  the current mode and in-memory key, applying the re-encryption rules above and
  POSTing to `/api/shares/:slug/update`. Returns the new `{ key, passwordProtected }`
  so the UI can show an updated link/hint.

`templates/view_share.html`:

- Render the 编辑 button when `is_owner` is true and decryption succeeded.
- Add the editable form markup (reusing the field-row pattern from
  `new_share.html`) and the toggle/save logic. After a successful save, refresh
  the displayed content and, for link mode, show the (possibly unchanged) link;
  for password mode, show the password hint.

## Dashboard change

`templates/dashboard.html`:

- Make each `/s/{{ share.slug }}` an `<a href="/s/{{ share.slug }}">` link to the
  view page. Caveat: link-mode shares opened from the dashboard won't carry the
  `#key=` fragment, so they open but can't decrypt without the full original
  link; this works fully for password-mode shares.

## Testing

`tests/integration_test.rs`:

- Owner can update their own share → 200, stored ciphertext changes.
- Non-owner update is rejected → 403.
- Unauthenticated update is rejected → 401.
- `is_owner` is `true` for the owner and `false` for an anonymous viewer.
- Updating a nonexistent slug → forbidden/not-found.

## Out of scope

- Changing the slug.
- Server-side editing of plaintext (impossible by design).
- Edit history / versioning.
