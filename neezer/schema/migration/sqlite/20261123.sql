-- Durable trace of administrative actions (accounts, invitations). No foreign
-- key on purpose: deleting an account must not erase who deleted it.
-- Idempotent: CREATE ... IF NOT EXISTS.
CREATE TABLE IF NOT EXISTS audit_log (
    id INTEGER PRIMARY KEY,
    at DATETIME NOT NULL,
    actor VARCHAR(64),
    action VARCHAR(64) NOT NULL,
    target VARCHAR(128),
    detail VARCHAR(200),
    result VARCHAR(16) NOT NULL
);
CREATE INDEX IF NOT EXISTS index_audit_log_at ON audit_log(at);
