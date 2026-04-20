# IDEA.md — Как я вижу Nebula

Это не спецификация и не роадмап. Это попытка отойти от реализации и описать **что за вещь ты строишь и зачем** — личная рамка поверх кода и канона.

Три жанра доков, чтобы не путать:

- [`docs/PRODUCT_CANON.md`](docs/PRODUCT_CANON.md) — что можно делать (нормативно).
- [`docs/COMPETITIVE.md`](docs/COMPETITIVE.md) — как мы сравниваемся с рынком (убеждающе).
- `IDEA.md` (этот файл) — зачем это вообще стоит строить (личная рамка).

---

## Содержание

1. [В одной фразе](#1-в-одной-фразе)
2. [Почему существует](#2-почему-существует)
3. [Главная ставка](#3-главная-ставка)
4. [Для кого](#4-для-кого)
5. [Форма продукта](#5-форма-продукта)
6. [Философия](#6-философия)
7. [Чем выигрывает](#7-чем-выигрывает)
8. [Чем не пытается быть](#8-чем-не-пытается-быть)
9. [Куда идёт](#9-куда-идёт)
10. [Финал](#10-финал)

---

## 1. В одной фразе

Nebula — это **серьёзный оркестратор рабочих процессов, где автор интеграции — первоклассный гражданин**, а не досадное приложение к UI.

Если нужно ещё короче — для инвестора или в лифте:

> "Мы делаем Temporal, но с n8n-уровнем DX для интеграций — на Rust, с честной моделью durability и без UI-first налога."

---

## 2. Почему существует

В мире workflow-движков есть две устойчивые дыры:

1. **Интеграции живут на периферии.** У n8n и Zapier тысяча коннекторов, но написаны они как скрипты-обёртки: без типов, без тестов, без внятной lifecycle модели. Community чинит их медленнее, чем ломаются API. Temporal даёт надёжный execution core, но писать workflow — это Go/TS/Java бойлерплейт, а не "расскажи системе, что делает твой шаг".
2. **Happy path принимается за норму.** Production workflow — это long-running, flaky API, рестарты посреди выполнения, ретраи, компенсация. Большинство движков обещают durability как feature, но реально доносит до прода её единицы.

Nebula бьёт в обе дыры одним движением: **делает durable execution дефолтом, а integration authoring — первоклассным DX**.

---

## 3. Главная ставка

> **Меньше честных гарантий лучше, чем много привлекательных, но мягких.**

Это и есть продуктовый нерв. Коротко:

- Типы на границах — неправильный integration contract ломается в 3 утра.
- Durable state как дефолт — "положил в память" это не orchestration.
- Пять ортогональных концепций (Resource, Credential, Action, Plugin, Schema) — путать auth rotation и connection lifecycle мы не хотим.
- Честный SDK — один стабильный фасад, а не 30 внутренних крейтов.

Эти ставки кодифицированы в [`PRODUCT_CANON.md` §2 "Position"](docs/PRODUCT_CANON.md#2-position) и [§3 "The problem & core thesis"](docs/PRODUCT_CANON.md#3-the-problem--core-thesis).

---

## 4. Для кого

**Primary:** разработчик, пишущий интеграцию. Он хочет `use nebula_sdk::prelude::*;`, написать `StatefulAction`, получить типизированные параметры, вернуть `Result`, и чтобы движок сам разобрался с ретраями, credential rotation, resource pooling и durability.

**Secondary:** команда, встраивающая Nebula в свою платформу. Ей нужен стабильный API-слой, понятная модель расширения, и уверенность, что engine не протечёт в её бизнес-код.

**Tertiary:** оператор, запускающий workflow. Он не главный, но его не забывают — observability первого класса ([`docs/OBSERVABILITY.md`](docs/OBSERVABILITY.md)), credential safety, предсказуемая lifecycle.

Явно **не** primary: юзер low-code UI, ETL-инженер на петабайтах данных, ML-пайплайн автор. Это другие рынки с другими ставками.

---

## 5. Форма продукта

Если убрать реализацию и смотреть с высоты, продукт держится на четырёх опорах:

- **Golden path** — автор описывает Action → регистрирует в registry → движок планирует исполнение → state durable → observability снаружи. Канонично: [`PRODUCT_CANON §10 "Golden path"`](docs/PRODUCT_CANON.md#10-golden-path-product).
- **Integration surface** — маленькая (5 концепций), ортогональная, расширяется через ADR. Полная модель: [`docs/INTEGRATION_MODEL.md`](docs/INTEGRATION_MODEL.md).
- **Plugin isolation** — out-of-process через `plugin-sdk`, чтобы чужой код не ронял движок. Детали: [`docs/INTEGRATION_MODEL.md` §7](docs/INTEGRATION_MODEL.md).
- **Две параллельные поверхности API** — `nebula-api` для runtime caller'ов и `nebula-sdk` для integration author'ов. Параллельны, не вложены.

Что **намеренно отсутствует** — это явные запреты канона, а не забывчивость:

- Implicit in-memory backbone → [`§12.2`](docs/PRODUCT_CANON.md#122-execution-single-semantic-core-durable-control-plane).
- Public surface, которую движок не honorит end-to-end → [`§4.5`](docs/PRODUCT_CANON.md#45-operational-honesty--no-false-capabilities).
- Capability, обещанная в доках без реализации в коде → [`§11.6`](docs/PRODUCT_CANON.md#116-documentation-truth).

---

## 6. Философия

Несколько вещей, которые я считываю из кода, канона и твоих реакций на ревью:

- **Clean design > backward compatibility.** Сломать чистый API лучше, чем нарастить на него адаптерный слой. "Shim" — ругательство.
- **Root cause > symptom.** Починить место, где баг возник, а не замазать там, где всплыл.
- **DX — это фича, а не документация.** Плохой API — это bug, даже если он "работает".
- **Security — не-переговариваемый инвариант.** Credential encryption, zeroization, redacted logs — не опция, а вход в игру. Формально: [`PRODUCT_CANON §4.2 "Safety"`](docs/PRODUCT_CANON.md#42-safety).
- **Канон — не тюрьма.** Если правило блокирует правильное улучшение — ADR, а не workaround. Условия пересмотра: [`§0.2 "When canon is wrong"`](docs/PRODUCT_CANON.md#02-when-canon-is-wrong-revision-triggers).

---

## 7. Чем выигрывает

Разбор ceiling'ов и наших bet'ов против каждого peer'а — в [`docs/COMPETITIVE.md` "Competitive bets"](docs/COMPETITIVE.md#competitive-bets). Здесь — только личная рамка: чего именно у них не хватает, что я хочу видеть в Nebula.

- У **n8n** — типобезопасности в месте, где в 3 утра ломается именно интеграция, а не UI.
- У **Temporal** — чтобы автор шага думал про шаг, а не про durable primitive.
- У **Airflow** — другого рынка: Nebula про события и интеграции, не про scheduled DAG'и.
- У **Windmill** — фокуса: не пытаться быть и IDE, и runtime, и low-code одновременно.

---

## 8. Чем не пытается быть

Это не менее важно, чем то, чем пытается:

- **Не низкокодовый UI-first instrument.** UI может появиться, но не как ядро.
- **Не batch/ETL movement at scale.** Не конкурирует с Spark/Flink/Airflow по TB/час.
- **Не general async runtime.** Это про workflow, не про "напиши любой асинхронный код".
- **Не replacement для Kubernetes operators.** Плагины изолированы, но это не пода-на-степ.
- **Не полигон абстракций.** Канон явно давит на "fewer real guarantees" — если абстракция не держит end-to-end, её не должно быть в публичном API.

---

## 9. Куда идёт

Моя проекция, не обязательство. Актуальный статус крейтов — [`docs/MATURITY.md`](docs/MATURITY.md).

- **Сейчас (alpha):** стабилизация execution core, честные invariants (lease-управление, CAS на переходах состояний, восстановление `ExecutionBudget` на resume), SDK façade оформлен.
- **Следующий горизонт:** Agent actions как полноценная integration family, health trait унификация (когда появится второй consumer), distributed execution с multi-node leases.
- **За горизонтом:** UI, marketplace интеграций, managed hosting — но только когда ядро станет скучным и надёжным.

---

## 10. Финал

Продукт не побеждает тем, что делает больше. Он побеждает тем, что делает **меньше, но честно**. Каждое "да" в канон — это обещание оператору в 3 утра. Nebula выбирает мало обещаний и много доставленного — и это, пожалуй, её самая сильная ставка.
