use rusqlite::Connection;
use std::collections::HashMap;

pub struct VfsBuilder {
    conn: Connection,
    project_id: i64,
    next_entry_id: i64,
    next_edge_id: i64,
}

impl VfsBuilder {
    pub fn new(conn: Connection, project_id: i64) -> Self {
        let next_entry_id = Self::next_id(&conn, "vfs_entries");
        let next_edge_id = Self::next_id(&conn, "vfs_edges");
        VfsBuilder {
            conn,
            project_id,
            next_entry_id,
            next_edge_id,
        }
    }

    pub fn build(&mut self) -> rusqlite::Result<()> {
        self.build_directory_tree()?;
        self.build_file_entries()?;
        self.build_node_entries()?;
        self.build_vfs_edges()?;
        Ok(())
    }

    pub fn into_connection(self) -> Connection {
        self.conn
    }

    fn next_id(conn: &Connection, table: &str) -> i64 {
        conn.query_row(
            &format!("SELECT COALESCE(MAX(id), 0) + 1 FROM {}", table),
            [],
            |row| row.get(0),
        )
        .unwrap_or(1)
    }

    fn build_directory_tree(&mut self) -> rusqlite::Result<()> {
        // Build directory entries from file paths
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT project_rel_path FROM files WHERE project_id = ?1")?;

        let paths: Vec<String> = stmt
            .query_map(rusqlite::params![self.project_id], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        drop(stmt);

        let mut seen_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
        seen_dirs.insert("".to_string());

        for path in &paths {
            let normalized = path.replace('\\', "/");
            let parts: Vec<&str> = normalized.split('/').collect();
            let mut current = String::new();

            for (i, part) in parts.iter().enumerate() {
                if part.is_empty() {
                    continue;
                }
                let parent = current.clone();
                if current.is_empty() {
                    current = part.to_string();
                } else {
                    current = format!("{}/{}", current, part);
                }

                // Only create directory entries for intermediate paths (not the file itself)
                if (i < parts.len() - 1 || path.ends_with('/')) && seen_dirs.insert(current.clone())
                {
                    self.conn.execute(
                            "INSERT OR IGNORE INTO vfs_entries
                             (id, project_id, entry_type, entry_kind, vfs_path, parent_vfs_path, display_name)
                             VALUES (?1, ?2, 'directory', 'directory', ?3, ?4, ?5)",
                            rusqlite::params![
                                self.next_entry_id,
                                self.project_id,
                                &current,
                                if parent.is_empty() { None } else { Some(&parent) },
                                part,
                            ],
                        )?;
                    self.next_entry_id += 1;
                }
            }
        }

        Ok(())
    }

    fn build_file_entries(&mut self) -> rusqlite::Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_rel_path, kind, guid
             FROM files
             WHERE project_id = ?1 AND kind != 'meta'",
        )?;

        let files: Vec<(i64, String, String, Option<String>)> = stmt
            .query_map(rusqlite::params![self.project_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        drop(stmt);

        for (file_id, rel_path, kind, _guid) in files {
            let normalized = rel_path.replace('\\', "/");
            let parent = normalized.rsplit_once('/').map(|(p, _)| p.to_string());

            self.conn.execute(
                "INSERT OR IGNORE INTO vfs_entries
                 (id, project_id, entry_type, entry_kind, vfs_path, parent_vfs_path, source_file_id, display_name)
                 VALUES (?1, ?2, 'file', ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    self.next_entry_id,
                    self.project_id,
                    &kind,
                    &normalized,
                    parent.as_deref(),
                    file_id,
                    normalized.rsplit('/').next().unwrap_or(&normalized),
                ],
            )?;
            self.next_entry_id += 1;
        }

        Ok(())
    }

    fn build_node_entries(&mut self) -> rusqlite::Result<()> {
        // Create node entries for entities (GameObjects, Components, Materials, etc.)
        let mut stmt = self.conn.prepare(
            "SELECT e.id, e.asset_id, e.entity_kind, e.local_key, e.name, e.type_name,
                    a.vfs_root_path, e.parent_entity_id
             FROM entities e
             JOIN assets a ON e.asset_id = a.id
             WHERE a.project_id = ?1",
        )?;

        let entities: Vec<(
            i64,
            i64,
            String,
            String,
            Option<String>,
            String,
            String,
            Option<i64>,
        )> = stmt
            .query_map(rusqlite::params![self.project_id], |row| {
                Ok((
                    row.get(0)?,                   // entity_id
                    row.get(1)?,                   // asset_id
                    row.get(2)?,                   // entity_kind
                    row.get(3)?,                   // local_key
                    row.get(4)?,                   // name
                    row.get(5)?,                   // type_name
                    row.get(6)?,                   // vfs_root_path
                    row.get::<_, Option<i64>>(7)?, // parent_entity_id
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        drop(stmt);

        // Build entity_id → local_key lookup for parent resolution
        let mut entity_local_keys: HashMap<i64, String> = HashMap::new();
        for (_, _, _, _local_key, _, _, _, _) in &entities {
            // We need the entity_id to local_key map separately
        }

        // Re-query to build the lookup
        let mut key_stmt = self.conn.prepare(
            "SELECT id, local_key FROM entities WHERE asset_id IN
             (SELECT id FROM assets WHERE project_id = ?1)",
        )?;
        let key_rows: Vec<(i64, String)> = key_stmt
            .query_map(rusqlite::params![self.project_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(key_stmt);

        for (eid, lk) in &key_rows {
            entity_local_keys.insert(*eid, lk.clone());
        }

        for (
            entity_id,
            _asset_id,
            entity_kind,
            local_key,
            name,
            type_name,
            vfs_root_path,
            _parent_entity_id,
        ) in &entities
        {
            // VFS path: <vfs_root_path>:/<entity_kind>/<local_key>
            let vfs_path = format!("{}:/{}", vfs_root_path, local_key);
            let display_name = name.clone().unwrap_or_else(|| type_name.clone());

            self.conn.execute(
                "INSERT OR IGNORE INTO vfs_entries
                 (id, project_id, entry_type, entry_kind, vfs_path, parent_vfs_path,
                  source_entity_id, display_name)
                 VALUES (?1, ?2, 'node', ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    self.next_entry_id,
                    self.project_id,
                    entity_kind,
                    &vfs_path,
                    vfs_root_path,
                    entity_id,
                    display_name,
                ],
            )?;
            self.next_entry_id += 1;
        }

        Ok(())
    }

    fn build_vfs_edges(&mut self) -> rusqlite::Result<()> {
        // 1. child_of edges: directory → file
        self.conn.execute(
            "INSERT OR IGNORE INTO vfs_edges (id, from_entry_id, to_entry_id, edge_kind)
             SELECT ?1 + ROW_NUMBER() OVER (ORDER BY e.vfs_path), e.id, d.id, 'child_of'
             FROM vfs_entries e
             JOIN vfs_entries d ON e.parent_vfs_path = d.vfs_path
             WHERE e.project_id = ?2 AND d.project_id = ?2",
            rusqlite::params![self.next_edge_id, self.project_id],
        )?;
        self.next_edge_id += self.conn.changes() as i64;

        // 2. defined_in edges: node → file
        self.conn.execute(
            "INSERT OR IGNORE INTO vfs_edges (id, from_entry_id, to_entry_id, edge_kind)
             SELECT ?1 + ROW_NUMBER() OVER (ORDER BY n.vfs_path), n.id, f.id, 'defined_in'
             FROM vfs_entries n
             JOIN vfs_entries f ON n.parent_vfs_path = f.vfs_path
             WHERE n.project_id = ?2 AND f.project_id = ?2
               AND n.entry_type = 'node' AND f.entry_type = 'file'",
            rusqlite::params![self.next_edge_id, self.project_id],
        )?;
        self.next_edge_id += self.conn.changes() as i64;

        // 3. depends_on edges: file → file (from yaml_references via guid)
        self.conn.execute(
            "INSERT OR IGNORE INTO vfs_edges (id, from_entry_id, to_entry_id, edge_kind, edge_subkind)
             SELECT DISTINCT
                ?1 + ROW_NUMBER() OVER (ORDER BY from_entry.id, to_entry.id),
                from_entry.id, to_entry.id, 'depends_on', yr.ref_kind
             FROM yaml_references yr
             JOIN files from_file ON yr.file_id = from_file.id
             JOIN files to_file ON lower(to_file.guid) = lower(yr.target_guid)
             JOIN vfs_entries from_entry ON from_entry.source_file_id = from_file.id AND from_entry.entry_type = 'file'
             JOIN vfs_entries to_entry ON to_entry.source_file_id = to_file.id AND to_entry.entry_type = 'file'
             WHERE from_file.project_id = ?2 AND to_file.project_id = ?2
               AND yr.target_guid IS NOT NULL",
            rusqlite::params![self.next_edge_id, self.project_id],
        )?;
        self.next_edge_id += self.conn.changes() as i64;

        // 4. binds_to edges: component node → script class node
        self.conn.execute(
            "INSERT OR IGNORE INTO vfs_edges (id, from_entry_id, to_entry_id, edge_kind, edge_subkind)
             SELECT ?1 + ROW_NUMBER() OVER (ORDER BY comp_entity.id),
                    comp_entry.id, script_entry.id, 'binds_to', 'component_script'
             FROM entities comp_entity
             JOIN entities script_symbol ON comp_entity.script_symbol_id = script_symbol.id
             JOIN vfs_entries comp_entry ON comp_entry.source_entity_id = comp_entity.id
             JOIN vfs_entries script_entry ON script_entry.source_entity_id = script_symbol.id
             WHERE comp_entity.entity_kind = 'component'",
            rusqlite::params![self.next_edge_id],
        )?;
        self.next_edge_id += self.conn.changes() as i64;

        // 5. instance_of edges: prefab instance → source prefab
        self.conn.execute(
            "INSERT OR IGNORE INTO vfs_edges (id, from_entry_id, to_entry_id, edge_kind)
             SELECT DISTINCT
                ?1 + ROW_NUMBER() OVER (ORDER BY from_entry.id, to_entry.id),
                from_entry.id, to_entry.id, 'instance_of'
             FROM yaml_references yr
             JOIN files from_file ON yr.file_id = from_file.id
             JOIN files to_file ON lower(to_file.guid) = lower(yr.target_guid)
             JOIN vfs_entries from_entry ON from_entry.source_file_id = from_file.id AND from_entry.entry_type = 'file'
             JOIN vfs_entries to_entry ON to_entry.source_file_id = to_file.id AND to_entry.entry_type = 'file'
             WHERE from_file.project_id = ?2 AND to_file.project_id = ?2
               AND yr.target_guid IS NOT NULL
               AND from_file.kind IN ('scene', 'prefab')",
            rusqlite::params![self.next_edge_id, self.project_id],
        )?;
        self.next_edge_id += self.conn.changes() as i64;

        Ok(())
    }
}
