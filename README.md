# share-secret

## Database

`share-secret` supports two backends, selected at runtime by the `DATABASE_URL`
environment variable:

- **SQLite** (default): `DATABASE_URL=sqlite:share_secret.db`. The file is
  created automatically. Used by the test suite and simple/local deployments.
- **PostgreSQL**: `DATABASE_URL=postgres://user:password@host:5432/dbname`. The
  database must already exist; the app creates its tables on startup. TLS is
  supported (e.g. managed providers) with no extra system libraries.

Notes:
- `created_at` is stored as text on both backends.
- Tables are created automatically at startup; there is no separate migration
  step.

### Running the PostgreSQL tests

The default `cargo test` runs entirely on in-memory SQLite. The Postgres
integration tests in `tests/postgres_test.rs` are skipped unless you point them
at a real database:

```sh
TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres \
  cargo test --test postgres_test -- --nocapture
```
