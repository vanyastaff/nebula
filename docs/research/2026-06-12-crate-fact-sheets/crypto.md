# nebula-crypto — fact sheet

## Назначение
Leaf cross-cutting крейт криптопримитивов: AES-256-GCM authenticated encryption
(AAD обязателен в проде, SEC-11) + Argon2id KDF, конверт `EncryptedData` и
таксономия `CryptoError`. Извлечён из `nebula-credential` по ADR-0088, чтобы
contract-крейт credential не тянул aes-gcm/argon2, а `EncryptionLayer` в
`nebula-storage` брал примитивы напрямую. Весь крейт — один файл `src/lib.rs`.

## Публичная поверхность
- `CryptoError` (enum, `#[non_exhaustive]`, derive `nebula_error::Classify`) — src/lib.rs:34; варианты DecryptionFailed / EncryptionFailed / KeyDerivation / NonceGeneration / UnsupportedVersion / InvalidKeyId
- `EncryptionKey` (256-bit, `Zeroize + ZeroizeOnDrop`) — src/lib.rs:74
  - `EncryptionKey::derive_from_password(password, &[u8;16])` — Argon2id (19 MiB, t=2, p=1) — src/lib.rs:92
  - `EncryptionKey::from_bytes([u8;32])` — src/lib.rs:112 (`as_bytes` — pub(crate))
- `EncryptedData` (Serialize/Deserialize конверт: `version`, `key_id` `#[serde(default)]`, nonce 12B, ciphertext, tag 16B) — src/lib.rs:124
  - `EncryptedData::CURRENT_VERSION = 1` — src/lib.rs:147; `new(...)` :150; `is_supported_version()` :166
- `decrypt(key, &EncryptedData) -> Zeroizing<Vec<u8>>` — no-AAD decrypt — src/lib.rs:228
- `encrypt_with_aad(key, plaintext, aad)` — src/lib.rs:262 (key_id остаётся "")
- `decrypt_with_aad(key, &EncryptedData, aad)` — src/lib.rs:301
- `encrypt_with_key_id(key, key_id, plaintext, aad)` — отвергает пустой key_id (`InvalidKeyId`) — src/lib.rs:340
- `trait Cipher: Send + Sync` (ADR-0092 port; нет no-AAD encrypt-метода by construction) — src/lib.rs:386
- `trait Kdf: Send + Sync` — src/lib.rs:437
- `AesGcmCipher` (unit struct, default impl Cipher) — src/lib.rs:452
- `Argon2Kdf` (unit struct, default impl Kdf) — src/lib.rs:486
- приватный `fresh_nonce()` — 96-bit CSPRNG nonce (`rand::rng()`/ThreadRng) — src/lib.rs:186
- `encrypt_no_aad` — `#[cfg(test)]`-only (SEC-11) — src/lib.rs:201

## Workspace-зависимости
Deps (Cargo.toml): aes-gcm, argon2, zeroize, rand, serde(derive), thiserror,
**nebula-error** (features=["derive"]) — единственная workspace-зависимость.
Кто зависит от nebula-crypto:
- `crates/credential/Cargo.toml:22`
- `crates/storage/Cargo.toml:18`

## Структура модулей
- `src/lib.rs` — единственный файл: Error / EncryptionKey+EncryptedData+free fns / Cipher+Kdf ports (ADR-0092) / tests (~15 unit-тестов).

## Напряжения
- Коды ошибок сохраняют legacy-префикс `CREDENTIAL:CRYPTO_*` в generic-крейте — осознанный compat-долг, задокументирован (src/lib.rs:29-31); переименование сломает потребителей по всему credential-стеку.
- Асимметрия SEC-11: `encrypt_no_aad` спрятан под `#[cfg(test)]` (src/lib.rs:200), но no-AAD `decrypt` (src/lib.rs:228) остаётся pub — нужен для legacy/pre-AAD данных, однако это публичный путь, который проду нечем штатно произвести.
- `encrypt_with_aad` пишет пустой `key_id` (src/lib.rs:289), при том что doc на `EncryptedData.key_id` (src/lib.rs:130-132) говорит «encryption layer rejects empty key IDs at runtime» — инвариант непустого key_id живёт только в `encrypt_with_key_id`, дисциплина переложена на потребителя.
- Дубль поверхности by design: free functions и trait-методы `Cipher` полностью совпадают (trait делегирует, src/lib.rs:454-482) — два равноправных входа в одни и те же операции.
- README.md:14-15 «depends only on nebula-error plus the RustCrypto stack» — неточно: ещё serde/rand/thiserror (мелочь).
- AGENTS.md:20 «Cross-crate calls go through nebula-eventbus» — boilerplate из корневых правил, к leaf-крейту без eventbus-зависимости неприменим.
- TODO/FIXME/deprecated/shims — отсутствуют.

## Роль в credential/resource redesign
Прямой артефакт credential-rewrite: создан экстрактом из nebula-credential
(ADR-0088), а порты `Cipher`/`Kdf` (ADR-0092) — точка инверсии, через которую
credential `EncryptionLayer` становится generic по шифру (тест-фейки,
будущий ChaCha20/HSM). `key_id` в конверте + `InvalidKeyId` — несущая деталь
key-rotation пути (rotation-gated resolver в credential). Потребители:
nebula-credential и nebula-storage (`EncryptionLayer`). К resource redesign
напрямую не относится. План rewrite предполагал «new nebula-crypto» — этот
пункт уже реализован.
