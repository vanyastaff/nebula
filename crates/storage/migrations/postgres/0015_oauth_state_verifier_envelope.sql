-- The PKCE `code_verifier` of an OAuth login between redirect and callback
-- was stored in plaintext. It is now an identity-secret envelope bound to the
-- row's provider and state. A state row lives at most ten minutes and is
-- single-use, so the table is recreated rather than converted: a login in
-- flight across the upgrade fails its callback and is started again.
DROP TABLE oauth_states;

CREATE TABLE oauth_states (
    state                  TEXT        NOT NULL,
    provider               TEXT        NOT NULL,
    code_verifier_envelope BYTEA       NOT NULL,
    redirect_uri           TEXT,
    created_at             TIMESTAMPTZ NOT NULL,
    expires_at             TIMESTAMPTZ NOT NULL,
    consumed_at            TIMESTAMPTZ,
    CONSTRAINT pk_oauth_states PRIMARY KEY (state),
    CONSTRAINT ck_oauth_states__code_verifier_envelope_length
        CHECK (octet_length(code_verifier_envelope) BETWEEN 1 AND 4096)
);
CREATE INDEX ix_oauth_states__expires_at ON oauth_states (expires_at);
