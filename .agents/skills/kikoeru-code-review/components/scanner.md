Component: library scanning and metadata

Lenses: correctness, data

Files in scope: crates/neo-scanner/ (src and tests).

Book chapters: scanning.md, data-model.md (works, tracks, metadata),
storage.md (only how the scanner walks storage).

Look especially for: work-code (RJ/BJ/VJ and similar) matching that picks the
wrong work, misses a work, or matches two works to one folder; track
discovery that misorders tracks (natural sort), drops files, or treats
non-audio files as tracks; track IDs that change on a rescan with no real
change; a failed page or fetch that still deletes or marks data as missing;
metadata fetchers (DLsite, asmr.one, hvdb) that parse real responses wrongly
(missing fields, changed shapes, localized fields, age categories, price,
circle names), merge sources in the wrong priority, or overwrite user edits;
retry and rate-limit logic that hammers or gives up early; non-UTF-8 and
Unicode normalization issues in names.
