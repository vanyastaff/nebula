-- Operator runtime settings for stored resources, kept apart from the
-- resource config: topology (pool size, timeouts, concurrency mode) and the
-- per-row rate limit. NULL means the kind's defaults / no limit, which is
-- exactly how every existing row was activated before this migration.
ALTER TABLE port_resources ADD COLUMN topology TEXT;
ALTER TABLE port_resources ADD COLUMN rate_limit TEXT;
