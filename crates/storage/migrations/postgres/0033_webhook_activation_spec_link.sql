-- ADR-0101 L1 spec link: add spec_trigger_id to port_webhook_activations.
--
-- spec_trigger_id is the port_triggers row PK (TriggerId, trg_ prefix) that
-- this activation was built from.  Bootstrap reconstruct uses it to re-resolve
-- the webhook spec via TriggerSpecLookup::lookup without relying on trigger_id
-- (which is now the dispatch routing NodeKey, not the spec-row PK).
--
-- NULL on legacy rows written before this migration: bootstrap skips those rows
-- with MissingSpec rather than silently mis-routing.
--
-- Down (manual): ALTER TABLE port_webhook_activations DROP COLUMN spec_trigger_id;

ALTER TABLE port_webhook_activations
    ADD COLUMN spec_trigger_id TEXT;
