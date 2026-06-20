-- ADR-0099 W-S3a: durable resume-target carry on port_control_queue.
--
-- resume_target stores the serialized ResumeTarget JSON as TEXT.
-- NULL on rows written before this migration → deserializes as None.
-- Existing rows (non-Resume commands) also deserialize as None, which is
-- correct: only Resume commands carry a non-null target.
--
-- Down: ALTER TABLE port_control_queue DROP COLUMN resume_target;

ALTER TABLE port_control_queue
    ADD COLUMN resume_target TEXT;
