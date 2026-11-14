-- Tags are matched case-insensitively so "Urgent" and "urgent" can't split into two tags.
CREATE UNIQUE INDEX tag_name_nocase ON tag (name COLLATE NOCASE);
