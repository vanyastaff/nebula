-- 0036: make Plane-A OAuth-state expiry cleanup cover every row
-- Layer: Identity
--
-- Admission deletes expired rows regardless of whether a callback consumed
-- them. The original partial index excluded consumed rows and therefore could
-- not support that bounded cleanup sweep.

DROP INDEX IF EXISTS idx_plane_a_oauth_states_cleanup;

CREATE INDEX idx_plane_a_oauth_states_cleanup
    ON plane_a_oauth_states (expires_at);
