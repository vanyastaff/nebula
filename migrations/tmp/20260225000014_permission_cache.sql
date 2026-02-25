-- ============================================================
-- 014: Permission Cache & Auto-provisioning Triggers
-- ============================================================

-- ============================================================
-- PERMISSION CACHE  (optional — speed up frequent auth checks)
-- ============================================================
-- Denormalized effective permissions per (user, resource).
-- Invalidated on any role/membership change.
-- Used by nebula-api middleware, backed by Redis in prod.
-- Kept in Postgres as fallback + source of truth snapshot.

CREATE TABLE permission_cache (
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    resource_type   acl_resource_type NOT NULL,
    resource_id     UUID NOT NULL,
    scopes          TEXT[] NOT NULL,                        -- effective granted scopes
    computed_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at      TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '5 minutes',
    PRIMARY KEY (user_id, resource_type, resource_id)
);

CREATE INDEX idx_perm_cache_expires ON permission_cache(expires_at);

-- Auto-invalidate on project membership change
CREATE OR REPLACE FUNCTION invalidate_permission_cache()
RETURNS TRIGGER AS $$
BEGIN
    DELETE FROM permission_cache
    WHERE user_id = COALESCE(NEW.user_id, OLD.user_id);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_invalidate_cache_pm
    AFTER INSERT OR UPDATE OR DELETE ON project_members
    FOR EACH ROW EXECUTE FUNCTION invalidate_permission_cache();

CREATE TRIGGER trg_invalidate_cache_tm
    AFTER INSERT OR UPDATE OR DELETE ON team_members
    FOR EACH ROW EXECUTE FUNCTION invalidate_permission_cache();

-- ============================================================
-- TRIGGER: auto-create personal project for new user in tenant
-- ============================================================

CREATE OR REPLACE FUNCTION create_personal_project_for_member()
RETURNS TRIGGER AS $$
DECLARE
    v_tenant_id UUID;
BEGIN
    -- Get default tenant for this org
    SELECT id INTO v_tenant_id
    FROM tenants
    WHERE organization_id = (
        SELECT organization_id FROM organization_members WHERE user_id = NEW.user_id LIMIT 1
    ) AND is_default = TRUE
    LIMIT 1;

    IF v_tenant_id IS NOT NULL THEN
        INSERT INTO projects (tenant_id, name, type, owner_user_id, created_by)
        SELECT v_tenant_id,
               u.username || '''s workspace',
               'personal',
               NEW.user_id,
               NEW.user_id
        FROM users u WHERE u.id = NEW.user_id
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_personal_project
    AFTER INSERT ON organization_members
    FOR EACH ROW EXECUTE FUNCTION create_personal_project_for_member();
