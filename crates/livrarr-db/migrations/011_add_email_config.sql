-- Email / Send to Kindle configuration (singleton)
CREATE TABLE IF NOT EXISTS email_config (
    id              INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    enabled         BOOLEAN NOT NULL DEFAULT 0,
    smtp_host       TEXT NOT NULL DEFAULT 'smtp.gmail.com',
    smtp_port       INTEGER NOT NULL DEFAULT 587,
    encryption      TEXT NOT NULL DEFAULT 'starttls',
    username        TEXT,
    password        TEXT,
    from_address    TEXT,
    recipient_email TEXT,
    send_on_import  BOOLEAN NOT NULL DEFAULT 0
);

INSERT OR IGNORE INTO email_config (id) VALUES (1);

UPDATE _livrarr_meta SET value = '11' WHERE key = 'schema_version';
