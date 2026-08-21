use crate::cli::IndexAction;
use crate::db;
use crate::discovery;
use crate::extract;
use crate::materialize::VfsBuilder;
use crate::model::{AssetKind, FileKind};
use crate::resolve::{script_binding, EntityGraphBuilder};
use indicatif::{ProgressBar, ProgressStyle};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

pub fn run_index_action(action: IndexAction) -> anyhow::Result<()> {
    match action {
        IndexAction::Build {
            project,
            output,
            packages,
        } => run_build(project, output, packages),
        IndexAction::Sync { project } => run_sync(project),
        IndexAction::Status { project } => run_status(project),
    }
}

fn default_db_path(project: &Path) -> PathBuf {
    project.join(".unity-refgraph").join("index.db")
}

pub fn run_build_internal(project_root: &Path) -> anyhow::Result<()> {
    run_build(project_root.to_path_buf(), None, true)
}

fn run_build(
    project: PathBuf,
    output: Option<PathBuf>,
    include_packages: bool,
) -> anyhow::Result<()> {
    let project_root = project.canonicalize()?;
    let db_path = output.unwrap_or_else(|| default_db_path(&project_root));

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Remove existing DB
    if db_path.exists() {
        std::fs::remove_file(&db_path)?;
    }

    tracing::info!("Indexing project: {}", project_root.display());

    // Phase 1: Discovery
    let pb = ProgressBar::new(0);
    pb.set_style(ProgressStyle::with_template(
        "{spinner} {msg} {wide_bar} {pos}/{len}",
    )?);
    pb.set_message("Discovering files...");
    let discovery_result = discovery::discover_files(&project_root, include_packages);
    let total_files = discovery_result.files.len();
    pb.set_length(total_files as u64);
    pb.finish_with_message(format!("Discovered {} files", total_files));

    // Phase 2: Open DB and create schema
    let conn = db::open_db(&db_path)?;
    db::init_schema(&conn)?;

    // Insert project record
    conn.execute(
        "INSERT INTO projects (id, project_path, schema_version, indexed_at)
         VALUES (1, ?1, 1, ?2)",
        params![
            project_root.to_string_lossy(),
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    // Use current time without chrono dependency
    conn.execute(
        "UPDATE projects SET indexed_at = datetime('now') WHERE id = 1",
        [],
    )?;

    // Phase 3: Insert files
    let pb = ProgressBar::new(total_files as u64);
    pb.set_style(ProgressStyle::with_template(
        "{spinner} Inserting files... {wide_bar} {pos}/{len}",
    )?);
    for file in &discovery_result.files {
        conn.execute(
            "INSERT INTO files (id, project_id, project_rel_path, abs_path, kind, guid,
                                size_bytes, mtime_ms, content_hash, importer_type)
             VALUES (NULL, 1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                file.project_rel_path,
                file.abs_path,
                file.kind.as_str(),
                file.guid,
                file.size_bytes as i64,
                file.mtime_ms,
                file.content_hash,
                file.importer_type,
            ],
        )?;
        pb.inc(1);
    }
    pb.finish_with_message("Files inserted");

    // Phase 4: Extract and insert YAML objects + references
    let pb = ProgressBar::new(total_files as u64);
    pb.set_style(ProgressStyle::with_template(
        "{spinner} Extracting... {wide_bar} {pos}/{len}",
    )?);

    let yaml_files: Vec<_> = discovery_result
        .files
        .iter()
        .filter(|f| f.kind.is_unity_yaml())
        .collect();

    let cs_files: Vec<_> = discovery_result
        .files
        .iter()
        .filter(|f| f.kind == FileKind::CSharp)
        .collect();

    pb.set_length((yaml_files.len() + cs_files.len()) as u64);
    pb.set_message("Extracting YAML + C#...");

    // Process YAML files
    for file in &yaml_files {
        let content = match std::fs::read_to_string(&file.abs_path) {
            Ok(c) => c,
            Err(_) => {
                pb.inc(1);
                continue;
            }
        };

        let file_id: i64 = conn.query_row(
            "SELECT id FROM files WHERE project_rel_path = ?1",
            params![&file.project_rel_path],
            |row| row.get(0),
        )?;

        if let Some(result) = extract::extract_from_unity_yaml(&content) {
            let mut next_yaml_obj_id = conn.query_row(
                "SELECT COALESCE(MAX(id), 0) + 1 FROM yaml_objects",
                [],
                |row| row.get::<_, i64>(0),
            )?;

            let mut next_ref_id = conn.query_row(
                "SELECT COALESCE(MAX(id), 0) + 1 FROM yaml_references",
                [],
                |row| row.get::<_, i64>(0),
            )?;

            for obj in &result.objects {
                conn.execute(
                    "INSERT INTO yaml_objects
                     (id, file_id, doc_index, unity_class_id, anchor, object_type,
                      local_identifier, game_object_file_id, component_type_name,
                      script_guid, script_file_id, name, line_start, line_end)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    params![
                        next_yaml_obj_id,
                        file_id,
                        obj.doc_index as i64,
                        obj.unity_class_id,
                        obj.anchor,
                        obj.object_type,
                        obj.local_identifier,
                        obj.game_object_file_id,
                        obj.component_type_name,
                        obj.script_guid,
                        obj.script_file_id,
                        obj.name,
                        obj.line_start,
                        obj.line_end,
                    ],
                )?;
                next_yaml_obj_id += 1;
            }

            for r in &result.references {
                conn.execute(
                    "INSERT INTO yaml_references
                     (id, file_id, source_yaml_object_id, field_path,
                      target_guid, target_file_id, target_local_id, ref_kind)
                     VALUES (?1, ?2,
                        (SELECT id FROM yaml_objects WHERE file_id = ?2 AND local_identifier = ?3 LIMIT 1),
                        ?4, ?5, ?6, ?7, ?8)",
                    params![
                        next_ref_id, file_id, r.source_local_identifier,
                        r.field_path, r.target_guid, r.target_file_id,
                        r.target_local_id, r.ref_kind,
                    ],
                )?;
                next_ref_id += 1;
            }
        }
        pb.inc(1);
    }

    // Process C# files
    for file in &cs_files {
        let content = match std::fs::read_to_string(&file.abs_path) {
            Ok(c) => c,
            Err(_) => {
                pb.inc(1);
                continue;
            }
        };

        let file_id: i64 = conn.query_row(
            "SELECT id FROM files WHERE project_rel_path = ?1",
            params![&file.project_rel_path],
            |row| row.get(0),
        )?;

        if let Some(result) = extract::extract_from_csharp(&content) {
            let mut next_decl_id = conn.query_row(
                "SELECT COALESCE(MAX(id), 0) + 1 FROM cs_declarations",
                [],
                |row| row.get::<_, i64>(0),
            )?;

            for decl in &result.declarations {
                conn.execute(
                    "INSERT INTO cs_declarations
                     (id, file_id, decl_kind, simple_name, qualified_name, signature, line_start, line_end)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        next_decl_id, file_id, decl.decl_kind, decl.simple_name,
                        decl.qualified_name, decl.signature,
                        decl.line_start as i64, decl.line_end as i64,
                    ],
                )?;
                next_decl_id += 1;
            }

            let mut next_mention_id = conn.query_row(
                "SELECT COALESCE(MAX(id), 0) + 1 FROM cs_mentions",
                [],
                |row| row.get::<_, i64>(0),
            )?;

            for mention in &result.mentions {
                let containing_id: Option<i64> = if let Some(ref decl_name) =
                    mention.containing_declaration
                {
                    conn.query_row(
                        "SELECT id FROM cs_declarations WHERE file_id = ?1 AND simple_name = ?2 ORDER BY id LIMIT 1",
                        params![file_id, decl_name],
                        |row| row.get(0),
                    ).ok()
                } else {
                    None
                };

                conn.execute(
                    "INSERT INTO cs_mentions
                     (id, file_id, mention_kind, text, receiver_text, containing_declaration_id, line_start, line_end)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        next_mention_id, file_id, mention.mention_kind, mention.text,
                        mention.receiver_text, containing_id,
                        mention.line_start as i64, mention.line_end as i64,
                    ],
                )?;
                next_mention_id += 1;
            }
        }
        pb.inc(1);
    }
    pb.finish_with_message("Extraction complete");

    // Phase 5: Build assets
    let pb = ProgressBar::new(0);
    pb.set_message("Building assets...");
    let mut next_asset_id =
        conn.query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM assets", [], |row| {
            row.get::<_, i64>(0)
        })?;

    let asset_files: Vec<_> = discovery_result
        .files
        .iter()
        .filter(|f| f.guid.is_some() && f.kind != FileKind::Meta)
        .collect();

    for file in &asset_files {
        let guid = file.guid.as_ref().unwrap();
        let file_id: i64 = conn.query_row(
            "SELECT id FROM files WHERE project_rel_path = ?1",
            params![&file.project_rel_path],
            |row| row.get(0),
        )?;

        let asset_kind = AssetKind::from_file_kind(&file.kind).unwrap_or(AssetKind::YamlAsset);

        let name = Path::new(&file.project_rel_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        conn.execute(
            "INSERT OR IGNORE INTO assets (id, project_id, file_id, asset_kind, guid, name, vfs_root_path)
             VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6)",
            params![next_asset_id, file_id, asset_kind.as_str(), guid, name, file.project_rel_path],
        )?;
        next_asset_id += 1;
    }
    pb.finish_with_message(format!("Built {} assets", asset_files.len()));

    // Phase 6: Build entities
    let pb = ProgressBar::new(0);
    pb.set_message("Building entity graph...");

    let mut next_entity_id =
        conn.query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM entities", [], |row| {
            row.get::<_, i64>(0)
        })?;

    let assets: Vec<(i64, i64, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, file_id, asset_kind, vfs_root_path FROM assets WHERE project_id = 1",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        let mut result = Vec::new();
        for v in rows.flatten() {
            result.push(v);
        }
        result
    };

    for (asset_id, file_id, asset_kind_str, _vfs_root) in &assets {
        let asset_kind = parse_asset_kind(asset_kind_str);

        // Load YAML objects for this file
        let yaml_objects: Vec<crate::extract::YamlObject> = {
            let mut stmt = conn.prepare(
                "SELECT doc_index, unity_class_id, anchor, object_type, local_identifier,
                        game_object_file_id, component_type_name, script_guid, script_file_id,
                        name, line_start, line_end
                 FROM yaml_objects WHERE file_id = ?1",
            )?;

            let rows: Vec<_> = stmt
                .query_map(params![file_id], |row| {
                    Ok(crate::extract::YamlObject {
                        doc_index: row.get::<_, i64>(0)? as usize,
                        unity_class_id: row.get(1)?,
                        anchor: row.get(2)?,
                        object_type: row.get(3)?,
                        local_identifier: row.get(4)?,
                        game_object_file_id: row.get(5)?,
                        component_type_name: row.get(6)?,
                        script_guid: row.get(7)?,
                        script_file_id: row.get(8)?,
                        name: row.get(9)?,
                        line_start: row.get(10)?,
                        line_end: row.get(11)?,
                        payload: serde_yaml::Value::Null,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        let mut builder = EntityGraphBuilder::new();
        builder.build_for_asset(*asset_id, &asset_kind, &yaml_objects);

        // Insert entities
        for entity in &builder.entities {
            let yaml_obj_id: Option<i64> = if entity.yaml_object_id.is_some() {
                entity.yaml_object_id
            } else {
                // Try to find yaml_object_id by local_key
                conn.query_row(
                    "SELECT id FROM yaml_objects WHERE file_id = ?1 AND local_identifier = ?2 LIMIT 1",
                    params![file_id, entity.local_key],
                    |row| row.get(0),
                ).ok()
            };

            conn.execute(
                "INSERT INTO entities
                 (id, asset_id, yaml_object_id, entity_kind, local_key, name,
                  type_name, parent_entity_id, line_start, line_end)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7,
                   (SELECT id FROM entities WHERE asset_id = ?2 AND local_key = ?8 LIMIT 1),
                   ?9, ?10)",
                params![
                    next_entity_id,
                    asset_id,
                    yaml_obj_id,
                    entity.entity_kind.as_str(),
                    entity.local_key,
                    entity.name,
                    entity.type_name,
                    entity.parent_local_key,
                    entity.line_start,
                    entity.line_end,
                ],
            )?;
            next_entity_id += 1;
        }

        // Insert edges
        let mut next_edge_id = conn.query_row(
            "SELECT COALESCE(MAX(id), 0) + 1 FROM entity_edges",
            [],
            |row| row.get::<_, i64>(0),
        )?;

        for edge in &builder.edges {
            // Resolve local keys to entity IDs
            let from_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM entities WHERE asset_id = ?1 AND local_key = ?2",
                    params![asset_id, edge.from_local_key],
                    |row| row.get(0),
                )
                .ok();

            let to_id: Option<i64> = if edge.to_local_key.starts_with("guid:") {
                None // Cross-asset edges resolved later
            } else {
                conn.query_row(
                    "SELECT id FROM entities WHERE asset_id = ?1 AND local_key = ?2",
                    params![asset_id, edge.to_local_key],
                    |row| row.get(0),
                )
                .ok()
            };

            if let (Some(from), Some(to)) = (from_id, to_id) {
                conn.execute(
                    "INSERT OR IGNORE INTO entity_edges (id, from_entity_id, to_entity_id, edge_kind, edge_subkind)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![next_edge_id, from, to, edge.edge_kind, edge.edge_subkind],
                )?;
                next_edge_id += 1;
            }
        }
    }
    pb.finish_with_message("Entity graph built");

    // Phase 7: Script binding
    let pb = ProgressBar::new(0);
    pb.set_message("Binding scripts to symbols...");
    let bound = script_binding::bind_scripts_to_symbols(&conn)?;
    pb.finish_with_message(format!("Bound {} script bindings", bound));

    // Phase 8: Build symbols from declarations
    build_symbols(&conn)?;

    // Phase 9: Record summary (before conn is moved into VfsBuilder)
    conn.execute(
        "INSERT OR REPLACE INTO rebuild_summary
         (project_id, mode, discovered_file_count, diagnostic_count,
          completed_stages_json, published_index_path, created_at)
         VALUES (1, 'full', ?1, 0, '[]', ?2, datetime('now'))",
        params![total_files as i64, db_path.to_string_lossy()],
    )?;

    // Phase 10: VFS materialization
    let pb = ProgressBar::new(0);
    pb.set_message("Materializing VFS...");
    let mut vfs_builder = VfsBuilder::new(conn, 1);
    vfs_builder.build()?;
    pb.finish_with_message("VFS materialized");

    println!(
        "✅ Index built successfully: {} ({} files)",
        db_path.display(),
        total_files
    );
    Ok(())
}

fn build_symbols(conn: &Connection) -> rusqlite::Result<()> {
    // Create symbols from cs_declarations
    let mut next_symbol_id =
        conn.query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM symbols", [], |row| {
            row.get::<_, i64>(0)
        })?;

    let mut stmt = conn.prepare(
        "SELECT id, file_id, decl_kind, simple_name, qualified_name, line_start, line_end
         FROM cs_declarations ORDER BY file_id, id",
    )?;

    let declarations: Vec<(i64, i64, String, String, String, i64, i64)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?, // declaration id
                row.get(1)?, // file_id
                row.get(2)?, // decl_kind
                row.get(3)?, // simple_name
                row.get(4)?, // qualified_name
                row.get(5)?, // line_start
                row.get(6)?, // line_end
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);

    for (decl_id, file_id, decl_kind, simple_name, qualified_name, line_start, line_end) in
        declarations
    {
        conn.execute(
            "INSERT INTO symbols
             (id, project_id, file_id, declaration_id, symbol_kind, simple_name,
              qualified_name, display_name, line_start, line_end)
             VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?5, ?7, ?8)",
            params![
                next_symbol_id,
                file_id,
                decl_id,
                &decl_kind,
                &simple_name,
                &qualified_name,
                line_start,
                line_end,
            ],
        )?;
        next_symbol_id += 1;
    }

    Ok(())
}

fn parse_asset_kind(s: &str) -> AssetKind {
    match s {
        "scene" => AssetKind::Scene,
        "prefab" => AssetKind::Prefab,
        "material" => AssetKind::Material,
        "script" => AssetKind::Script,
        "scriptable_object" => AssetKind::ScriptableObject,
        "yaml-asset" => AssetKind::YamlAsset,
        _ => AssetKind::YamlAsset,
    }
}

fn run_sync(project: PathBuf) -> anyhow::Result<()> {
    // For now, sync = rebuild
    tracing::warn!("Sync not yet implemented, falling back to full rebuild");
    run_build(project, None, true)
}

fn run_status(project: PathBuf) -> anyhow::Result<()> {
    let project_root = project.canonicalize()?;
    let db_path = default_db_path(&project_root);

    if !db_path.exists() {
        println!("❌ No index found for project: {}", project_root.display());
        println!(
            "   Run `unity-refgraph index build {}` to create one.",
            project_root.display()
        );
        return Ok(());
    }

    let conn = db::open_db(&db_path)?;

    let file_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM files WHERE project_id = 1",
        [],
        |row| row.get(0),
    )?;

    let yaml_obj_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM yaml_objects", [], |row| row.get(0))?;
    let yaml_ref_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM yaml_references", [], |row| row.get(0))?;
    let cs_decl_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM cs_declarations", [], |row| row.get(0))?;
    let asset_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM assets WHERE project_id = 1",
        [],
        |row| row.get(0),
    )?;
    let entity_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))?;
    let edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM entity_edges", [], |row| row.get(0))?;
    let vfs_entry_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM vfs_entries WHERE project_id = 1",
        [],
        |row| row.get(0),
    )?;
    let vfs_edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM vfs_edges", [], |row| row.get(0))?;

    let summary: Option<(String, i64, String)> = conn.query_row(
        "SELECT mode, discovered_file_count, published_index_path FROM rebuild_summary WHERE project_id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).ok();

    println!("📊 Unity Insight Index Status");
    println!("   Project:  {}", project_root.display());
    println!("   Database: {}", db_path.display());
    if let Some((mode, discovered, path)) = summary {
        println!("   Mode:     {}", mode);
        println!("   Indexed:  {} files discovered", discovered);
        println!("   DB Path:  {}", path);
    }
    println!();
    println!("   Files:          {}", file_count);
    println!("   YAML Objects:   {}", yaml_obj_count);
    println!("   YAML Refs:      {}", yaml_ref_count);
    println!("   C# Decls:       {}", cs_decl_count);
    println!("   Assets:         {}", asset_count);
    println!("   Entities:       {}", entity_count);
    println!("   Entity Edges:   {}", edge_count);
    println!("   VFS Entries:    {}", vfs_entry_count);
    println!("   VFS Edges:      {}", vfs_edge_count);

    Ok(())
}
