# production overlay → 外部 Postgres Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `k8s/overlays/production` 连接外部托管 Postgres（凭据从 Secret 注入），base 保持 SQLite 不变。

**Architecture:** 纯 kustomize 覆盖：production overlay 增加两个补丁——一个 strategic-merge 补丁改造 base Deployment（`DATABASE_URL` 改用 `secretKeyRef`、删 `/data` 卷与挂载、策略改 `RollingUpdate`），一个 `$patch: delete` 补丁移除 PVC。不改 Rust 代码（二进制已双后端）。补丁内容已用真实 base 跑 `kustomize build` 验证通过。

**Tech Stack:** Kubernetes、kustomize v5、外部 PostgreSQL。

---

## File Structure

- `k8s/overlays/production/patch-postgres.yaml`（新增）：strategic-merge 补丁，改造 `share-secret` Deployment。
- `k8s/overlays/production/patch-drop-pvc.yaml`（新增）：删除 base 的 `share-secret-data` PVC。
- `k8s/overlays/production/kustomization.yaml`（修改）：增加 `patches:` 引用上面两个文件。
- `k8s/README.md`（修改）：「Database backend」段更新为 production 已用外部 Postgres。

base 完全不动。

---

## Task 1: production overlay 接外部 Postgres（补丁 + 接线 + 校验）

**Files:**
- Create: `k8s/overlays/production/patch-postgres.yaml`
- Create: `k8s/overlays/production/patch-drop-pvc.yaml`
- Modify: `k8s/overlays/production/kustomization.yaml`

- [ ] **Step 1: 创建 Deployment 改造补丁** `k8s/overlays/production/patch-postgres.yaml`

```yaml
# production：连接外部 Postgres。
# - DATABASE_URL 改为从 Secret share-secret-db 注入（value: null 删除 base 的内联 sqlite 值）
# - 删除 /data 卷与挂载（外部 DB 不再需要本地盘）
# - 策略由 base 的 Recreate 改为 RollingUpdate（不再有 PVC 单写争用）
apiVersion: apps/v1
kind: Deployment
metadata:
  name: share-secret
spec:
  strategy:
    type: RollingUpdate
  template:
    spec:
      containers:
        - name: share-secret
          env:
            - name: DATABASE_URL
              value: null
              valueFrom:
                secretKeyRef:
                  name: share-secret-db
                  key: DATABASE_URL
          volumeMounts:
            - name: data
              $patch: delete
      volumes:
        - name: data
          $patch: delete
```

- [ ] **Step 2: 创建 PVC 删除补丁** `k8s/overlays/production/patch-drop-pvc.yaml`

```yaml
# 外部 Postgres 后不再需要本地 PVC；把 base 引入的 PVC 从 production 产物中移除。
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: share-secret-data
$patch: delete
```

- [ ] **Step 3: 接线 kustomization** — 在 `k8s/overlays/production/kustomization.yaml` 末尾追加 `patches:` 段。当前文件结尾是 `images:` 块（最后一行 `  newTag: sha-2b2b48a`，CI 会改这个 tag，不要动它）。在文件末尾追加：

```yaml

patches:
- path: patch-postgres.yaml
- path: patch-drop-pvc.yaml
```

- [ ] **Step 4: 校验 build** — Run: `kustomize build k8s/overlays/production`

Expected: 退出码 0，且产物中（可肉眼核对，或用下一步的断言）：
- Deployment `spec.strategy.type: RollingUpdate`，`spec.replicas: 1`
- 容器 `DATABASE_URL` 只有 `valueFrom.secretKeyRef`（name `share-secret-db`, key `DATABASE_URL`），**没有**内联 `value:`
- `volumeMounts: []`、`volumes: []`（data 已删）
- 没有 `kind: PersistentVolumeClaim`
- `BIND_ADDR`、`SECURE_COOKIES`、liveness/readiness 探针仍在
- `${APP_DOMAIN}` 占位符原样保留在 Gateway/HTTPRoute（apply 时才 envsubst）

- [ ] **Step 5: 断言式校验**（确认关键不变量，不靠肉眼）— Run:

```bash
out=$(kustomize build k8s/overlays/production)
echo "$out" | grep -q "type: RollingUpdate" && echo "OK strategy" || echo "FAIL strategy"
echo "$out" | grep -q "name: share-secret-db" && echo "OK secretRef" || echo "FAIL secretRef"
echo "$out" | grep -q "kind: PersistentVolumeClaim" && echo "FAIL pvc-present" || echo "OK no-pvc"
echo "$out" | grep -q "sqlite:/data" && echo "FAIL sqlite-leftover" || echo "OK no-sqlite"
echo "$out" | grep -q 'hostname: ${APP_DOMAIN}' && echo "OK appdomain" || echo "FAIL appdomain"
```

Expected: 全部打印 `OK ...`，没有任何 `FAIL`。

- [ ] **Step 6: 提交**

```bash
git add k8s/overlays/production/patch-postgres.yaml \
        k8s/overlays/production/patch-drop-pvc.yaml \
        k8s/overlays/production/kustomization.yaml
git commit -m "feat(k8s): production overlay uses external Postgres via Secret"
```

---

## Task 2: 更新 README 的数据库后端说明

**Files:**
- Modify: `k8s/README.md`（「## Database backend」段）

- [ ] **Step 1: 替换「Database backend」段** — 找到现有段落：

```markdown
## Database backend

The base manifests run on SQLite backed by the PersistentVolumeClaim mounted at
`/data` (`DATABASE_URL=sqlite:/data/share_secret.db`). To use PostgreSQL instead,
point `DATABASE_URL` in `k8s/base/deployment.yaml` at a `postgres://…` URL
(inject the credentials from a Secret) and drop the `/data` volume + PVC. The
application binary supports both backends with no rebuild.
```

替换为：

```markdown
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
```

- [ ] **Step 2: 更新「Notes」里关于单副本的说明** — 找到：

```markdown
- **Single replica + Recreate**: SQLite is a single-writer file on a
  ReadWriteOnce PVC. Do not scale `replicas` above 1 without switching to a
  networked database.
```

替换为：

```markdown
- **Single replica**: the base (SQLite) uses `Recreate` because SQLite is a
  single-writer file on a ReadWriteOnce PVC. The production overlay switches to
  PostgreSQL + `RollingUpdate`; you may scale `replicas` above 1 there safely.
```

- [ ] **Step 3: 校验 README 改动一致**（确认没有残留旧表述）— Run:

```bash
grep -n "external\|RollingUpdate\|share-secret-db" k8s/README.md
```

Expected: 能看到上面新增的关于 external Postgres / RollingUpdate / Secret 名称的行。

- [ ] **Step 4: 提交**

```bash
git add k8s/README.md
git commit -m "docs(k8s): document production external Postgres backend"
```

---

## 最终验证

- [ ] Run: `kustomize build k8s/overlays/production >/dev/null && echo BUILD_OK`
  Expected: `BUILD_OK`。
- [ ] 重跑 Task 1 Step 5 的断言块，确认全 `OK`。
