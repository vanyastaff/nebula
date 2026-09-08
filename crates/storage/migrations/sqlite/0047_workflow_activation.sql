-- Aggregate-neutral: legacy workflow versions remain explicitly unactivated.
ALTER TABLE port_workflow_versions ADD COLUMN activation TEXT
    CHECK (activation IS NULL OR (
        json_valid(activation)
        AND json_type(activation) = 'object'
        AND json_type(activation, '$.workflow_version_id') IS 'text'
        AND json_type(activation, '$.executable_plan_id') IS 'text'
        AND json_type(activation, '$.worker_flavor_id') IS 'text'
    ));

CREATE UNIQUE INDEX idx_workflow_versions_activation_identity
    ON port_workflow_versions (json_extract(activation, '$.workflow_version_id'))
    WHERE activation IS NOT NULL;
