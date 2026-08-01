-- Claim-generation fencing for the control queue.
--
-- The same defect migration 0042 removed from `port_job_dispatch_queue`, in the
-- queue that carries accepted lifecycle commands. Acknowledgement was fenced on
-- `processed_by` — a stable processor identity — which cannot fence an ABA
-- reclaim: one consumer may claim a Cancel, lose it to the reclaim sweep, and
-- claim it again, at which point an acknowledgement issued against the first
-- claim still satisfies `processed_by = <self>` and terminalises a command the
-- second attempt is still dispatching.
--
-- `claim_generation` makes the two attempts distinguishable. Every successful
-- claim increments it in the same statement that flips Pending -> Processing,
-- and acknowledgement is fenced on the generation rather than on the processor.
-- Reclaim clears ownership but never decrements or reuses a generation, so a
-- superseded token can never match again.
--
-- `processed_by` is retained as bounded observability data (which runner held
-- which attempt); it is no longer an authority.
--
-- Existing rows start at generation 0. A row already in `Processing` when this
-- migration runs holds a claim whose in-flight token this deployment cannot
-- have minted, so its acknowledgement will not match and it falls to the
-- ordinary reclaim sweep — the fail-closed outcome, not a lost command.

ALTER TABLE port_control_queue
    ADD COLUMN claim_generation BIGINT NOT NULL DEFAULT 0
        CHECK (claim_generation >= 0);

COMMENT ON COLUMN port_control_queue.claim_generation IS
    'Monotonic per-row claim attempt; minted on claim, never decremented or reused, fences acknowledgement';
