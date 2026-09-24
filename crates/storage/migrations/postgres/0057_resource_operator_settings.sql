-- Operator runtime settings for stored resources, kept apart from the
-- resource config: topology (pool size, timeouts, concurrency mode) and the
-- override of the resilience the kind declares ({"rate": {...}}, validated
-- against the kind's policy before it is stored). NULL means the kind's
-- defaults, which is exactly how every existing row was activated before this
-- migration.
ALTER TABLE port_resources ADD COLUMN topology JSONB;
ALTER TABLE port_resources ADD COLUMN resilience_override JSONB;
