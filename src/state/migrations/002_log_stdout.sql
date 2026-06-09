-- sshi DB schema v2: add stdout preview column to operation_log
ALTER TABLE operation_log ADD COLUMN stdout TEXT;
