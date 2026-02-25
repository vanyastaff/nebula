# Kubernetes

Манифесты для деплоя Nebula в кластер.

- БД в продакшене обычно внешняя (RDS, Cloud SQL, managed Postgres). Подключение через `Secret` и `DATABASE_URL` в конфиге приложения.
- Здесь можно добавить: `Namespace`, `Deployment`/`StatefulSet` приложения, `Service`, `ConfigMap`, `Secret` (шаблоны без реальных паролей), при необходимости `Job` для миграций.

Порядок применения (пример):

```bash
kubectl apply -f namespace.yaml
kubectl apply -f configmap.yaml
kubectl apply -f secret.yaml   # заполнить данные
kubectl apply -f deployment.yaml
kubectl apply -f service.yaml
```

Миграции: отдельный `Job` с образом приложения и командой `sqlx migrate run`, либо выполнять вручную из пода с доступом к БД.
