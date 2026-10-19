-- What a space is for, where the code has to know. The diary used to be found
-- by its slug, so renaming it — which moves the slug — left the reminder
-- believing nothing had been written in it, ever. A role survives a rename.
ALTER TABLE nodes ADD COLUMN role TEXT;
UPDATE nodes SET role = 'diary'
WHERE parent_id IS NULL AND kind = 'space' AND slug = 'diary';
CREATE UNIQUE INDEX idx_nodes_role ON nodes (role) WHERE role IS NOT NULL;

-- (parent_id, slug) is unique, but SQLite counts every NULL as distinct, so
-- that index never covered the spaces themselves: root slugs were unique only
-- because the code checked before it inserted.
CREATE UNIQUE INDEX idx_nodes_root_slug ON nodes (slug) WHERE parent_id IS NULL;
