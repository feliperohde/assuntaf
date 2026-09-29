-- Migration: optional Qdrant vector store for the knowledge index.
-- vector_store: 'local' (vectors in SQLite only) or 'qdrant' (also written to and
-- searched in Qdrant; SQLite keeps text, keyword index and a fallback copy).

ALTER TABLE rag_config ADD COLUMN vector_store TEXT NOT NULL DEFAULT 'local';
ALTER TABLE rag_config ADD COLUMN qdrant_url TEXT;
ALTER TABLE rag_config ADD COLUMN qdrant_api_key TEXT;
ALTER TABLE rag_config ADD COLUMN qdrant_collection TEXT;
