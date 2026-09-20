-- Operator-granted retry groups preserve every prior external write attempt.
ALTER TABLE delivery_action ADD COLUMN attempt_limit integer NOT NULL DEFAULT 3
 CHECK(attempt_limit >= 3);
