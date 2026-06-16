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
cluster (`kubectl get gatewayclass`) and set the real hostname in both
`base/gateway.yaml` and `base/httproute.yaml`.

## Deploy

```bash
# Preview the rendered manifests
kubectl kustomize k8s/overlays/production

# Apply
kubectl apply -k k8s/overlays/production
```

Argo CD / Flux users: point the application at `k8s/overlays/production` and it
syncs automatically whenever CI bumps the tag.

## Notes

- **Single replica + Recreate**: SQLite is a single-writer file on a
  ReadWriteOnce PVC. Do not scale `replicas` above 1 without switching to a
  networked database.
- **Local image build**: `docker build -t share-secret .`
