-- 026: Invitations

CREATE TYPE invitation_type AS ENUM ('organization', 'project', 'team');

CREATE TABLE invitations (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    invited_email   VARCHAR(320) NOT NULL,
    invited_user_id UUID REFERENCES users(id) ON DELETE SET NULL,  -- NULL if user doesn't exist yet
    invitation_type invitation_type NOT NULL DEFAULT 'organization',
    -- For project/team invites
    project_id      UUID REFERENCES projects(id) ON DELETE CASCADE,
    team_id         UUID REFERENCES teams(id) ON DELETE CASCADE,
    -- Role to assign on accept
    org_role        VARCHAR(32),
    project_role    VARCHAR(64),
    custom_role_id  UUID REFERENCES roles(id) ON DELETE SET NULL,
    -- Invite lifecycle
    token           TEXT NOT NULL UNIQUE,                   -- secure random token
    invited_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    accepted_at     TIMESTAMPTZ,
    expires_at      TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '7 days',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_invitations_org ON invitations(organization_id);
CREATE INDEX idx_invitations_email ON invitations(invited_email);
CREATE INDEX idx_invitations_token ON invitations(token);
CREATE INDEX idx_invitations_pending ON invitations(expires_at)
    WHERE accepted_at IS NULL;
