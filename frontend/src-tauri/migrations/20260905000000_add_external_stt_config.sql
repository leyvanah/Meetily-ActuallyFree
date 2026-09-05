-- External HTTP STT provider settings (GigaAM and other local speech services).
-- Stored as a JSON blob so new knobs don't need a migration each time.
ALTER TABLE transcript_settings ADD COLUMN externalSttConfig TEXT;
