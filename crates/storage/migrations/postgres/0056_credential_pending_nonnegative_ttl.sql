-- Admit immediately-expired pending state while preserving the released 0055 checksum.

ALTER TABLE credential_pending_states
    DROP CONSTRAINT credential_pending_states_check;

ALTER TABLE credential_pending_states
    ADD CONSTRAINT credential_pending_states_check
    CHECK (expires_at >= created_at);
