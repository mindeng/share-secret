# production overlay 切换到外部 Postgres

**日期**: 2026-06-16
**状态**: 设计已确认，待写实现计划

## 背景

k8s 部署用 kustomize：`base/` 跑 SQLite（`DATABASE_URL=sqlite:/data/share_secret.db`，挂在 ReadWriteOnce PVC 上，单副本 + `Recreate` 策略），`overlays/production/` 只加 namespace、镜像 pin（CI 自动更新 tag）。

应用二进制已同时支持两种后端（`src/db.rs`：`DATABASE_URL` 以 `postgres://`/`postgresql://` 开头时走 `init_postgres_schema`，自动建表），切换无需重新编译。`k8s/README.md` 已记录迁移路径（指向 postgres URL、从 Secret 注入、去掉 `/data` 卷与 PVC）。

## 目标

把 **production overlay** 改成连接一个**外部/托管 Postgres**，`base` 保持 SQLite 不变。

## 决策（已确认）

- **Postgres 位置**：外部/托管。overlay 不部署 Postgres，只接上它。
- **凭据注入**：一个 Secret 存完整 `DATABASE_URL`（单 key `DATABASE_URL`），deployment 用 `valueFrom.secretKeyRef` 读取。
- **Secret 来源**：运维用 `kubectl` 手动创建，**不提交进 git**；仓库只在 README 留命令（与现有 `ghcr-pull` / `APP_DOMAIN` 惯例一致）。不加示例文件。
- **副本/策略**：`replicas: 1` 不变，策略由 `Recreate` 改为 `RollingUpdate`。

## 非目标

- 不改 `base`（仍是 SQLite，可用于本地/dev）。
- 不改 Rust 代码（二进制已双后端）。
- 不部署 Postgres、不做备份/HA（交给托管方）。
- 不把任何真实凭据写入 git。

## 设计

### 1. 凭据 Secret（不入 git）

运维侧一次性创建：

```bash
kubectl create secret generic share-secret-db \
  --namespace share-secret \
  --from-literal=DATABASE_URL='postgres://USER:PASSWORD@HOST:5432/share_secret'
```

deployment 通过 `secretKeyRef`（name `share-secret-db`, key `DATABASE_URL`）引用。

### 2. Deployment patch（overlay 内，strategic merge）

对 base 的 `share-secret` Deployment 打补丁：

- **`DATABASE_URL`**：把 base 的内联 `value: "sqlite:/data/share_secret.db"` 换成 `valueFrom.secretKeyRef`。机制：strategic merge 按 `name` 合并 env 列表条目；用 `value: null` 删除 base 的内联值，并加上 `valueFrom`，避免同一条 env 同时带 `value` 和 `valueFrom`（非法）。实现时以 `kubectl kustomize` build 产物校验最终只有 `valueFrom`。
- **卷**：用 `$patch: delete` 删除名为 `data` 的 `volumeMount` 和 `volume`。
- **策略**：`spec.strategy.type` 设为 `RollingUpdate`（覆盖 base 的 `Recreate`）。
- **继承不动**：`replicas`、`BIND_ADDR`、`SECURE_COOKIES`、探针、资源、securityContext 等全部继承 base，patch 里不重述。

### 3. 删除 PVC

base 的 `kustomization.yaml` 把 `pvc.yaml` 列为 resource，overlay 默认会渲染出 PVC。用一个 `$patch: delete` 的补丁（targeting `PersistentVolumeClaim/share-secret-data`）把它从 production 产物里移除——外部 Postgres 后不再需要本地卷。

### 4. overlay kustomization 接线

在 `k8s/overlays/production/kustomization.yaml` 增加 `patches:`，引用上面两个补丁（deployment 改造、删 PVC）。`namespace`、`resources`、`images` 不变。

### 5. README 更新

`k8s/README.md` 的「Database backend」段改为：production overlay 已用外部 Postgres（说明 Secret 创建命令、引用方式、副本/策略变化）；base 仍是 SQLite。其余说明（镜像、APP_DOMAIN、gateway）不动。

## 改动文件

- `k8s/overlays/production/kustomization.yaml`（加 `patches:`）
- `k8s/overlays/production/` 下新增补丁文件（deployment strategic-merge 改造 + PVC `$patch: delete`）
- `k8s/README.md`（Database backend 段）

具体补丁是放一个文件还是两个文件、内联还是独立文件，由实现计划决定，以 `kubectl kustomize` build 通过为准。

## 验证

`kubectl kustomize k8s/overlays/production`（HTTPRoute 里有 `${APP_DOMAIN}` 占位符，build 本身不需要展开它即可成功；如需完整渲染再 `| envsubst`）应成功，且产物满足：

1. Deployment `strategy.type: RollingUpdate`，`replicas: 1`。
2. 容器 `DATABASE_URL` 来自 `secretKeyRef`（name `share-secret-db`, key `DATABASE_URL`），**没有**内联 `value`。
3. 没有 `/data` 的 `volumeMount`，没有名为 `data` 的 `volume`。
4. 没有 `PersistentVolumeClaim` 资源。
5. `BIND_ADDR`、`SECURE_COOKIES`、探针等仍在（继承 base）。

无自动化测试（纯 k8s 清单）；以 kustomize build 产物的人工核对为准。
