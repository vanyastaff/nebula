ALTER TABLE executions ADD COLUMN lease_holder TEXT;
ALTER TABLE executions ADD COLUMN lease_expires_at TIMESTAMPTZ;
