# Archived From "docs/archive/final.md"

### nebula-storage
**Назначение:** Абстракция над различными системами хранения данных.

**Поддерживаемые backends:**
- PostgreSQL/MySQL - реляционные данные
- MongoDB - документы
- Redis - кеш и сессии
- S3/MinIO - бинарные данные
- Local filesystem - разработка

See archive-final full content in git history. Key: Storage trait (get/set/delete/exists), WorkflowStorage, ExecutionStorage, BinaryStorage, TransactionalStorage.
