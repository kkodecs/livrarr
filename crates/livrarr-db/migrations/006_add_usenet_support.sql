-- Add Usenet support columns to download_clients.
-- Spec: USE-DLC-001, USE-DLC-004

-- client_type distinguishes qBittorrent from SABnzbd.
ALTER TABLE download_clients ADD COLUMN client_type TEXT NOT NULL DEFAULT 'qbittorrent';

-- SABnzbd uses API key auth instead of username/password.
ALTER TABLE download_clients ADD COLUMN api_key TEXT;

-- One default per protocol. Initially all false.
ALTER TABLE download_clients ADD COLUMN is_default_for_protocol BOOLEAN NOT NULL DEFAULT false;

-- Auto-promote: set first (lowest id) existing client as default for torrent.
UPDATE download_clients
SET is_default_for_protocol = true
WHERE id = (SELECT MIN(id) FROM download_clients);

-- Partial unique index: only one default per client_type.
CREATE UNIQUE INDEX idx_download_clients_default_per_protocol
ON download_clients (client_type)
WHERE is_default_for_protocol = true;
