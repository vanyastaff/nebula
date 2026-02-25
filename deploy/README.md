# Deploy — Docker и Kubernetes

Здесь лежат файлы для поднятия окружения: БД, приложение, локальная разработка и деплой в кластер.

## Структура

- **`docker/`** — Dockerfile(s) и docker-compose для локального запуска (Postgres, Redis, приложение).
- **`kubernetes/`** — манифесты K8s (ConfigMap, Secret, Deployment, Service, при необходимости БД в кластере или внешняя БД).

## Локально: поднять только БД

```bash
cd deploy
cp .env.example .env   # при необходимости отредактировать
docker compose -f docker/docker-compose.yml up -d postgres
```

После этого `DATABASE_URL` из `.env` можно использовать для миграций и приложения:

```bash
export $(grep -v '^#' deploy/.env | xargs)
sqlx migrate run
cargo run -p nebula-api --example unified_server
```

## Локально: БД + все сервисы (compose)

```bash
cd deploy
docker compose -f docker/docker-compose.yml up -d
```

## Kubernetes

Манифесты в `kubernetes/` — для деплоя в кластер. БД может быть внешней (RDS, Cloud SQL) или подниматься в кластере (StatefulSet/оператор). См. комментарии в файлах в `kubernetes/`.
