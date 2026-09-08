-- Aggregate-neutral: legacy workflow versions remain explicitly unactivated.
ALTER TABLE port_workflow_versions ADD COLUMN activation JSONB
    CHECK (activation IS NULL OR (
        jsonb_typeof(activation) = 'object'
        AND activation ?& ARRAY['workflow_version_id', 'executable_plan_id', 'worker_flavor_id']
        AND jsonb_typeof(activation->'workflow_version_id') = 'string'
        AND jsonb_typeof(activation->'executable_plan_id') = 'string'
        AND jsonb_typeof(activation->'worker_flavor_id') = 'string'
    ));

CREATE UNIQUE INDEX idx_workflow_versions_activation_identity
    ON port_workflow_versions ((activation->>'workflow_version_id'))
    WHERE activation IS NOT NULL;
