-- Durable trace of administrative actions (accounts, invitations). No foreign
-- key on purpose: deleting an account must not erase who deleted it.
-- Idempotent: CREATE ... IF NOT EXISTS.
CREATE TABLE IF NOT EXISTS audit_log (
    id INTEGER NOT NULL AUTO_INCREMENT PRIMARY KEY,
    at DATETIME NOT NULL,
    actor VARCHAR(64),
    action VARCHAR(64) NOT NULL,
    target VARCHAR(128),
    detail VARCHAR(200),
    result VARCHAR(16) NOT NULL,
    INDEX index_audit_log_at (at)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
