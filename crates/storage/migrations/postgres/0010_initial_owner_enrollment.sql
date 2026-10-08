-- Identity owns this permanent deployment admission record. It is not a child
-- of the account or tenant: purging either must never reopen initial setup.
CREATE TABLE initial_owner_enrollment (
    singleton      INTEGER     NOT NULL,
    state          TEXT        NOT NULL,
    user_id        BYTEA,
    tenant_request JSONB,
    recorded_at    TIMESTAMPTZ NOT NULL,
    CONSTRAINT pk_initial_owner_enrollment PRIMARY KEY (singleton),
    CONSTRAINT ck_initial_owner_enrollment__singleton CHECK (singleton = 1),
    CONSTRAINT ck_initial_owner_enrollment__state
        CHECK (state IN ('available', 'sealed', 'enrolled')),
    CONSTRAINT ck_initial_owner_enrollment__record CHECK (
        (state IN ('available', 'sealed') AND user_id IS NULL AND tenant_request IS NULL)
        OR (state = 'enrolled' AND user_id IS NOT NULL AND octet_length(user_id) = 16
            AND tenant_request IS NOT NULL AND jsonb_typeof(tenant_request) = 'object')
    )
);

-- Include archived accounts and historical tenant receipts. No current-state
-- check at a later startup may reverse this one-time eligibility decision.
INSERT INTO initial_owner_enrollment (singleton, state, recorded_at)
SELECT 1,
    CASE WHEN EXISTS (SELECT 1 FROM users)
              OR EXISTS (SELECT 1 FROM orgs)
              OR EXISTS (SELECT 1 FROM tenant_provisioning_receipts)
         THEN 'sealed' ELSE 'available' END,
    clock_timestamp();

-- Every ordinary identity writer seals eligibility in its own transaction.
-- Lock the singleton BEFORE insertion, matching initial enrollment's lock order.
-- Enrollment reserves 'enrolled' before inserting its account, so it is unchanged.
CREATE FUNCTION seal_initial_owner_on_user_insert() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE initial_owner_enrollment
       SET state = 'sealed', recorded_at = clock_timestamp()
     WHERE singleton = 1 AND state = 'available';
    RETURN NEW;
END;
$$;

CREATE TRIGGER tr_users__seal_initial_owner
BEFORE INSERT ON users
FOR EACH ROW EXECUTE FUNCTION seal_initial_owner_on_user_insert();
