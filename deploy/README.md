# Deploy — Docker и Kubernetes

Здесь лежат файлы для поднятия окружения: БД, приложение, локальная разработка и деплой в кластер.

## Структура

- **`docker/`** — Dockerfile(s) и docker-compose для локального запуска (Postgres, Redis, приложение).
- **`kubernetes/`** — манифесты K8s (ConfigMap, Secret, Deployment, Service, при необходимости БД в кластере или внешняя БД).
- **`STACKS.md`** — режимы деплоя в стиле n8n: Local / Self-Hosted / SaaS.

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
cargo run -p nebula-api --bin unified_server
```

## Локально: БД + все сервисы (compose)

```bash
cd deploy
docker compose -f docker/docker-compose.yml up -d
```

С Redis (профиль `cache`):

```bash
docker compose -f docker/docker-compose.yml --profile cache up -d
```

## Self-Hosted baseline (compose)

Полноценный single-node стек (Postgres + Redis + API):

```bash
cd deploy
docker compose -f docker/docker-compose.selfhosted.yml up -d --build
```

Примечание: по умолчанию compose собирает локальный образ API через
`deploy/docker/Dockerfile.api`. Для CI/CD можно переопределить
`NEBULA_API_IMAGE`.

## SaaS blueprint

Для архитектурного ориентира добавлен файл:

```bash
deploy/docker/docker-compose.saas.blueprint.yml
```

Это reference-топология (реплики API c embedded workers). Для реального SaaS нужен
оркестратор (Kubernetes/ECS/Nomad) и managed сервисы (Postgres/Redis, secrets,
ingress, observability).

## Локально: observability для `nebula-log` telemetry (OTLP + Jaeger)

Поднимает OpenTelemetry Collector и Jaeger UI для проверки трейсов:

```bash
docker compose -f deploy/docker/docker-compose.observability.yml up -d
```

После запуска:

- OTLP gRPC endpoint для приложения: `http://localhost:4317`
- OTLP HTTP endpoint для приложения: `http://localhost:4318`
- Jaeger UI: `http://localhost:16686`

Остановить и удалить контейнеры:

```bash
docker compose -f deploy/docker/docker-compose.observability.yml down
```

## Kubernetes

Манифесты в `kubernetes/` — для деплоя в кластер. БД может быть внешней (RDS, Cloud SQL) или подниматься в кластере (StatefulSet/оператор). См. комментарии в файлах в `kubernetes/`.
