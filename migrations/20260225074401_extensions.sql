-- ============================================================
-- 000: PostgreSQL Extensions
-- ============================================================
-- All required extensions are created upfront so that subsequent
-- migrations can rely on them (uuid generation, cryptography,
-- trigram indexes).
-- ============================================================

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pgcrypto";
CREATE EXTENSION IF NOT EXISTS "pg_trgm";
