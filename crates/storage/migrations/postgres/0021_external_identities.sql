-- 0021: External identities (OAuth provider-to-user linkage)
-- Layer: Identity (Plane A)
-- Spec: ADR-0085 D-8 + REQ-oauth-005 / REQ-oauth-006
-- Migration sequenced after 0020 (resume result persistence).

-- Per ADR-0085 D-8: stable per-IdP identity link. `(provider, subject)`
-- is the PK because the IdP's `sub` claim is the source of truth for
-- "same human"; `user_id` is the Nebula-side foreign key with CASCADE
-- so deleting a user atomically purges every external link.
--
-- `user_id BYTEA` matches the `users.id` shape (16-byte ULID per
-- `0001_users.sql`); using UUID here would fail FK validation at
-- migration apply time.
--
-- `email` is the IdP-side email captured AT LINK TIME (audit only).
-- Per REQ-oauth-006 Scenario 6.2 the snapshot is NOT refreshed on
-- subsequent logins — the `subject` link takes precedence over the
-- email if the user later rotates their IdP-side address.
CREATE TABLE external_identities (
    provider     TEXT        NOT NULL,
    subject      TEXT        NOT NULL,
    user_id      BYTEA       NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    email        TEXT,
    linked_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (provider, subject)
);

-- Reverse lookup: "all external identities owned by this user". Used by
-- the future PR-5 admin endpoint that lists linked providers; not on
-- the OAuth hot path.
CREATE INDEX external_identities_user_id_idx
    ON external_identities (user_id);
