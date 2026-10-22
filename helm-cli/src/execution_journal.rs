//! Durable execution evidence and authenticated, encrypted recovery material.

use std::{
    fs::{File, OpenOptions},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
};

use anyhow::{Context, Result, anyhow};
use helm_memory::{ChangeSetStore, SnapshotStore};
use helm_monitor::{
    ChangeSet, ChangeSetStep, PreChangeBackup, SystemSnapshot, execute::ExecutionJournal,
};
use nix::fcntl::{Flock, FlockArg};
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use rusqlite::{Connection, params};

use crate::secrets::SecretsStore;

const BACKUP_KEY: &str = "HELM_BACKUP_KEY";
const PREFIX: &str = "helm-backup-v1:";

pub struct DurableJournal {
    connection: Mutex<Connection>,
    secrets: SecretsStore,
    db_path: PathBuf,
    _lock: Flock<File>,
}

pub fn lock_database(db: &Path) -> Result<Flock<File>> {
    let path = db.with_extension("execution.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Flock::lock(file, FlockArg::LockExclusiveNonblock)
        .map_err(|_| anyhow!("another change is active for this database; retry after it finishes"))
}

fn connection(db: &Path) -> Result<Connection> {
    let connection = Connection::open(db)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
    Ok(connection)
}

fn backup_key(secrets: &SecretsStore, create: bool) -> Result<aead::LessSafeKey> {
    let secret = if create {
        let mut candidate = [0u8; 32];
        SystemRandom::new()
            .fill(&mut candidate)
            .map_err(|_| anyhow!("cannot generate recovery key"))?;
        secrets
            .get_or_insert(BACKUP_KEY, helm_core::Secret::new(hex::encode(candidate)))
            .map_err(|_| {
                anyhow!(
                    "cannot initialize recovery key; check secrets store permissions and validity"
                )
            })?
    } else {
        secrets
            .get(BACKUP_KEY)
            .map_err(|_| anyhow!("cannot read recovery key"))?
            .ok_or_else(|| anyhow!("recovery key is missing; no files were restored"))?
    };
    let bytes = hex::decode(secret.expose()).map_err(|_| anyhow!("invalid recovery key"))?;
    let key = aead::UnboundKey::new(&aead::AES_256_GCM, &bytes)
        .map_err(|_| anyhow!("invalid recovery key length"))?;
    Ok(aead::LessSafeKey::new(key))
}

fn associated_data(
    change: &str,
    step: &str,
    id: &str,
    path: &str,
    checksum: &str,
) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&(change, step, id, path, checksum))?)
}

fn seal(secrets: &SecretsStore, body: &str, associated: &[u8]) -> Result<String> {
    let mut nonce = [0u8; 12];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| anyhow!("cannot generate backup nonce"))?;
    let key = backup_key(secrets, true)?;
    let mut ciphertext = body.as_bytes().to_vec();
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(associated),
        &mut ciphertext,
    )
    .map_err(|_| anyhow!("cannot encrypt recovery material"))?;
    Ok(format!(
        "{PREFIX}{}:{}",
        hex::encode(nonce),
        hex::encode(ciphertext)
    ))
}

pub fn open_backup(
    secrets: &SecretsStore,
    backup: &helm_memory::ChangeSetBackupRecord,
) -> Result<String> {
    let encoded = backup.backup_content.strip_prefix(PREFIX).ok_or_else(|| {
        anyhow!("legacy backup has no verified recovery format; refusing restoration")
    })?;
    let (nonce, ciphertext) = encoded
        .split_once(':')
        .ok_or_else(|| anyhow!("invalid backup envelope"))?;
    let nonce: [u8; 12] = hex::decode(nonce)?
        .try_into()
        .map_err(|_| anyhow!("invalid nonce"))?;
    let mut ciphertext = hex::decode(ciphertext)?;
    let associated = associated_data(
        &backup.change_set_id,
        &backup.step_id,
        &backup.id,
        &backup.file_path,
        &backup.checksum_before,
    )?;
    let key = backup_key(secrets, false)?;
    let plaintext = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(&associated),
            &mut ciphertext,
        )
        .map_err(|_| anyhow!("backup authentication failed; refusing restoration"))?;
    String::from_utf8(plaintext.to_vec()).context("invalid recovery body")
}

pub fn has_verified_backup(backup: &helm_memory::ChangeSetBackupRecord) -> bool {
    backup.backup_content.starts_with(PREFIX)
}

impl DurableJournal {
    pub fn new(db: &Path, secrets: SecretsStore) -> Result<Self> {
        let lock = lock_database(db)?;
        let connection = connection(db)?;
        std::fs::set_permissions(db, std::fs::Permissions::from_mode(0o600))?;
        // The lock excludes another active executor. Never silently replay interrupted work.
        connection.execute_batch("BEGIN IMMEDIATE;
            UPDATE change_sets SET status='interrupted', summary=summary || '\nExecution interrupted; inspect and recover before retrying.' WHERE status IN ('pending','approved','running','rolling_back');
            UPDATE change_set_steps SET status='interrupted' WHERE status='running';
            COMMIT;")?;
        Ok(Self {
            connection: Mutex::new(connection),
            secrets,
            db_path: db.canonicalize()?,
            _lock: lock,
        })
    }

    fn persist_start(&self, change: &ChangeSet, before: &SystemSnapshot) -> Result<()> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("journal lock poisoned"))?;
        let tx = connection.transaction()?;
        SnapshotStore::insert(
            &tx,
            &helm_core::redact_secrets(&serde_json::to_string(before)?),
            "[]",
        )?;
        ChangeSetStore::insert(
            &tx,
            &change.id,
            &change.plan_id,
            &helm_core::redact_secrets(&change.plan_title),
            &change.snapshot_id,
            &change.before_snapshot_id,
            "running",
            change.created_at,
            "",
        )?;
        for step in &change.steps {
            let command_text = step
                .command
                .command_text
                .as_deref()
                .map(helm_core::redact_secrets);
            ChangeSetStore::insert_step(
                &tx,
                &step.id,
                &change.id,
                &helm_core::redact_secrets(&step.plan_step_title),
                &step.command.tool,
                &helm_core::redact_secrets(&serde_json::to_string(&step.command.input)?),
                command_text.as_deref(),
                &helm_core::redact_secrets(&step.command.expected_effect),
                step.command.risk.as_str(),
                "pending",
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn persist_preparation(
        &self,
        change: &str,
        step: &ChangeSetStep,
        backup: Option<&PreChangeBackup>,
    ) -> Result<()> {
        let sealed = if let Some(backup) = backup {
            let secret_path = self.secrets.path();
            let mut protected = vec![
                self.db_path.clone(),
                self.db_path.with_extension("execution.lock"),
                PathBuf::from(format!("{}-wal", self.db_path.display())),
                PathBuf::from(format!("{}-shm", self.db_path.display())),
                secret_path.to_path_buf(),
            ];
            if let Some(parent) = secret_path.parent() {
                protected.push(parent.join(".secrets.toml.lock"));
            }
            if protected.iter().any(|path| {
                let resolved = path.canonicalize().or_else(|_| {
                    Ok::<_, std::io::Error>(
                        path.parent()
                            .unwrap_or(Path::new("."))
                            .canonicalize()?
                            .join(path.file_name().unwrap_or_default()),
                    )
                });
                resolved.is_ok_and(|path| path == Path::new(&backup.file_path))
            }) {
                return Err(anyhow!("cannot overwrite active HELM recovery state"));
            }
            let associated = associated_data(
                change,
                &step.id,
                &backup.id,
                &backup.file_path,
                &backup.checksum_before,
            )?;
            Some(seal(&self.secrets, &backup.backup_content, &associated)?)
        } else {
            None
        };
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("journal lock poisoned"))?;
        let tx = connection.transaction()?;
        if let (Some(backup), Some(sealed)) = (backup, sealed) {
            tx.execute("INSERT INTO change_set_backups (id,change_set_id,step_id,file_path,checksum_before,backup_content,restored) VALUES (?1,?2,?3,?4,?5,?6,0)",
                params![backup.id, change, step.id, backup.file_path, backup.checksum_before, sealed])?;
        }
        tx.execute(
            "UPDATE change_set_steps SET status='running',started_at=?2 WHERE id=?1",
            params![step.id, step.started_at],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn persist_finish(&self, change: &ChangeSet, after: &SystemSnapshot) -> Result<()> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("journal lock poisoned"))?;
        let tx = connection.transaction()?;
        SnapshotStore::insert(
            &tx,
            &helm_core::redact_secrets(&serde_json::to_string(after)?),
            "[]",
        )?;
        for step in &change.steps {
            persist_step(&tx, step)?;
        }
        tx.execute("UPDATE change_sets SET status=?2,after_snapshot_id=?3,completed_at=?4,summary=?5 WHERE id=?1",
            params![change.id, change.status.to_string(), change.after_snapshot_id, change.completed_at, helm_core::redact_secrets(&change.summary)])?;
        tx.commit()?;
        Ok(())
    }
}

fn persist_step(connection: &Connection, step: &ChangeSetStep) -> Result<()> {
    ChangeSetStore::update_step_outcome(
        connection,
        &step.id,
        &step.status.to_string(),
        &helm_core::redact_secrets(&step.output_text),
        &helm_core::redact_secrets(&step.error_text),
        &helm_core::redact_secrets(&step.verification_result),
    )?;
    Ok(())
}

impl ExecutionJournal for DurableJournal {
    fn start(&self, change: &ChangeSet, before: &SystemSnapshot) -> Result<(), String> {
        self.persist_start(change, before)
            .map_err(|e| helm_core::redact_secrets(&e.to_string()))
    }
    fn prepare(
        &self,
        change: &str,
        step: &ChangeSetStep,
        backup: Option<&PreChangeBackup>,
    ) -> Result<(), String> {
        self.persist_preparation(change, step, backup)
            .map_err(|e| helm_core::redact_secrets(&e.to_string()))
    }
    fn record_step(&self, step: &ChangeSetStep) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "journal lock poisoned".to_owned())?;
        persist_step(&connection, step).map_err(|e| helm_core::redact_secrets(&e.to_string()))
    }
    fn finish(&self, change: &ChangeSet, after: &SystemSnapshot) -> Result<(), String> {
        self.persist_finish(change, after)
            .map_err(|e| helm_core::redact_secrets(&e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helm_monitor::{
        ChangeSetStatus, CommandPreview, ExecutionEngine, PlanSource, PlanStep, TroubleshootingPlan,
    };

    async fn fixture() -> (tempfile::TempDir, PathBuf, SecretsStore) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("helm.db");
        helm_memory::MemoryStore::open(&db).await.unwrap();
        let secrets = SecretsStore::open_at(dir.path().join("secrets.toml")).unwrap();
        (dir, db, secrets)
    }

    fn write_step(path: &Path, content: &str) -> PlanStep {
        let mut command = CommandPreview::new("fs_write", "write fixture", "isolated change");
        command.input = serde_json::json!({"path": path, "content": content});
        PlanStep {
            title: "file change".into(),
            command,
            hypothesis_id: None,
            expected_output: None,
            interpretation_guide: None,
        }
    }

    async fn apply(db: &Path, secrets: &SecretsStore, steps: Vec<PlanStep>) -> ChangeSet {
        let mut plan = TroubleshootingPlan::new(
            PlanSource::UserQuestion("recovery fixture".into()),
            "fixture".into(),
        );
        plan.proposed_fix_steps = steps;
        let journal = std::sync::Arc::new(DurableJournal::new(db, secrets.clone()).unwrap());
        let mut engine = ExecutionEngine::new(Box::new(|_| true)).with_journal(journal);
        engine.execute(&plan).await
    }

    #[tokio::test]
    async fn recovery_restart_restores_binary_permissions_and_new_file_existence() {
        let (dir, db, secrets) = fixture().await;
        let original = dir.path().join("original.bin");
        let created = dir.path().join("created.txt");
        let bytes = b"\0\xffprivate-original-data\n";
        std::fs::write(&original, bytes).unwrap();
        std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o640)).unwrap();
        let result = apply(
            &db,
            &secrets,
            vec![
                write_step(&original, "changed"),
                write_step(&created, "created"),
            ],
        )
        .await;
        assert_eq!(result.status, ChangeSetStatus::Completed);
        let conn = Connection::open(&db).unwrap();
        let stored = ChangeSetStore::get_full(&conn, &result.id)
            .unwrap()
            .unwrap();
        assert_eq!(stored.backups.len(), 2);
        assert!(stored.backups.iter().all(has_verified_backup));
        assert!(
            !stored.backups[0]
                .backup_content
                .contains("private-original-data")
        );
        assert!(
            !serde_json::to_string(&result.backups)
                .unwrap()
                .contains("backup_content")
        );
        assert!(
            SnapshotStore::get(&conn, &result.before_snapshot_id)
                .unwrap()
                .is_some()
        );
        assert!(
            SnapshotStore::get(&conn, result.after_snapshot_id.as_ref().unwrap())
                .unwrap()
                .is_some()
        );
        let reopened = SecretsStore::open_at(secrets.path()).unwrap();
        crate::ops::rollback_change_set(&db, &result.id, Default::default(), &reopened)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        assert_eq!(
            std::fs::metadata(&original).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert!(!created.exists());
        assert_eq!(
            ChangeSetStore::get(&conn, &result.id)
                .unwrap()
                .unwrap()
                .status,
            "rolled_back"
        );
    }

    #[tokio::test]
    async fn recovery_authentication_and_newer_work_prevent_overwrite() {
        let (dir, db, secrets) = fixture().await;
        let target = dir.path().join("target");
        std::fs::write(&target, "original").unwrap();
        let result = apply(&db, &secrets, vec![write_step(&target, "changed")]).await;
        let conn = Connection::open(&db).unwrap();
        let backup = ChangeSetStore::get_backups(&conn, &result.id)
            .unwrap()
            .remove(0);
        let mut tampered = backup.backup_content.clone();
        let last = tampered.pop().unwrap();
        tampered.push(if last == '0' { '1' } else { '0' });
        conn.execute(
            "UPDATE change_set_backups SET backup_content=?2 WHERE id=?1",
            params![backup.id, tampered],
        )
        .unwrap();
        assert!(
            crate::ops::rollback_change_set(&db, &result.id, Default::default(), &secrets)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "changed");
        conn.execute(
            "UPDATE change_set_backups SET backup_content=?2 WHERE id=?1",
            params![backup.id, backup.backup_content],
        )
        .unwrap();
        std::fs::write(&target, "newer user work").unwrap();
        assert!(
            crate::ops::rollback_change_set(&db, &result.id, Default::default(), &secrets)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "newer user work");
        assert_eq!(
            ChangeSetStore::get(&conn, &result.id)
                .unwrap()
                .unwrap()
                .status,
            "rollback_failed"
        );
    }

    #[tokio::test]
    async fn recovery_failed_backup_preparation_leaves_file_untouched() {
        let (dir, db, secrets) = fixture().await;
        let target = dir.path().join("target");
        std::fs::write(&target, "original").unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TRIGGER fail_backup BEFORE INSERT ON change_set_backups BEGIN SELECT RAISE(FAIL, 'injected backup failure'); END;").unwrap();
        let result = apply(&db, &secrets, vec![write_step(&target, "changed")]).await;
        assert_eq!(result.status, ChangeSetStatus::Failed);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
    }

    #[tokio::test]
    async fn recovery_interrupted_final_record_keeps_restorable_evidence() {
        let (dir, db, secrets) = fixture().await;
        let target = dir.path().join("target");
        std::fs::write(&target, "original").unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TRIGGER fail_finish BEFORE UPDATE OF status ON change_sets WHEN NEW.status='completed' BEGIN SELECT RAISE(FAIL, 'injected finish failure'); END;").unwrap();
        let result = apply(&db, &secrets, vec![write_step(&target, "changed")]).await;
        assert_eq!(result.status, ChangeSetStatus::Failed);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "changed");
        drop(DurableJournal::new(&db, secrets.clone()).unwrap());
        assert_eq!(
            ChangeSetStore::get(&conn, &result.id)
                .unwrap()
                .unwrap()
                .status,
            "interrupted"
        );
        crate::ops::rollback_change_set(&db, &result.id, Default::default(), &secrets)
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
    }

    #[tokio::test]
    async fn recovery_reverses_multiple_changes_to_the_same_file() {
        let (dir, db, secrets) = fixture().await;
        let target = dir.path().join("target");
        std::fs::write(&target, "original").unwrap();
        let result = apply(
            &db,
            &secrets,
            vec![write_step(&target, "first"), write_step(&target, "second")],
        )
        .await;
        crate::ops::rollback_change_set(&db, &result.id, Default::default(), &secrets)
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
    }

    #[tokio::test]
    async fn recovery_reports_partial_failure_for_unbacked_operations() {
        let (dir, db, secrets) = fixture().await;
        let target = dir.path().join("target");
        std::fs::write(&target, "original").unwrap();
        let mut shell = write_step(&target, "unused");
        shell.command = CommandPreview::new("shell", "printf observed", "unsupported reversal");
        let result = apply(&db, &secrets, vec![write_step(&target, "changed"), shell]).await;
        assert!(
            crate::ops::rollback_change_set(&db, &result.id, Default::default(), &secrets)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
        let conn = Connection::open(&db).unwrap();
        assert_eq!(
            ChangeSetStore::get(&conn, &result.id)
                .unwrap()
                .unwrap()
                .status,
            "rollback_failed"
        );
    }

    #[tokio::test]
    async fn recovery_missing_key_never_replaces_key_or_restores_files() {
        let (dir, db, secrets) = fixture().await;
        let target = dir.path().join("target");
        std::fs::write(&target, "original").unwrap();
        let result = apply(&db, &secrets, vec![write_step(&target, "changed")]).await;
        secrets.delete(BACKUP_KEY).unwrap();
        assert!(
            crate::ops::rollback_change_set(&db, &result.id, Default::default(), &secrets)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "changed");
        assert!(secrets.get(BACKUP_KEY).unwrap().is_none());
    }

    #[tokio::test]
    async fn recovery_retries_after_final_status_failure_without_reapplying_backups() {
        let (dir, db, secrets) = fixture().await;
        let target = dir.path().join("target");
        std::fs::write(&target, "original").unwrap();
        let result = apply(
            &db,
            &secrets,
            vec![write_step(&target, "first"), write_step(&target, "second")],
        )
        .await;
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TRIGGER fail_restored BEFORE UPDATE OF status ON change_sets WHEN NEW.status='rolled_back' BEGIN SELECT RAISE(FAIL, 'injected final status failure'); END;").unwrap();
        assert!(
            crate::ops::rollback_change_set(&db, &result.id, Default::default(), &secrets)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
        conn.execute_batch("DROP TRIGGER fail_restored;").unwrap();
        crate::ops::rollback_change_set(&db, &result.id, Default::default(), &secrets)
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
    }

    #[tokio::test]
    async fn recovery_blocks_active_store_and_link_targets() {
        let (dir, db, secrets) = fixture().await;
        secrets
            .set("fixture", helm_core::Secret::new("original"))
            .unwrap();
        let original = std::fs::read(secrets.path()).unwrap();
        let result = apply(&db, &secrets, vec![write_step(secrets.path(), "bad")]).await;
        assert_eq!(result.status, ChangeSetStatus::Failed);
        assert_eq!(std::fs::read(secrets.path()).unwrap(), original);
        let target = dir.path().join("target");
        let alias = dir.path().join("alias");
        std::fs::write(&target, "original").unwrap();
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        assert!(
            helm_monitor::file_change::prepare_file_change(&write_step(&alias, "bad").command)
                .is_err()
        );
        std::fs::remove_file(&alias).unwrap();
        std::fs::hard_link(&target, &alias).unwrap();
        assert!(
            helm_monitor::file_change::prepare_file_change(&write_step(&alias, "bad").command)
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
    }
}
