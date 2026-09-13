ALTER TABLE port_resources
    ADD COLUMN credential_bindings JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE port_execution_contract_bundles
    DROP CONSTRAINT port_execution_contract_bundles_record_format_check;
ALTER TABLE port_execution_contract_bundles
    ADD CONSTRAINT port_execution_contract_bundles_record_format_check
    CHECK (record_format IN ('v1_json', 'v2_json'));
