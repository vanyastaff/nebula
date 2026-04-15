ALTER TABLE idempotency_keys
    ADD CONSTRAINT idempotency_keys_execution_fk
    FOREIGN KEY (execution_id)
    REFERENCES executions(id)
    ON DELETE CASCADE;
