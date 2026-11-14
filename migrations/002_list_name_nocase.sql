-- Agents refer to lists by name, so "Groceries" and "groceries" must be the same list.
CREATE UNIQUE INDEX list_name_nocase ON list (name COLLATE NOCASE);
