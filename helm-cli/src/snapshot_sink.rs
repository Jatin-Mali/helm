//! AgentEventSink wrapper that captures a snapshot of every file `fs_write`
//! is about to modify (U5 — snapshots for undo/redo) and that auto-saves the
//! current session row on completion (U7).
//!
//! Snapshots are stored in the SessionStore; `helm undo` and `helm redo` read
//! them back and call `apply_snapshot` to restore content to disk.

#![allow(dead_code)]

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

use helm_agent::{AgentEvent, AgentEventSink};
use helm_memory::SessionStore;
use tokio::runtime::Handle;

pub struct SnapshotSink<S: AgentEventSink> {
    inner: S,
    store: Arc<SessionStore>,
    session_id: Arc<Mutex<Option<String>>>,
    working_dir: PathBuf,
    step: AtomicU32,
}

impl<S: AgentEventSink> SnapshotSink<S> {
    pub fn new(
        inner: S,
        store: Arc<SessionStore>,
        session_id: Option<String>,
        working_dir: PathBuf,
    ) -> Self {
        Self {
            inner,
            store,
            session_id: Arc::new(Mutex::new(session_id)),
            working_dir,
            step: AtomicU32::new(0),
        }
    }

    pub fn set_session(&self, session_id: String) {
        if let Ok(mut guard) = self.session_id.lock() {
            *guard = Some(session_id);
        }
    }

    fn current_session(&self) -> Option<String> {
        self.session_id.lock().ok().and_then(|g| g.clone())
    }

    fn resolve(&self, raw: &str) -> PathBuf {
        let p = PathBuf::from(raw);
        if p.is_absolute() {
            p
        } else {
            self.working_dir.join(p)
        }
    }
}

impl<S: AgentEventSink> AgentEventSink for SnapshotSink<S> {
    fn emit(&self, event: AgentEvent) {
        if let AgentEvent::ToolCallParsed { name, input, .. } = &event
            && name == "fs_write"
            && let Some(session_id) = self.current_session()
            && let Some(path_str) = input.get("path").and_then(|v| v.as_str())
        {
            let resolved = self.resolve(path_str);
            if let Ok(content) = std::fs::read_to_string(&resolved) {
                let step = self.step.fetch_add(1, Ordering::SeqCst);
                let store = Arc::clone(&self.store);
                #[allow(clippy::redundant_locals)]
                let session_id = session_id;
                let path_for_record = resolved.clone();
                if let Ok(handle) = Handle::try_current() {
                    handle.spawn(async move {
                        if let Err(error) = store
                            .take_snapshot(&session_id, step, &content, &path_for_record)
                            .await
                        {
                            tracing::warn!(target: "helm::snapshot", "take_snapshot failed: {error}");
                        }
                    });
                }
            }
        }
        self.inner.emit(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;
    use tempfile::tempdir;

    struct CountSink(StdMutex<u32>);
    impl AgentEventSink for CountSink {
        fn emit(&self, _event: AgentEvent) {
            *self.0.lock().unwrap() += 1;
        }
    }

    #[tokio::test]
    async fn snapshots_taken_before_fs_write() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("helm.db");
        let snaps = dir.path().join("snaps");
        let store = Arc::new(SessionStore::open(&db, snaps).await.unwrap());
        let target = dir.path().join("hello.txt");
        std::fs::write(&target, "original").unwrap();
        let session_id = store
            .create_session("t", "g", "ep-1".into(), None, None, None)
            .await
            .unwrap();
        let sink = SnapshotSink::new(
            CountSink(StdMutex::new(0)),
            Arc::clone(&store),
            Some(session_id.clone()),
            dir.path().to_path_buf(),
        );
        sink.emit(AgentEvent::ToolCallParsed {
            id: "1".into(),
            name: "fs_write".into(),
            input: json!({"path": "hello.txt", "content": "new"}),
        });
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let snaps = store.list_snapshots(&session_id).await.unwrap();
            if !snaps.is_empty() {
                assert_eq!(snaps.len(), 1);
                return;
            }
        }
        panic!("snapshot never recorded");
    }
}
