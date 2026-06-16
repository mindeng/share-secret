# Deployment

Container image is built by GitHub Actions (`.github/workflows/deploy.yml`) and
pushed to **GitHub Container Registry** (`ghcr.io/<owner>/share-secret`). On every
push to `main` the workflow also runs `kustomize edit set image` and commits the
new `sha-<short>` tag into `overlays/production/kustomization.yaml`, so the desired
image version always lives in git.

```
k8s/
  base/                 deployment, service, pvc, gateway, httproute
  overlays/production/  namespace + image pin (CI updates the tag here)
```

## One-time setup

### 1. Image name

The CI uses `ghcr.io/<owner>/<repo>` automatically. The placeholder
`ghcr.io/OWNER/share-secret` in `overlays/production/kustomization.yaml` is
rewritten by the first successful CI run.

### 2. Pull access

The ghcr package is **public**, so no pull secret is needed — the cluster pulls
anonymously and `base/deployment.yaml` has no `imagePullSecrets`.

If you ever switch the package back to private (GitHub → Packages →
`share-secret` → Package settings → Change visibility), recreate the pull secret
and re-add `imagePullSecrets: [{name: ghcr-pull}]` to the deployment:

```bash
kubectl create secret docker-registry ghcr-pull \
  --namespace share-secret \
  --docker-server=ghcr.io \
  --docker-username=<github-username> \
  --docker-password=<github-PAT-with-read:packages>
```

### 3. Gateway

Set `gatewayClassName` in `base/gateway.yaml` to a class that exists in your
cluster (`kubectl get gatewayclass`). The hostname is **not** hard-coded — the
HTTPRoute uses a `${APP_DOMAIN}` placeholder filled in at apply time (below).

## Deploy

The HTTPRoute hostname comes from the `APP_DOMAIN` env var via `envsubst`, so the
domain lives in your environment / CI, not in git:

```bash
export APP_DOMAIN=secret.yourdomain.com

# Preview
kubectl kustomize k8s/overlays/production | envsubst

# Apply
kubectl kustomize k8s/overlays/production | envsubst | kubectl apply -f -
```

> `envsubst` ships with GNU gettext (`apt install gettext-base` if missing).
> Only `${APP_DOMAIN}` is substituted — nothing else in the manifests uses `$`.

Argo CD / Flux users: point the application at `k8s/overlays/production`. Argo
CD needs the **envsubst / Kustomize plugin** (or a `replacement`) to fill
`${APP_DOMAIN}`; set `APP_DOMAIN` as a build environment variable on the app.
The image tag still syncs automatically whenever CI bumps it.

## Database backend

The **base** manifests run on SQLite backed by the PersistentVolumeClaim mounted
at `/data` (`DATABASE_URL=sqlite:/data/share_secret.db`). This is suitable for
local / single-node use.

The **production overlay** (`overlays/production`) runs on an **external
PostgreSQL** instead. It patches the base Deployment to read `DATABASE_URL` from
a Secret, removes the `/data` volume + PVC, and switches the rollout strategy
from `Recreate` to `RollingUpdate` (no PVC single-writer contention once the DB
is networked). The application binary supports both backends with no rebuild.

Create the Secret once, out of band (it is **not** committed to git, matching the
`ghcr-pull` / `APP_DOMAIN` convention):

```bash
kubectl create secret generic share-secret-db \
  --namespace share-secret \
  --from-literal=DATABASE_URL='postgres://USER:PASSWORD@HOST:5432/share_secret'
```

The schema is created automatically on first connect (see `src/db.rs`
`init_postgres_schema`). `replicas` stays at 1; with a networked DB you may now
scale it up safely.

## Notes

- **Single replica**: the base (SQLite) uses `Recreate` because SQLite is a
  single-writer file on a ReadWriteOnce PVC. The production overlay switches to
  PostgreSQL + `RollingUpdate`; you may scale `replicas` above 1 there safely.
- **Local image build**: `docker build -t share-secret .`
