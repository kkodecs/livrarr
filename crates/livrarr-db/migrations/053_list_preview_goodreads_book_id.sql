-- Carry the Goodreads "Book Id" from the CSV parser through preview persistence
-- so list confirm can seed it as gr_key (REQ-006). Nullable: only Goodreads CSVs
-- carry a Book Id; Hardcover and other sources leave it NULL.
ALTER TABLE list_import_previews ADD COLUMN goodreads_book_id TEXT;
