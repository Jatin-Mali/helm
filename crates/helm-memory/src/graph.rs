//! Entity graph — SQLite-backed knowledge graph for HELM.
//!
//! Stores typed entities and weighted directed relations.  Used by the agent
//! to accumulate facts about the environment across episodes.

use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
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
            PRIMARY KEY (from_id, to_id, relation)
        );
        CREATE INDEX IF NOT EXISTS idx_relations_from ON relations(from_id);
        CREATE INDEX IF NOT EXISTS idx_relations_to   ON relations(to_id);",
    )
    .map_err(GraphError::Sqlite)
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
}
