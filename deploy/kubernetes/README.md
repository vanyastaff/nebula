# Kubernetes

Production-oriented Kubernetes manifests for Nebula.

## Layout

- `base/namespace.yaml`
- `base/configmap.yaml`
- `base/secret.example.yaml`
- `base/deployment-api.yaml`
- `base/service-api.yaml`
- `base/hpa-api.yaml`
- `base/pdb-api.yaml`
- `base/ingress.example.yaml` (optional)
- `base/kustomization.yaml`

## Important assumptions

- Primary DB is external managed PostgreSQL (RDS/Cloud SQL/etc).
- `DATABASE_URL` is provided via Secret.
- `nebula-api` image contains the `unified_server` binary.
- Worker loop is currently embedded in `nebula-api` process (`NEBULA_WORKER_COUNT`).

## Apply flow

1. Copy and edit secret template:

```bash
cp deploy/kubernetes/base/secret.example.yaml deploy/kubernetes/base/secret.yaml
```

2. Replace credentials and optional telemetry variables in `secret.yaml`.

3. Apply secret:

```bash
kubectl apply -f deploy/kubernetes/base/secret.yaml
```

4. Apply base resources:

```bash
kubectl apply -k deploy/kubernetes/base
```

5. (Optional) Apply ingress:

```bash
kubectl apply -f deploy/kubernetes/base/ingress.example.yaml
```

## Operational checks

```bash
kubectl -n nebula get deploy,po,svc,hpa,pdb
kubectl -n nebula rollout status deploy/nebula-api
kubectl -n nebula logs deploy/nebula-api --tail=200
```

Health endpoint used by probes: `GET /health`.

## Best-practice notes

- Use image tags pinned to immutable versions (no `latest` in production).
- Store secrets in a secret manager and sync to Kubernetes.
- Configure Pod security admission according to your cluster baseline.
- Run migrations as a dedicated Job before rollout.
