# IDEA.md — Как я вижу Nebula

Это не спецификация и не роадмап. Это попытка отойти от реализации и описать **что за вещь ты строишь и зачем** — как её видит человек со стороны, читающий канон, COMPETITIVE и код.

Канон (`docs/PRODUCT_CANON.md`) — что можно делать. Этот файл — **почему стоит**.

---

## Одна фраза

Nebula — это **серьёзный оркестратор рабочих процессов, где автор интеграции — первоклассный гражданин**, а не досадное приложение к UI.

---

## Почему существует

В мире workflow-движков есть две устойчивые дыры:

1. **Интеграции живут на периферии.** У n8n и Zapier тысяча коннекторов, но написаны они как скрипты-обёртки: без типов, без тестов, без внятной lifecycle модели. Community чинит их медленнее, чем ломаются API. Temporal даёт надёжный execution core, но писать workflow — это Go/TS/Java бойлерплейт, а не "расскажи системе, что делает твой шаг".
2. **Happy path принимается за норму.** Production workflow — это long-running, flaky API, рестарты посреди выполнения, ретраи, компенсация. Большинство движков обещают durability как feature, но реально доносит до прода её единицы.

Nebula бьёт в обе дыры одним движением: **делает durable execution дефолтом, а integration authoring — первоклассным DX**.

---

## Главная ставка

> **Меньше честных гарантий лучше, чем много привлекательных, но мягких.**

Это и есть продуктовый нерв. Nebula не хочет быть универсальным low-code или самым гибким graph-editor. Она хочет:

- Типы на границах — потому что неправильный integration contract ломается в 3 утра
- Durable state как дефолт — потому что "положил в память" — это не orchestration
- Integration model с пятью ортогональными концепциями (Resource, Credential, Action, Plugin, Schema) — потому что путать auth rotation и connection lifecycle — чужая боль, которую мы не хотим наследовать
- Честный SDK — потому что integration authors должны иметь стабильный фасад, а не копаться в 30 внутренних крейтах

---

## Для кого

**Primary:** разработчик, пишущий интеграцию. Он хочет `use nebula_sdk::prelude::*;`, написать `StatefulAction`, получить типизированные параметры, вернуть `Result`, и чтобы движок сам разобрался с ретраями, credential rotation, resource pooling и durability.

**Secondary:** команда, встраивающая Nebula в свою платформу. Ей нужен стабильный API-слой, понятная модель расширения, и уверенность, что engine не протечёт в её бизнес-код.

**Tertiary:** оператор, запускающий workflow. Он не главный, но его не забывают — observability первого класса (`docs/OBSERVABILITY.md`), credential safety, предсказуемая lifecycle.

Явно **не** primary: юзер low-code UI, ETL-инженер на петабайтах данных, ML-пайплайн автор. Это другие рынки с другими ставками.

---

## Философия

Несколько вещей, которые я считываю из кода, канона и твоих реакций на ревью:

- **Clean design > backward compatibility.** Сломать чистый API лучше, чем нарастить на него адаптерный слой. "Shim" — ругательство.
- **Root cause > symptom.** Починить место, где баг возник, а не замазать там, где всплыл.
- **DX — это фича, а не документация.** Плохой API — это bug, даже если он "работает".
- **Security — не-переговариваемый инвариант.** Credential encryption, zeroization, redacted logs — не опция, а вход в игру.
- **Канон — не тюрьма.** Если правило блокирует правильное улучшение — ADR, а не workaround. Правило может быть stale.

Эти принципы не декларации из README — они видны в том, как отказано от `release-plz`, как `nebula-sdk` изолирован в `deny.toml`, как `pitfalls.md` родился из session memory.

---

## Форма вещи

Если убрать реализацию и смотреть с высоты:

**Golden path:** автор описывает Action → регистрирует в registry → движок планирует исполнение → state durable → observability снаружи.

**Границы:**
- Integration surface — маленькая (5 концепций), ортогональная, расширяется через ADR.
- Execution core — durable, CAS-версионированный, с честной concurrency-моделью (leases, budgets, outbox).
- Plugin isolation — out-of-process через `plugin-sdk`, чтобы чужой код не ронял движок.
- Public API — две поверхности: `nebula-api` для runtime caller'ов (HTTP, webhook) и `nebula-sdk` для integration author'ов. Они **параллельны**, не вложены.

**Что намеренно отсутствует:** implicit in-memory backbone, hidden lifecycles, undocumented capabilities. Это явные запреты канона (§12.2, §4.5, §11.6).

---

## Чем Nebula выигрывает

- У **n8n** — типобезопасностью и серьёзной моделью durability. n8n прекрасен для citizen integrator, но серьёзный production — не его рынок.
- У **Temporal** — DX для integration authors. Temporal заставляет думать в рамках его SDK; Nebula даёт integration author сфокусироваться на шаге, а не на durable primitive.
- У **Airflow** — это вообще другой рынок (batch ETL), но: Nebula про события и интеграции, не про scheduled DAG'и.
- У **Windmill** — фокусом. Windmill хочет быть и IDE, и runtime, и low-code. Nebula узкая: orchestration + integrations, остальное — за границей.

---

## Чем Nebula **не** пытается быть

Это не менее важно, чем то, чем пытается:

- **Не низкокодовый UI-first instrument.** UI может появиться, но не как ядро.
- **Не batch/ETL movement at scale.** Не конкурирует с Spark/Flink/Airflow по TB/час.
- **Не general async runtime.** Это про workflow, не про "напиши любой асинхронный код".
- **Не replacement для Kubernetes operators.** Плагины изолированы, но это не пода-на-степ.
- **Не полигон абстракций.** Канон явно давит на "fewer real guarantees" — если абстракция не держит end-to-end, её не должно быть в публичном API.

---

## Куда это идёт (моя проекция, не обязательство)

- **Сейчас (alpha):** стабилизация execution core, честные invariants (см. свежие фиксы `ExecutionBudget`, leases, CAS), SDK façade оформлен.
- **Следующий горизонт:** Agent actions как полноценная integration family, health trait унификация (когда появится второй consumer — см. `project_health_trait.md`), distributed execution с multi-node leases.
- **За горизонтом:** UI, marketplace интеграций, managed hosting — но только когда ядро станет скучным и надёжным.

---

## Самая важная мысль

Продукт не побеждает тем, что делает больше. Он побеждает тем, что делает **меньше, но честно**. Каждое "да" в канон — это обещание оператору в 3 утра. Nebula выбирает мало обещаний и много доставленного — и это, пожалуй, её самая сильная ставка.

Если бы мне нужно было объяснить Nebula инвестору одним предложением:

> "Мы делаем Temporal, но с n8n-уровнем DX для интеграций — на Rust, с честной моделью durability и без UI-first налога."

Этого достаточно, чтобы отличить её от конкурентов и от искушения стать "ещё одним универсальным framework".
