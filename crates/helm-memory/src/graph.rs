//! Entity graph — SQLite-backed knowledge graph for HELM.
//!
//! Stores typed entities and weighted directed relations.  Used by the agent
//! to accumulate facts about the environment across episodes.

use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use thiserror::Error;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum GraphError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("lock poisoned")]
    Lock,
    #[error("entity not found: {0}")]
    NotFound(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

// ── Domain types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Entity {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub attributes: Value,
}

#[derive(Debug, Clone)]
pub struct Relation {
    pub from_id: String,
    pub to_id: String,
    pub relation: String,
    pub weight: f64,
}

// ── EntityGraph ───────────────────────────────────────────────────────────────

pub struct EntityGraph {
    conn: Arc<Mutex<Connection>>,
}

impl EntityGraph {
    pub fn open(path: &Path) -> Result<Self, GraphError> {
        let conn = Connection::open(path)?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_in_memory() -> Result<Self, GraphError> {
        let conn = Connection::open_in_memory()?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Insert or replace an entity (upsert by id).
    pub fn upsert_entity(&self, entity: &Entity) -> Result<(), GraphError> {
        let conn = lock(&self.conn)?;
        conn.execute(
            "INSERT INTO entities (id, kind, name, attributes) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, name=excluded.name, attributes=excluded.attributes",
            params![
                entity.id,
                entity.kind,
                entity.name,
                serde_json::to_string(&entity.attributes)?
            ],
        )?;
        Ok(())
    }

    pub fn get_entity(&self, id: &str) -> Result<Option<Entity>, GraphError> {
        let conn = lock(&self.conn)?;
        conn.query_row(
            "SELECT id, kind, name, attributes FROM entities WHERE id = ?1",
            params![id],
            row_to_entity,
        )
        .optional()
        .map_err(GraphError::Sqlite)
    }

    /// Full-text search by name (case-insensitive substring match).
    pub fn search_by_name(&self, query: &str, limit: u32) -> Result<Vec<Entity>, GraphError> {
        let conn = lock(&self.conn)?;
        let pattern = format!("%{query}%");
        let mut stmt = conn.prepare(
            "SELECT id, kind, name, attributes FROM entities
             WHERE name LIKE ?1 ORDER BY name LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit], row_to_entity)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(GraphError::Sqlite)
    }

    /// Add or update a directed relation between two entities.
    pub fn add_relation(&self, rel: &Relation) -> Result<(), GraphError> {
        let conn = lock(&self.conn)?;
        conn.execute(
            "INSERT INTO relations (from_id, to_id, relation, weight) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(from_id, to_id, relation) DO UPDATE SET weight=excluded.weight",
            params![rel.from_id, rel.to_id, rel.relation, rel.weight],
        )?;
        Ok(())
    }

    /// Returns all entities directly reachable from `entity_id` via any relation.
    pub fn neighbors(&self, entity_id: &str) -> Result<Vec<(Relation, Entity)>, GraphError> {
        let conn = lock(&self.conn)?;
        let mut stmt = conn.prepare(
            "SELECT r.from_id, r.to_id, r.relation, r.weight,
                    e.id, e.kind, e.name, e.attributes
             FROM relations r JOIN entities e ON r.to_id = e.id
             WHERE r.from_id = ?1
             ORDER BY r.weight DESC",
        )?;
        let rows = stmt.query_map(params![entity_id], |row| {
            let rel = Relation {
                from_id: row.get(0)?,
                to_id: row.get(1)?,
                relation: row.get(2)?,
                weight: row.get(3)?,
            };
            let attrs_str: String = row.get(7)?;
            let attrs: Value =
                serde_json::from_str(&attrs_str).unwrap_or(Value::Object(serde_json::Map::new()));
            let entity = Entity {
                id: row.get(4)?,
                kind: row.get(5)?,
                name: row.get(6)?,
                attributes: attrs,
            };
            Ok((rel, entity))
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(GraphError::Sqlite)
    }

    pub fn entity_count(&self) -> Result<u64, GraphError> {
        let conn = lock(&self.conn)?;
        conn.query_row("SELECT COUNT(*) FROM entities", [], |row| {
            row.get::<_, i64>(0)
        })
        .map(|n| n as u64)
        .map_err(GraphError::Sqlite)
    }

    /// Find entities by optional type filter and optional name pattern (SQL LIKE).
    pub fn find_entities(
        &self,
        entity_type: Option<&str>,
        name_pattern: Option<&str>,
    ) -> Result<Vec<Entity>, GraphError> {
        let conn = lock(&self.conn)?;
        match (entity_type, name_pattern) {
            (Some(et), Some(np)) => {
                let pattern = format!("%{np}%");
                let mut stmt = conn.prepare(
                    "SELECT id, kind, name, attributes FROM entities
                     WHERE kind = ?1 AND name LIKE ?2 ORDER BY name",
                )?;
                let rows = stmt.query_map(params![et, pattern], row_to_entity)?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(GraphError::Sqlite)
            }
            (Some(et), None) => {
                let mut stmt = conn.prepare(
                    "SELECT id, kind, name, attributes FROM entities
                     WHERE kind = ?1 ORDER BY name",
                )?;
                let rows = stmt.query_map(params![et], row_to_entity)?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(GraphError::Sqlite)
            }
            (None, Some(np)) => {
                let pattern = format!("%{np}%");
                let mut stmt = conn.prepare(
                    "SELECT id, kind, name, attributes FROM entities
                     WHERE name LIKE ?1 ORDER BY name",
                )?;
                let rows = stmt.query_map(params![pattern], row_to_entity)?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(GraphError::Sqlite)
            }
            (None, None) => {
                let mut stmt = conn.prepare(
                    "SELECT id, kind, name, attributes FROM entities
                     ORDER BY name",
                )?;
                let rows = stmt.query_map([], row_to_entity)?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(GraphError::Sqlite)
            }
        }
    }

    /// Find relations from a source entity, optionally filtered by relation type.
    pub fn find_relations(
        &self,
        source_id: &str,
        relation_type: Option<&str>,
    ) -> Result<Vec<Relation>, GraphError> {
        let conn = lock(&self.conn)?;
        if let Some(rt) = relation_type {
            let mut stmt = conn.prepare(
                "SELECT from_id, to_id, relation, weight FROM relations
                 WHERE from_id = ?1 AND relation = ?2 ORDER BY weight DESC",
            )?;
            let rows = stmt.query_map(params![source_id, rt], |row| {
                Ok(Relation {
                    from_id: row.get(0)?,
                    to_id: row.get(1)?,
                    relation: row.get(2)?,
                    weight: row.get(3)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(GraphError::Sqlite)
        } else {
            let mut stmt = conn.prepare(
                "SELECT from_id, to_id, relation, weight FROM relations
                 WHERE from_id = ?1 ORDER BY weight DESC",
            )?;
            let rows = stmt.query_map(params![source_id], |row| {
                Ok(Relation {
                    from_id: row.get(0)?,
                    to_id: row.get(1)?,
                    relation: row.get(2)?,
                    weight: row.get(3)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(GraphError::Sqlite)
        }
    }

    /// Remove relations older than `age_days` OR with confidence below `min_confidence`.
    /// Returns count of pruned relations.
    pub fn prune_stale_relations(
        &self,
        age_days: u32,
        min_confidence: f32,
    ) -> Result<u32, GraphError> {
        let conn = lock(&self.conn)?;
        // Calculate cutoff date
        let cutoff = format!("datetime('now', '-{} days')", age_days);
        conn.execute(
            &format!(
                "DELETE FROM relations
                 WHERE weight < ?1 OR created_at < {}",
                cutoff
            ),
            params![min_confidence as f64],
        )
        .map(|n| n as u32)
        .map_err(GraphError::Sqlite)
    }

    /// Store a text embedding (Vec<f32>) for an entity as a BLOB.
    pub fn store_embedding(&self, entity_id: &str, embedding: &[f32]) -> Result<(), GraphError> {
        let conn = lock(&self.conn)?;
        // Convert f32 vec to bytes (little-endian)
        let bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
        conn.execute(
            "INSERT INTO entity_embeddings (entity_id, embedding) VALUES (?1, ?2)
             ON CONFLICT(entity_id) DO UPDATE SET embedding=excluded.embedding",
            params![entity_id, bytes],
        )?;
        Ok(())
    }

    /// Compute cosine similarity across all stored embeddings. Returns top_k most similar.
    pub fn semantic_search(
        &self,
        query_embedding: &[f32],
        top_k: u32,
    ) -> Result<Vec<(Entity, f32)>, GraphError> {
        let conn = lock(&self.conn)?;
        let mut stmt = conn.prepare(
            "SELECT e.id, e.kind, e.name, e.attributes, ee.embedding
             FROM entity_embeddings ee
             JOIN entities e ON ee.entity_id = e.id",
        )?;

        let rows = stmt.query_map([], |row| {
            let attrs_str: String = row.get(3)?;
            let attributes: Value =
                serde_json::from_str(&attrs_str).unwrap_or(Value::Object(serde_json::Map::new()));
            let entity = Entity {
                id: row.get(0)?,
                kind: row.get(1)?,
                name: row.get(2)?,
                attributes,
            };
            let embedding_bytes: Vec<u8> = row.get(4)?;
            Ok((entity, embedding_bytes))
        })?;

        let mut results: Vec<(Entity, f32)> = rows
            .filter_map(Result::ok)
            .map(|(entity, bytes)| {
                let embedding: Vec<f32> = bytes
                    .chunks(4)
                    .filter_map(|chunk| {
                        if chunk.len() == 4 {
                            let array: [u8; 4] = [chunk[0], chunk[1], chunk[2], chunk[3]];
                            Some(f32::from_le_bytes(array))
                        } else {
                            None
                        }
                    })
                    .collect();
                let similarity = cosine_similarity(query_embedding, &embedding);
                (entity, similarity)
            })
            .collect();

        // Sort by similarity descending
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(top_k as usize);
        Ok(results)
    }

    /// Export all entities and relations as a JSON string.
    pub fn export_json(&self) -> Result<String, GraphError> {
        let conn = lock(&self.conn)?;
        let mut stmt_entities =
            conn.prepare("SELECT id, kind, name, attributes FROM entities ORDER BY id")?;
        let entities: Vec<Value> = stmt_entities
            .query_map([], |row| {
                let attrs_str: String = row.get(3)?;
                let attributes: Value = serde_json::from_str(&attrs_str)
                    .unwrap_or(Value::Object(serde_json::Map::new()));
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "kind": row.get::<_, String>(1)?,
                    "name": row.get::<_, String>(2)?,
                    "attributes": attributes
                }))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(GraphError::Sqlite)?;

        let mut stmt_relations = conn.prepare(
            "SELECT from_id, to_id, relation, weight FROM relations ORDER BY from_id, to_id",
        )?;
        let relations: Vec<Value> = stmt_relations
            .query_map([], |row| {
                Ok(json!({
                    "from_id": row.get::<_, String>(0)?,
                    "to_id": row.get::<_, String>(1)?,
                    "relation": row.get::<_, String>(2)?,
                    "weight": row.get::<_, f64>(3)?
                }))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(GraphError::Sqlite)?;

        let export = json!({
            "entities": entities,
            "relations": relations
        });
        serde_json::to_string(&export).map_err(GraphError::Json)
    }

    /// Import entities and relations from JSON string.
    /// Skip duplicates by entity id.
    pub fn import_json(&self, json: &str) -> Result<(u32, u32), GraphError> {
        let export: Value = serde_json::from_str(json).map_err(GraphError::Json)?;
        let conn = lock(&self.conn)?;

        let mut entities_added = 0u32;
        let mut relations_added = 0u32;

        // Import entities
        if let Some(entities) = export.get("entities").and_then(Value::as_array) {
            for entity_val in entities {
                let id = entity_val
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let kind = entity_val
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let name = entity_val
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let attributes = entity_val.get("attributes").cloned().unwrap_or(Value::Null);

                // Check if exists
                let exists: bool = conn
                    .query_row("SELECT 1 FROM entities WHERE id = ?1", params![id], |_| {
                        Ok(true)
                    })
                    .optional()
                    .unwrap_or(Some(false))
                    .unwrap_or(false);

                if !exists {
                    conn.execute(
                        "INSERT INTO entities (id, kind, name, attributes) VALUES (?1, ?2, ?3, ?4)",
                        params![
                            id,
                            kind,
                            name,
                            serde_json::to_string(&attributes).unwrap_or_else(|_| "{}".to_owned())
                        ],
                    )?;
                    entities_added += 1;
                }
            }
        }

        // Import relations
        if let Some(relations) = export.get("relations").and_then(Value::as_array) {
            for rel_val in relations {
                let from_id = rel_val
                    .get("from_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let to_id = rel_val
                    .get("to_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let relation = rel_val
                    .get("relation")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let weight = rel_val.get("weight").and_then(Value::as_f64).unwrap_or(1.0);

                // Check if exists
                let exists: bool = conn
                    .query_row(
                        "SELECT 1 FROM relations WHERE from_id = ?1 AND to_id = ?2 AND relation = ?3",
                        params![from_id, to_id, relation],
                        |_| Ok(true),
                    )
                    .optional()
                    .unwrap_or(Some(false))
                    .unwrap_or(false);

                if !exists {
                    conn.execute(
                        "INSERT INTO relations (from_id, to_id, relation, weight) VALUES (?1, ?2, ?3, ?4)",
                        params![from_id, to_id, relation, weight],
                    )?;
                    relations_added += 1;
                }
            }
        }

        Ok((entities_added, relations_added))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn row_to_entity(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entity> {
    let attrs_str: String = row.get(3)?;
    let attributes: Value =
        serde_json::from_str(&attrs_str).unwrap_or(Value::Object(serde_json::Map::new()));
    Ok(Entity {
        id: row.get(0)?,
        kind: row.get(1)?,
        name: row.get(2)?,
        attributes,
    })
}

fn lock(conn: &Arc<Mutex<Connection>>) -> Result<MutexGuard<'_, Connection>, GraphError> {
    conn.lock().map_err(|_| GraphError::Lock)
}

fn run_migrations(conn: &Connection) -> Result<(), GraphError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS entities (
            id          TEXT PRIMARY KEY,
            kind        TEXT NOT NULL,
            name        TEXT NOT NULL,
            attributes  TEXT NOT NULL DEFAULT '{}',
            created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
        );
        CREATE INDEX IF NOT EXISTS idx_entities_name ON entities(name);
        CREATE INDEX IF NOT EXISTS idx_entities_kind ON entities(kind);

        CREATE TABLE IF NOT EXISTS relations (
            from_id   TEXT NOT NULL,
            to_id     TEXT NOT NULL,
            relation  TEXT NOT NULL,
            weight    REAL NOT NULL DEFAULT 1.0,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
            PRIMARY KEY (from_id, to_id, relation)
        );
        CREATE INDEX IF NOT EXISTS idx_relations_from ON relations(from_id);
        CREATE INDEX IF NOT EXISTS idx_relations_to   ON relations(to_id);

        CREATE TABLE IF NOT EXISTS entity_embeddings (
            entity_id TEXT PRIMARY KEY,
            embedding BLOB NOT NULL,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
        );",
    )
    .map_err(GraphError::Sqlite)
}

/// Compute cosine similarity between two embedding vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let min_len = a.len().min(b.len());
    let dot_product: f32 = a[..min_len]
        .iter()
        .zip(b[..min_len].iter())
        .map(|(x, y)| x * y)
        .sum();
    let norm_a: f32 = a[..min_len].iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b[..min_len].iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot_product / (norm_a * norm_b)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Entity, EntityGraph, Relation};

    fn graph() -> EntityGraph {
        EntityGraph::open_in_memory().unwrap()
    }

    fn entity(id: &str, kind: &str, name: &str) -> Entity {
        Entity {
            id: id.to_owned(),
            kind: kind.to_owned(),
            name: name.to_owned(),
            attributes: json!({}),
        }
    }

    #[test]
    fn upsert_and_get_happy_path() {
        let g = graph();
        let e = Entity {
            id: "host:web01".to_owned(),
            kind: "host".to_owned(),
            name: "web01".to_owned(),
            attributes: json!({"ip": "10.0.0.1"}),
        };
        g.upsert_entity(&e).unwrap();
        let got = g.get_entity("host:web01").unwrap().unwrap();
        assert_eq!(got.name, "web01");
        assert_eq!(got.attributes["ip"], "10.0.0.1");
    }

    #[test]
    fn upsert_updates_existing_happy_path() {
        let g = graph();
        g.upsert_entity(&entity("a", "file", "foo.txt")).unwrap();
        let updated = Entity {
            id: "a".to_owned(),
            kind: "file".to_owned(),
            name: "bar.txt".to_owned(),
            attributes: json!({}),
        };
        g.upsert_entity(&updated).unwrap();
        let got = g.get_entity("a").unwrap().unwrap();
        assert_eq!(got.name, "bar.txt");
        assert_eq!(g.entity_count().unwrap(), 1);
    }

    #[test]
    fn get_missing_returns_none_edge_case() {
        let g = graph();
        assert!(g.get_entity("nope").unwrap().is_none());
    }

    #[test]
    fn search_by_name_happy_path() {
        let g = graph();
        g.upsert_entity(&entity("1", "host", "alpha-01")).unwrap();
        g.upsert_entity(&entity("2", "host", "beta-02")).unwrap();
        g.upsert_entity(&entity("3", "host", "alpha-02")).unwrap();

        let results = g.search_by_name("alpha", 10).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|e| e.name.contains("alpha")));
    }

    #[test]
    fn add_relation_and_neighbors_happy_path() {
        let g = graph();
        g.upsert_entity(&entity("a", "svc", "frontend")).unwrap();
        g.upsert_entity(&entity("b", "svc", "backend")).unwrap();
        g.add_relation(&Relation {
            from_id: "a".to_owned(),
            to_id: "b".to_owned(),
            relation: "calls".to_owned(),
            weight: 1.0,
        })
        .unwrap();

        let neighbors = g.neighbors("a").unwrap();
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].0.relation, "calls");
        assert_eq!(neighbors[0].1.name, "backend");
    }

    #[test]
    fn relation_upsert_updates_weight_edge_case() {
        let g = graph();
        g.upsert_entity(&entity("x", "t", "x")).unwrap();
        g.upsert_entity(&entity("y", "t", "y")).unwrap();
        g.add_relation(&Relation {
            from_id: "x".to_owned(),
            to_id: "y".to_owned(),
            relation: "r".to_owned(),
            weight: 1.0,
        })
        .unwrap();
        g.add_relation(&Relation {
            from_id: "x".to_owned(),
            to_id: "y".to_owned(),
            relation: "r".to_owned(),
            weight: 5.0,
        })
        .unwrap();
        let neighbors = g.neighbors("x").unwrap();
        assert_eq!(neighbors[0].0.weight, 5.0);
    }

    #[test]
    fn no_neighbors_on_empty_graph_edge_case() {
        let g = graph();
        g.upsert_entity(&entity("lone", "t", "lone")).unwrap();
        assert!(g.neighbors("lone").unwrap().is_empty());
    }

    #[test]
    fn find_entities_by_type() {
        let g = graph();
        g.upsert_entity(&entity("h1", "host", "server1")).unwrap();
        g.upsert_entity(&entity("h2", "host", "server2")).unwrap();
        g.upsert_entity(&entity("s1", "svc", "nginx")).unwrap();

        let hosts = g.find_entities(Some("host"), None).unwrap();
        assert_eq!(hosts.len(), 2);
        assert!(hosts.iter().all(|e| e.kind == "host"));
    }

    #[test]
    fn find_entities_by_name_pattern() {
        let g = graph();
        g.upsert_entity(&entity("h1", "host", "webserver-01"))
            .unwrap();
        g.upsert_entity(&entity("h2", "host", "webserver-02"))
            .unwrap();
        g.upsert_entity(&entity("h3", "host", "dbserver-01"))
            .unwrap();

        let results = g.find_entities(None, Some("web")).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|e| e.name.contains("web")));
    }

    #[test]
    fn find_relations_from_source() {
        let g = graph();
        g.upsert_entity(&entity("a", "svc", "frontend")).unwrap();
        g.upsert_entity(&entity("b", "svc", "backend")).unwrap();
        g.upsert_entity(&entity("c", "svc", "db")).unwrap();
        g.add_relation(&Relation {
            from_id: "a".to_owned(),
            to_id: "b".to_owned(),
            relation: "calls".to_owned(),
            weight: 1.0,
        })
        .unwrap();
        g.add_relation(&Relation {
            from_id: "a".to_owned(),
            to_id: "c".to_owned(),
            relation: "depends_on".to_owned(),
            weight: 0.8,
        })
        .unwrap();

        let rels = g.find_relations("a", None).unwrap();
        assert_eq!(rels.len(), 2);
        assert!(rels.iter().all(|r| r.from_id == "a"));
    }

    #[test]
    fn find_relations_filtered_by_type() {
        let g = graph();
        g.upsert_entity(&entity("a", "svc", "frontend")).unwrap();
        g.upsert_entity(&entity("b", "svc", "backend")).unwrap();
        g.upsert_entity(&entity("c", "svc", "db")).unwrap();
        g.add_relation(&Relation {
            from_id: "a".to_owned(),
            to_id: "b".to_owned(),
            relation: "calls".to_owned(),
            weight: 1.0,
        })
        .unwrap();
        g.add_relation(&Relation {
            from_id: "a".to_owned(),
            to_id: "c".to_owned(),
            relation: "depends_on".to_owned(),
            weight: 0.8,
        })
        .unwrap();

        let calls = g.find_relations("a", Some("calls")).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].relation, "calls");
    }

    #[test]
    fn store_and_search_embeddings() {
        let g = graph();
        g.upsert_entity(&entity("e1", "doc", "doc1")).unwrap();
        g.upsert_entity(&entity("e2", "doc", "doc2")).unwrap();

        let emb1 = vec![1.0, 0.0, 0.0];
        let emb2 = vec![0.9, 0.1, 0.0];
        g.store_embedding("e1", &emb1).unwrap();
        g.store_embedding("e2", &emb2).unwrap();

        let query = vec![1.0, 0.0, 0.0];
        let results = g.semantic_search(&query, 10).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0.id, "e1"); // e1 should be most similar
    }

    #[test]
    fn export_import_roundtrip() {
        let g = graph();
        g.upsert_entity(&entity("e1", "host", "server1")).unwrap();
        g.upsert_entity(&entity("e2", "host", "server2")).unwrap();
        g.add_relation(&Relation {
            from_id: "e1".to_owned(),
            to_id: "e2".to_owned(),
            relation: "peers".to_owned(),
            weight: 1.0,
        })
        .unwrap();

        let json = g.export_json().unwrap();
        let g2 = graph();
        let (ents, rels) = g2.import_json(&json).unwrap();
        assert_eq!(ents, 2);
        assert_eq!(rels, 1);

        let imported = g2.get_entity("e1").unwrap().unwrap();
        assert_eq!(imported.name, "server1");
    }

    #[test]
    fn prune_stale_relations() {
        let g = graph();
        g.upsert_entity(&entity("a", "t", "a")).unwrap();
        g.upsert_entity(&entity("b", "t", "b")).unwrap();
        // Add two relations with different weights
        g.add_relation(&Relation {
            from_id: "a".to_owned(),
            to_id: "b".to_owned(),
            relation: "weak".to_owned(),
            weight: 0.05,
        })
        .unwrap();
        g.add_relation(&Relation {
            from_id: "a".to_owned(),
            to_id: "b".to_owned(),
            relation: "strong".to_owned(),
            weight: 0.9,
        })
        .unwrap();

        // Prune relations with weight < 0.1
        let pruned = g.prune_stale_relations(0, 0.1).unwrap();
        assert!(pruned > 0);

        let rels = g.find_relations("a", None).unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].relation, "strong");
    }
}
