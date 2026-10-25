use sqlx::{
    migrate::{MigrateDatabase, MigrateError, Migrator},
    Result, Sqlite, SqlitePool, Transaction,
};
use std::fs;
use std::path::Path;

pub(crate) static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const PEOPLE_MIGRATION_VERSION: i64 = 20260811000000;
const PEOPLE_MIGRATION_LF_CHECKSUM: &str =
    "3722B8A73598E02E31D989430BF2E756539BA575BC4C4A7D981BEE4FB218E549AB36C93071EAA73C006E14CF6CF58B1B";
const PEOPLE_MIGRATION_CRLF_CHECKSUM: &str =
    "75A90F5D84A2D6E6FE0AF8ABC583FEE66A10816AA39916AE2CB73AA728BD5F9FAB199C62CFEDD1F5D1083D795BD09B57";
/// The later migration that rebuilds `people` with a client. Once it has run,
/// the table no longer has the shape the people migration created, and that
/// is not a reason to refuse the checksum repair.
const PEOPLE_BY_CLIENT_VERSION: i64 = 20261007000000;

/// How long the copies made before a migration are kept.
const BACKUP_RETENTION: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);
/// Folder-name prefix of the copy made before a migration.
const BACKUP_PREFIX: &str = "backup-before-";

/// Why the archive could not be opened, as far as the person at the screen
/// needs to know it.
#[derive(Debug)]
pub enum OpenError {
    /// The archive has migrations this build does not know: a newer version
    /// has opened it. Nothing was changed. `needs` is the version that last
    /// migrated it, when it recorded one.
    NewerArchive { needs: Option<String> },
    /// The archive needed migrating and could not be copied first, so it was
    /// not migrated.
    BackupFailed(String),
    Database(sqlx::Error),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NewerArchive { needs } => write!(
                f,
                "the archive was migrated by a newer version ({})",
                needs.as_deref().unwrap_or("unknown")
            ),
            Self::BackupFailed(detail) => {
                write!(f, "could not copy the archive before migrating it: {detail}")
            }
            Self::Database(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<sqlx::Error> for OpenError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl From<MigrateError> for OpenError {
    fn from(error: MigrateError) -> Self {
        Self::Database(error.into())
    }
}

/// This build's version as one number, for `PRAGMA user_version`:
/// `major * 1_000_000 + minor * 1_000 + patch`.
fn version_number(version: &str) -> Option<i64> {
    let mut parts = version.split(|c: char| c == '.' || c == '-' || c == '+');
    let major: i64 = parts.next()?.parse().ok()?;
    let minor: i64 = parts.next()?.parse().ok()?;
    let patch: i64 = parts.next()?.parse().ok()?;
    (minor < 1_000 && patch < 1_000).then_some(major * 1_000_000 + minor * 1_000 + patch)
}

fn version_text(number: i64) -> Option<String> {
    (number > 0).then(|| {
        format!("{}.{}.{}", number / 1_000_000, number / 1_000 % 1_000, number % 1_000)
    })
}

#[derive(Clone)]
pub struct DatabaseManager {
    pool: SqlitePool,
}

impl DatabaseManager {
    /// Opens the archive at `tauri_db_path`, creating an empty one if there is
    /// none. Never fills a new archive from another database found nearby: a
    /// copy of someone else's install would arrive unasked and unencrypted.
    ///
    /// Before migrating an existing archive, a copy of it is made next to it
    /// (`backup-before-<version>-<time>/`): a version that turns out bad can
    /// be rolled back only to an archive it can still read, because an older
    /// version refuses one migrated by a newer one rather than guess.
    pub async fn new(tauri_db_path: &str) -> std::result::Result<Self, OpenError> {
        if let Some(parent_dir) = Path::new(tauri_db_path).parent() {
            if !parent_dir.exists() {
                fs::create_dir_all(parent_dir).map_err(|e| sqlx::Error::Io(e))?;
            }
        }

        if !Path::new(tauri_db_path).exists() {
            log::info!("Creating database at {}", tauri_db_path);
            Sqlite::create_database(tauri_db_path).await?;
        }

        let migration_pool = SqlitePool::connect_with(Self::connect_options(tauri_db_path)?).await?;

        let applied = Self::applied_versions(&migration_pool).await?;
        let known: std::collections::HashSet<i64> = MIGRATOR.iter().map(|m| m.version).collect();
        if applied.iter().any(|version| !known.contains(version)) {
            // Only a version newer than this one is worth naming. A newer
            // version that failed half way through its migrations never
            // recorded itself, and the number there is an older one's -
            // possibly this very version's.
            let own = version_number(env!("CARGO_PKG_VERSION"));
            let needs = sqlx::query_scalar::<_, i64>("PRAGMA user_version")
                .fetch_one(&migration_pool)
                .await
                .ok()
                .filter(|&recorded| own.is_some_and(|own| recorded > own))
                .and_then(version_text);
            migration_pool.close().await;
            log::error!(
                "The archive has migrations this version does not know; a newer version ({}) opened it. Not touching it.",
                needs.as_deref().unwrap_or("unknown")
            );
            return Err(OpenError::NewerArchive { needs });
        }

        let pending = MIGRATOR
            .iter()
            .filter(|migration| !applied.contains(&migration.version))
            .count();
        // Every start that still has migrations to apply copies again, also
        // after a failed attempt. Archives are small, and the copy from the
        // first attempt stays next to the new one until the week is up.
        if pending > 0 && !applied.is_empty() {
            if let Some(root) = Path::new(tauri_db_path).parent() {
                match Self::copy_before_migrating(&migration_pool, root).await {
                    Ok(copy) => log::info!(
                        "{pending} migrations to apply; the archive was copied first to {}",
                        copy.display()
                    ),
                    Err(error) => {
                        migration_pool.close().await;
                        log::error!("Not migrating: the archive could not be copied first: {error}");
                        return Err(OpenError::BackupFailed(error.to_string()));
                    }
                }
            }
        }

        Self::run_migrations(&migration_pool).await?;
        if pending > 0 {
            Self::record_migrating_version(&migration_pool).await;
        }
        // A failed checksum pass can leave another pooled SQLite connection
        // holding the pre-migration schema. Reopen before serving app queries.
        migration_pool.close().await;
        let pool = SqlitePool::connect_with(Self::connect_options(tauri_db_path)?).await?;

        Ok(DatabaseManager { pool })
    }

    /// Versions of the migrations applied to the archive; none for a new one.
    async fn applied_versions(pool: &SqlitePool) -> Result<std::collections::HashSet<i64>> {
        let has_table: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
        )
        .fetch_one(pool)
        .await?;
        if has_table == 0 {
            return Ok(Default::default());
        }
        let versions: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success = 1")
                .fetch_all(pool)
                .await?;
        Ok(versions.into_iter().collect())
    }

    /// A consistent copy of the archive into `<root>/backup-before-<version>-<time>/`.
    ///
    /// `VACUUM INTO` writes the whole database, journal included, through
    /// SQLite itself, so the copy is consistent even with a write-ahead log
    /// beside the file. It copies values as they are: sealed fields stay
    /// sealed.
    async fn copy_before_migrating(pool: &SqlitePool, root: &Path) -> Result<std::path::PathBuf> {
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let folder = root.join(format!("{BACKUP_PREFIX}{}-{stamp}", env!("CARGO_PKG_VERSION")));
        fs::create_dir_all(&folder).map_err(sqlx::Error::Io)?;
        let copy = folder.join("meeting_minutes.sqlite");
        if let Err(error) = sqlx::query("VACUUM INTO ?")
            .bind(copy.to_string_lossy().to_string())
            .execute(pool)
            .await
        {
            // A copy cut short (a full disk) must not stay looking like a
            // good one.
            let _ = fs::remove_dir_all(&folder);
            return Err(error);
        }
        Ok(folder)
    }

    /// Remember which version migrated the archive, so an older version that
    /// meets it can say which one to install instead. `user_version` is a
    /// header field SQLite keeps for the application; sqlx does not use it.
    async fn record_migrating_version(pool: &SqlitePool) {
        let Some(number) = version_number(env!("CARGO_PKG_VERSION")) else {
            return;
        };
        if let Err(error) = sqlx::query(&format!("PRAGMA user_version = {number}"))
            .execute(pool)
            .await
        {
            log::warn!("Could not record the version that migrated the archive: {error}");
        }
    }

    /// How every connection to the archive is opened.
    ///
    /// `secure_delete` makes SQLite overwrite what it frees with zeros. Without
    /// it a deleted meeting, or a value replaced by its sealed form, stays
    /// readable in the file's free pages until something runs VACUUM.
    fn connect_options(path: &str) -> Result<sqlx::sqlite::SqliteConnectOptions> {
        use std::str::FromStr;
        Ok(sqlx::sqlite::SqliteConnectOptions::from_str(path)?.pragma("secure_delete", "ON"))
    }

    async fn run_migrations(pool: &SqlitePool) -> Result<()> {
        // One repair per pass; each pass either finishes or names the next
        // migration whose checksum it disputes.
        for _ in 0..=MIGRATOR.iter().count() {
            match MIGRATOR.run(pool).await {
                Ok(()) => return Ok(()),
                Err(MigrateError::VersionMismatch(PEOPLE_MIGRATION_VERSION)) => {
                    Self::repair_people_migration_checksum(pool).await?;
                }
                Err(MigrateError::VersionMismatch(version)) => {
                    Self::repair_line_ending_checksum(pool, version).await?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        MIGRATOR.run(pool).await.map_err(Into::into)
    }

    /// Accept an applied migration whose recorded checksum is that of this
    /// build's text with the other line endings.
    ///
    /// sqlx checksums the migration's text as it was compiled in, and a
    /// checkout on Windows may turn LF into CRLF (or the other way round).
    /// The ten migrations released before they were pinned in
    /// `.gitattributes` are therefore out there with either checksum, and an
    /// update built from the other kind of checkout would refuse the archive.
    /// The SQL is the same either way, so the record is brought in line with
    /// this build. Any other difference is still refused.
    async fn repair_line_ending_checksum(pool: &SqlitePool, version: i64) -> Result<()> {
        let migration = MIGRATOR
            .iter()
            .find(|migration| migration.version == version)
            .ok_or(MigrateError::VersionNotPresent(version))?;
        let mut transaction = pool.begin().await?;
        let stored = sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT checksum FROM _sqlx_migrations WHERE version = ? AND success = 1",
        )
        .bind(version)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(stored) = stored else {
            return Err(MigrateError::VersionMismatch(version).into());
        };
        if !line_ending_variants_checksums(&migration.sql).contains(&stored) {
            return Err(MigrateError::VersionMismatch(version).into());
        }

        let result = sqlx::query(
            "UPDATE _sqlx_migrations SET checksum = ? WHERE version = ? AND success = 1 AND checksum = ?",
        )
        .bind(migration.checksum.as_ref())
        .bind(version)
        .bind(&stored)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(sqlx::Error::Protocol(format!(
                "Failed to repair the checksum of migration {version}"
            )));
        }
        transaction.commit().await?;
        log::warn!("Repaired migration {version} checksum: the archive recorded it with other line endings");
        Ok(())
    }

    async fn repair_people_migration_checksum(pool: &SqlitePool) -> Result<()> {
        let mut transaction = pool.begin().await?;
        let stored_checksum = sqlx::query_scalar::<_, String>(
            "SELECT upper(hex(checksum)) FROM _sqlx_migrations WHERE version = ? AND success = 1",
        )
        .bind(PEOPLE_MIGRATION_VERSION)
        .fetch_optional(&mut *transaction)
        .await?;

        let Some(stored_checksum) = stored_checksum else {
            return Err(sqlx::Error::Protocol(
                "People migration checksum mismatch has no successful migration record".to_string(),
            ));
        };
        if stored_checksum != PEOPLE_MIGRATION_LF_CHECKSUM
            && stored_checksum != PEOPLE_MIGRATION_CRLF_CHECKSUM
        {
            return Err(MigrateError::VersionMismatch(PEOPLE_MIGRATION_VERSION).into());
        }

        let schema_objects = sqlx::query_as::<_, (String, String, String)>(
            r#"
            SELECT type, name, sql
            FROM sqlite_master
            WHERE (type = 'table' AND name IN ('people', 'person_speakers'))
               OR (type = 'index' AND name IN (
                   'idx_person_speakers_person_id',
                   'idx_person_speakers_meeting_id',
                   'idx_people_display_name'
               ))
            ORDER BY type, name
            "#,
        )
        .fetch_all(&mut *transaction)
        .await?;
        let expected_schema = [
            (
                "index",
                "idx_people_display_name",
                "CREATE INDEX idx_people_display_name ON people(display_name)",
            ),
            (
                "index",
                "idx_person_speakers_meeting_id",
                "CREATE INDEX idx_person_speakers_meeting_id ON person_speakers(meeting_id)",
            ),
            (
                "index",
                "idx_person_speakers_person_id",
                "CREATE INDEX idx_person_speakers_person_id ON person_speakers(person_id)",
            ),
            (
                "table",
                "people",
                r#"CREATE TABLE people (
                    id TEXT PRIMARY KEY,
                    display_name TEXT NOT NULL CHECK (length(trim(display_name)) > 0),
                    normalized_name TEXT NOT NULL UNIQUE CHECK (length(normalized_name) > 0),
                    notes TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                )"#,
            ),
            (
                "table",
                "person_speakers",
                r#"CREATE TABLE person_speakers (
                    person_id TEXT NOT NULL,
                    meeting_id TEXT NOT NULL,
                    speaker_label TEXT NOT NULL,
                    PRIMARY KEY (person_id, meeting_id, speaker_label),
                    UNIQUE (meeting_id, speaker_label),
                    FOREIGN KEY (person_id) REFERENCES people(id) ON DELETE CASCADE,
                    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
                )"#,
            ),
        ];
        let people_rebuilt: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM _sqlx_migrations WHERE version = ? AND success = 1",
        )
        .bind(PEOPLE_BY_CLIENT_VERSION)
        .fetch_one(&mut *transaction)
        .await?;
        let checked = |name: &str| !(people_rebuilt > 0 && name == "people");
        let schema_objects: Vec<_> = schema_objects
            .into_iter()
            .filter(|(_, name, _)| checked(name))
            .collect();
        let expected_schema: Vec<_> = expected_schema
            .into_iter()
            .filter(|(_, name, _)| checked(name))
            .collect();
        let schema_matches = schema_objects.len() == expected_schema.len()
            && schema_objects.iter().zip(expected_schema).all(
                |((object_type, name, sql), (expected_type, expected_name, expected_sql))| {
                    object_type == expected_type
                        && name == expected_name
                        && Self::normalize_schema_sql(sql)
                            == Self::normalize_schema_sql(expected_sql)
                },
            );
        if !schema_matches {
            return Err(sqlx::Error::Protocol(
                "Refusing to repair the people migration checksum because its schema differs"
                    .to_string(),
            ));
        }

        let current_migration = MIGRATOR
            .iter()
            .find(|migration| migration.version == PEOPLE_MIGRATION_VERSION)
            .ok_or_else(|| MigrateError::VersionNotPresent(PEOPLE_MIGRATION_VERSION))?;
        let result = sqlx::query(
            "UPDATE _sqlx_migrations SET checksum = ? WHERE version = ? AND success = 1 AND upper(hex(checksum)) = ?",
        )
        .bind(current_migration.checksum.as_ref())
        .bind(PEOPLE_MIGRATION_VERSION)
        .bind(&stored_checksum)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(sqlx::Error::Protocol(
                "Failed to repair the people migration checksum".to_string(),
            ));
        }
        transaction.commit().await?;

        log::warn!(
            "Repaired migration {} checksum after the known Windows LF/CRLF packaging mismatch",
            PEOPLE_MIGRATION_VERSION
        );
        Ok(())
    }

    fn normalize_schema_sql(sql: &str) -> String {
        sql.chars()
            .filter(|character| !character.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect()
    }

    // NOTE: So for the first time users they needs to start the application
    // after they can just delete the existing .sqlite file and then copy the existing .db file to
    // the current app dir, So the system detects legacy db and copy it and starts with that data
    // (Newly created .sqlite with the copied content from .db)
    pub async fn new_from_app_handle(
        app_handle: &tauri::AppHandle,
    ) -> std::result::Result<Self, OpenError> {
        let opened = Self::open_install_archive(app_handle).await;
        if opened.is_ok() {
            // Only once this version has opened the archive: until then the
            // copy may be what a roll-back needs.
            prune_migration_backups(&crate::paths::install_data_root(), std::time::SystemTime::now());
        }
        opened
    }

    async fn open_install_archive(
        app_handle: &tauri::AppHandle,
    ) -> std::result::Result<Self, OpenError> {
        // Resolve the install-local data directory (portable / self-contained).
        let _ = app_handle; // retained for signature compatibility
        let app_data_dir = crate::paths::install_data_root();
        if !app_data_dir.exists() {
            fs::create_dir_all(&app_data_dir).map_err(|e| sqlx::Error::Io(e))?;
        }

        // Define database paths
        let tauri_db_path = app_data_dir
            .join("meeting_minutes.sqlite")
            .to_string_lossy()
            .to_string();

        // WAL file paths for defensive cleanup
        let wal_path = app_data_dir.join("meeting_minutes.sqlite-wal");
        let shm_path = app_data_dir.join("meeting_minutes.sqlite-shm");

        log::info!("Tauri DB path: {}", tauri_db_path);

        // Try to open database with defensive WAL handling
        match Self::new(&tauri_db_path).await {
            Ok(db_manager) => {
                log::info!("Database opened successfully");
                Ok(db_manager)
            }
            Err(e) => {
                // Check if error is due to corrupted WAL file
                let error_msg = e.to_string();
                let database_error = matches!(e, OpenError::Database(_));
                if database_error && (error_msg.contains("malformed") || error_msg.contains("corrupt")) {
                    log::warn!("Database appears corrupted, likely due to orphaned WAL file. Attempting recovery...");
                    log::warn!("Error details: {}", error_msg);

                    // The WAL holds committed transactions that have not been
                    // copied into the database yet — the most recent work. It
                    // is set aside with the database, never simply deleted,
                    // so it can still be replayed by hand if the retry below
                    // opens a database without it.
                    let db_path = app_data_dir.join("meeting_minutes.sqlite");
                    match quarantine(&app_data_dir, &[&db_path, &wal_path, &shm_path]) {
                        Ok(folder) => log::error!(
                            "Database and journal copied aside before recovery: {}",
                            folder.display()
                        ),
                        Err(error) => {
                            log::error!(
                                "Could not copy the database aside ({error}); not touching the journal"
                            );
                            return Err(e);
                        }
                    }
                    for journal in [&wal_path, &shm_path] {
                        if let Err(error) = fs::remove_file(journal) {
                            if error.kind() != std::io::ErrorKind::NotFound {
                                log::warn!("Could not move the journal out of the way: {error}");
                            }
                        }
                    }

                    // Retry connection without WAL files
                    log::info!("Retrying database connection after WAL cleanup...");
                    match Self::new(&tauri_db_path).await {
                        Ok(db_manager) => {
                            log::info!("Database opened successfully after WAL recovery");
                            Ok(db_manager)
                        }
                        Err(retry_err) => {
                            log::error!(
                                "Database connection failed even after WAL cleanup: {}",
                                retry_err
                            );
                            Err(retry_err)
                        }
                    }
                } else {
                    // Not a WAL-related error, propagate original error
                    log::error!("Database connection failed: {}", error_msg);
                    Err(e)
                }
            }
        }
    }

    /// Check if this is the first launch (sqlite database doesn't exist yet)
    pub async fn is_first_launch(app_handle: &tauri::AppHandle) -> Result<bool> {
        let _ = app_handle; // retained for signature compatibility
        let app_data_dir = crate::paths::install_data_root();

        let tauri_db_path = app_data_dir.join("meeting_minutes.sqlite");

        Ok(!tauri_db_path.exists())
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn with_transaction<T, F, Fut>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Transaction<'_, Sqlite>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut tx = self.pool.begin().await?;
        let result = f(&mut tx).await;

        match result {
            Ok(val) => {
                tx.commit().await?;
                Ok(val)
            }
            Err(err) => {
                tx.rollback().await?;
                Err(err)
            }
        }
    }

    /// Cleanup database connection and checkpoint WAL
    /// This should be called on application shutdown to ensure:
    /// - All WAL changes are written to the main database file
    /// - The .wal and .shm files are deleted
    /// - Connection pool is gracefully closed
    pub async fn cleanup(&self) -> Result<()> {
        log::info!("Starting database cleanup...");

        // Force checkpoint of WAL to main database file and remove WAL file
        // TRUNCATE mode: checkpoints all pages AND deletes the WAL file
        match sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await
        {
            Ok(_) => log::info!("WAL checkpoint completed successfully"),
            Err(e) => log::warn!("WAL checkpoint failed (non-fatal): {}", e),
        }

        // Close the connection pool gracefully
        self.pool.close().await;
        log::info!("Database connection pool closed");

        Ok(())
    }
}

/// sqlx's checksums (SHA-384) of `sql` with LF and with CRLF line endings.
fn line_ending_variants_checksums(sql: &str) -> [Vec<u8>; 2] {
    use sha2::{Digest, Sha384};
    let lf = sql.replace("\r\n", "\n");
    let crlf = lf.replace('\n', "\r\n");
    [
        Sha384::digest(lf.as_bytes()).to_vec(),
        Sha384::digest(crlf.as_bytes()).to_vec(),
    ]
}

/// The folders in `root` that hold a copy made before a migration.
fn migration_backups(root: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            entry.path().is_dir()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(BACKUP_PREFIX))
        })
        .map(|entry| entry.path())
        .collect()
}

/// Removes every copy made before a migration, whatever its age, and returns
/// how many went. For when the archive has just been sealed: a copy made
/// while it was unprotected is the whole archive in the clear.
pub fn remove_migration_backups(root: &Path) -> std::io::Result<usize> {
    let mut removed = 0;
    for folder in migration_backups(root) {
        fs::remove_dir_all(&folder)?;
        removed += 1;
        log::info!("Removed a copy made before a migration: {}", folder.display());
    }
    Ok(removed)
}

/// Removes the copies made before a migration once they are older than a
/// week (by the copy's own modification time), and returns how many went.
/// Anything else in `root` is left alone.
pub(crate) fn prune_migration_backups(root: &Path, now: std::time::SystemTime) -> usize {
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_backup = path.is_dir()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(BACKUP_PREFIX));
        if !is_backup {
            continue;
        }
        let made = fs::metadata(path.join("meeting_minutes.sqlite"))
            .or_else(|_| fs::metadata(&path))
            .and_then(|meta| meta.modified());
        let old = made
            .ok()
            .and_then(|made| now.duration_since(made).ok())
            .is_some_and(|age| age > BACKUP_RETENTION);
        if old {
            match fs::remove_dir_all(&path) {
                Ok(()) => {
                    removed += 1;
                    log::info!("Removed a copy made before a migration a week ago: {}", path.display());
                }
                Err(error) => log::warn!("Could not remove an old pre-migration copy: {error}"),
            }
        }
    }
    removed
}

/// Copies the given files into `<root>/recovery/<timestamp>/`, skipping any
/// that do not exist, and returns that folder. Copies rather than moves: the
/// caller decides what to remove once the copies are safe.
pub(crate) fn quarantine(root: &Path, files: &[&Path]) -> std::io::Result<std::path::PathBuf> {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ").to_string();
    let folder = root.join("recovery").join(stamp);
    fs::create_dir_all(&folder)?;
    for file in files {
        if !file.exists() {
            continue;
        }
        let name = file
            .file_name()
            .ok_or_else(|| std::io::Error::other("a file to set aside has no name"))?;
        fs::copy(file, folder.join(name))?;
    }
    Ok(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connections_overwrite_what_they_free() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive.sqlite");
        let options = DatabaseManager::connect_options(path.to_str().unwrap())
            .unwrap()
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        let (secure_delete,): (i64,) = sqlx::query_as("PRAGMA secure_delete")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(secure_delete, 1);
    }

    /// The archive as it stood before people belonged to a client, with one
    /// profile glued across two clients and a meeting with none.
    async fn archive_before_people_had_clients() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for migration in MIGRATOR.iter() {
            if migration.version < PEOPLE_BY_CLIENT_VERSION {
                sqlx::raw_sql(&*migration.sql).execute(&pool).await.unwrap();
            }
        }
        sqlx::raw_sql(
            "INSERT INTO clients (id, display_name, normalized_name, created_at, updated_at) \
                 VALUES ('c1', 'Клиент 1', 'клиент 1', 'n', 'n'), \
                        ('c2', 'Клиент 2', 'клиент 2', 'n', 'n'); \
             INSERT INTO meetings (id, title, created_at, updated_at, client_id) VALUES \
                 ('m1', 'Первая', '2026-09-01', 'n', 'c1'), \
                 ('m2', 'Вторая', '2026-09-02', 'n', 'c1'), \
                 ('m3', 'Третья', '2026-09-03', 'n', 'c2'), \
                 ('m4', 'Без клиента', '2026-09-04', 'n', NULL); \
             INSERT INTO people (id, display_name, normalized_name, notes, created_at, updated_at) \
                 VALUES ('p-glued', 'Тест А', 'тест а', 'заметка', 'n', 'n'), \
                        ('p-loose', 'Гость А', 'гость а', NULL, 'n', 'n'), \
                        ('p-noted', 'Старый', 'старый', 'тоже заметка', 'n', 'n'); \
             INSERT INTO person_speakers VALUES \
                 ('p-glued', 'm1', 'Тест А'), ('p-glued', 'm2', 'Тест А'), \
                 ('p-glued', 'm3', 'Тест А'), ('p-glued', 'm4', 'Тест А'), \
                 ('p-loose', 'm4', 'Гость А'), ('p-noted', 'm4', 'Старый');",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn a_profile_glued_across_clients_is_split_and_keeps_its_notes_once() {
        let pool = archive_before_people_had_clients().await;
        sqlx::raw_sql(include_str!(
            "../../migrations/20261007000000_people_belong_to_a_client.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();

        let people: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT id, display_name, notes, client_id FROM people ORDER BY client_id, id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let links: Vec<(String, String)> = sqlx::query_as(
            "SELECT person_id, meeting_id FROM person_speakers ORDER BY meeting_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();

        // The client with more meetings keeps the profile, its id and the notes.
        let home = people.iter().find(|p| p.0 == "p-glued").unwrap();
        assert_eq!(home.2.as_deref(), Some("заметка"));
        assert_eq!(home.3.as_deref(), Some("c1"));
        // The other client gets a profile of its own, same name, no notes.
        let split = people
            .iter()
            .find(|p| p.3.as_deref() == Some("c2"))
            .expect("a profile for the second client");
        assert_ne!(split.0, "p-glued");
        assert_eq!(split.1, "Тест А");
        assert_eq!(split.2, None);

        assert_eq!(
            links,
            vec![
                ("p-glued".to_string(), "m1".to_string()),
                ("p-glued".to_string(), "m2".to_string()),
                (split.0.clone(), "m3".to_string()),
            ],
            "the meeting without a client keeps no links"
        );

        // A profile only the clientless meeting had: gone without notes, kept
        // with them.
        assert!(people.iter().all(|p| p.0 != "p-loose"));
        let noted = people.iter().find(|p| p.0 == "p-noted").unwrap();
        assert_eq!(noted.2.as_deref(), Some("тоже заметка"));
        assert_eq!(noted.3, None);

        // Foreign keys are back on, and nothing points at a missing person.
        let (foreign_keys,): (i64,) = sqlx::query_as("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(foreign_keys, 1);
        let broken = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(broken.is_empty());
    }

    #[tokio::test]
    async fn the_same_name_may_now_exist_once_per_client() {
        let pool = archive_before_people_had_clients().await;
        sqlx::raw_sql(include_str!(
            "../../migrations/20261007000000_people_belong_to_a_client.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();

        // A second "тест а" for the first client is still refused...
        let duplicate = sqlx::query(
            "INSERT INTO people (id, display_name, normalized_name, created_at, updated_at, client_id) \
             VALUES ('p-dup', 'Тест А', 'тест а', 'n', 'n', 'c1')",
        )
        .execute(&pool)
        .await;
        assert!(duplicate.is_err());
        // ...and deleting a person still takes their links with them.
        sqlx::query("DELETE FROM people WHERE id = 'p-glued'")
            .execute(&pool)
            .await
            .unwrap();
        let left: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM person_speakers WHERE person_id = 'p-glued'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn a_new_archive_does_not_take_in_a_database_lying_beside_it() {
        // The old backend's file name, in the same folder, with a meeting in
        // it. Upstream builds copied it into a new archive without asking —
        // in the clear, whoever's it was.
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("meeting_minutes.db");
        let pool = SqlitePool::connect_with(
            DatabaseManager::connect_options(other.to_str().unwrap())
                .unwrap()
                .create_if_missing(true),
        )
        .await
        .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at) \
             VALUES ('m1', 'Чужая встреча', 'n', 'n')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;

        let archive = dir.path().join("meeting_minutes.sqlite");
        let manager = DatabaseManager::new(archive.to_str().unwrap()).await.unwrap();
        let (meetings,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM meetings")
            .fetch_one(manager.pool())
            .await
            .unwrap();
        assert_eq!(meetings, 0, "the new archive started from the other database");
        manager.pool().close().await;
    }

    #[test]
    fn a_journal_is_set_aside_before_anything_removes_it() {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("meeting_minutes.sqlite");
        let wal = root.path().join("meeting_minutes.sqlite-wal");
        let shm = root.path().join("meeting_minutes.sqlite-shm");
        fs::write(&db, b"db").unwrap();
        fs::write(&wal, b"latest work").unwrap();

        let folder = quarantine(root.path(), &[&db, &wal, &shm]).unwrap();

        assert!(folder.starts_with(root.path().join("recovery")));
        assert_eq!(fs::read(folder.join("meeting_minutes.sqlite-wal")).unwrap(), b"latest work");
        assert!(folder.join("meeting_minutes.sqlite").is_file());
        // A missing file is skipped, not an error.
        assert!(!folder.join("meeting_minutes.sqlite-shm").exists());
        // The originals are untouched: removing them is the caller's call.
        assert!(wal.is_file());
    }

    /// Migrations released before the convention below; their checksums are
    /// already out in the wild with platform-specific line endings.
    const HISTORICAL_MIGRATIONS: [&str; 10] = [
        "20250916100000_initial_schema.sql",
        "20250920155811_add_openrouter_api_key.sql",
        "20251006000000_add_audio_sync_fields.sql",
        "20251010153942_add_ollama_endpoint.sql",
        "20251101000000_add_summary_backup.sql",
        "20251105120000_add_pro_license_custom_openai.sql",
        "20251110000000_add_grace_period_to_licensing.sql",
        "20251110000001_add_speaker_field.sql",
        "20251223000000_add_meeting_notes.sql",
        "20251229000000_add_gemini_api_key.sql",
    ];

    /// sqlx checksums the text of each migration, so a checkout that rewrites
    /// line endings makes an applied migration look modified and the app then
    /// refuses to open its database. Every new migration must therefore be
    /// pinned to LF in .gitattributes - this catches the omission here rather
    /// than in a build that has already reached someone's machine.
    #[test]
    fn every_new_migration_is_pinned_to_lf_endings() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let attributes = std::fs::read_to_string(root.join("../../.gitattributes"))
            .expect("repository .gitattributes");

        let mut unpinned = Vec::new();
        for entry in std::fs::read_dir(root.join("migrations")).expect("migrations directory") {
            let entry = entry.expect("migration entry");
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".sql") || HISTORICAL_MIGRATIONS.contains(&name.as_str()) {
                continue;
            }
            let pinned = attributes.lines().any(|line| {
                line.contains(&name) && line.contains("eol=lf") && !line.trim_start().starts_with('#')
            });
            if !pinned {
                unpinned.push(name);
            }
        }

        assert!(
            unpinned.is_empty(),
            "these migrations are not pinned to LF in .gitattributes: {:?}",
            unpinned
        );
    }

    #[tokio::test]
    async fn repairs_known_people_migration_line_ending_checksum() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        sqlx::query(
            "UPDATE _sqlx_migrations SET checksum = X'75A90F5D84A2D6E6FE0AF8ABC583FEE66A10816AA39916AE2CB73AA728BD5F9FAB199C62CFEDD1F5D1083D795BD09B57' WHERE version = ?",
        )
        .bind(PEOPLE_MIGRATION_VERSION)
        .execute(&pool)
        .await
        .unwrap();

        DatabaseManager::run_migrations(&pool).await.unwrap();

        let stored_checksum: String = sqlx::query_scalar(
            "SELECT upper(hex(checksum)) FROM _sqlx_migrations WHERE version = ?",
        )
        .bind(PEOPLE_MIGRATION_VERSION)
        .fetch_one(&pool)
        .await
        .unwrap();
        let expected_checksum = MIGRATOR
            .iter()
            .find(|migration| migration.version == PEOPLE_MIGRATION_VERSION)
            .unwrap()
            .checksum
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>();
        assert_eq!(stored_checksum, expected_checksum);
    }

    #[tokio::test]
    async fn rejects_unknown_people_migration_checksum() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = ?")
            .bind(PEOPLE_MIGRATION_VERSION)
            .execute(&pool)
            .await
            .unwrap();

        let error = DatabaseManager::run_migrations(&pool).await.unwrap_err();
        assert!(error
            .to_string()
            .contains("previously applied but has been modified"));
    }

    #[tokio::test]
    async fn rejects_known_checksum_when_people_schema_differs() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        sqlx::query("DROP INDEX idx_people_display_name")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE INDEX idx_people_display_name ON people(normalized_name)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE _sqlx_migrations SET checksum = X'75A90F5D84A2D6E6FE0AF8ABC583FEE66A10816AA39916AE2CB73AA728BD5F9FAB199C62CFEDD1F5D1083D795BD09B57' WHERE version = ?",
        )
        .bind(PEOPLE_MIGRATION_VERSION)
        .execute(&pool)
        .await
        .unwrap();

        let error = DatabaseManager::run_migrations(&pool).await.unwrap_err();
        assert!(error.to_string().contains("schema differs"));
    }
}

#[cfg(test)]
mod migration_safety_tests {
    use super::*;
    use std::borrow::Cow;

    /// This build's migrations without the newest one: an archive left by the
    /// previous release, with one migration still to apply.
    fn previous_release() -> Migrator {
        let newest = MIGRATOR.iter().map(|m| m.version).max().unwrap();
        Migrator {
            migrations: Cow::Owned(MIGRATOR.iter().filter(|m| m.version < newest).cloned().collect()),
            ignore_missing: false,
            locking: true,
            no_tx: false,
        }
    }

    async fn archive_file(dir: &Path, migrator: &Migrator) -> String {
        let path = dir.join("meeting_minutes.sqlite").to_string_lossy().to_string();
        Sqlite::create_database(&path).await.unwrap();
        let pool = SqlitePool::connect(&path).await.unwrap();
        migrator.run(&pool).await.unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m1', 'Kept', 'n', 'n')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        path
    }

    fn backups(dir: &Path) -> Vec<std::path::PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name().unwrap().to_string_lossy().starts_with(BACKUP_PREFIX)
            })
            .collect()
    }

    #[tokio::test]
    async fn an_archive_is_copied_before_it_is_migrated_and_the_copy_still_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = archive_file(dir.path(), &previous_release()).await;

        let manager = DatabaseManager::new(&path).await.unwrap();
        manager.pool().close().await;

        let copies = backups(dir.path());
        assert_eq!(copies.len(), 1, "one copy before the migration");
        let copy = copies[0].join("meeting_minutes.sqlite");
        let old = SqlitePool::connect(copy.to_str().unwrap()).await.unwrap();
        // The copy is the archive as the previous release left it: that
        // release opens it, and the data is there.
        previous_release().run(&old).await.unwrap();
        let title: String = sqlx::query_scalar("SELECT title FROM meetings WHERE id = 'm1'")
            .fetch_one(&old)
            .await
            .unwrap();
        assert_eq!(title, "Kept");
        old.close().await;

        // Opening again with nothing to migrate makes no second copy.
        let again = DatabaseManager::new(&path).await.unwrap();
        again.pool().close().await;
        assert_eq!(backups(dir.path()).len(), 1);
    }

    #[tokio::test]
    async fn a_new_archive_is_not_copied() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meeting_minutes.sqlite");
        let manager = DatabaseManager::new(path.to_str().unwrap()).await.unwrap();
        manager.pool().close().await;
        assert!(backups(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn an_archive_from_a_newer_version_is_refused_untouched_and_named() {
        let dir = tempfile::tempdir().unwrap();
        let path = archive_file(dir.path(), &MIGRATOR).await;
        let pool = SqlitePool::connect(&path).await.unwrap();
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
             VALUES (99991231000000, 'from the future', 1, X'00', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("PRAGMA user_version = 2040").execute(&pool).await.unwrap();
        pool.close().await;
        let before = fs::read(&path).unwrap();

        let refused = DatabaseManager::new(&path).await.err().expect("refused");
        assert!(
            matches!(&refused, OpenError::NewerArchive { needs: Some(v) } if v == "0.2.40"),
            "{refused:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), before, "the archive is not touched");
        assert!(backups(dir.path()).is_empty());
    }

    /// A newer version that failed half way never recorded itself; the number
    /// left there is this version's own, and naming it would say "install
    /// the version you are running".
    #[tokio::test]
    async fn a_recorded_version_that_is_not_newer_is_not_named() {
        let dir = tempfile::tempdir().unwrap();
        let path = archive_file(dir.path(), &MIGRATOR).await;
        let pool = SqlitePool::connect(&path).await.unwrap();
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
             VALUES (99991231000000, 'half of a newer version', 1, X'00', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let own = version_number(env!("CARGO_PKG_VERSION")).unwrap();
        sqlx::query(&format!("PRAGMA user_version = {own}")).execute(&pool).await.unwrap();
        pool.close().await;

        let refused = DatabaseManager::new(&path).await.err().expect("refused");
        assert!(matches!(&refused, OpenError::NewerArchive { needs: None }), "{refused:?}");
    }

    #[tokio::test]
    async fn the_version_that_migrated_the_archive_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let path = archive_file(dir.path(), &previous_release()).await;
        let manager = DatabaseManager::new(&path).await.unwrap();
        let recorded: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(manager.pool())
            .await
            .unwrap();
        manager.pool().close().await;
        assert_eq!(version_text(recorded).as_deref(), Some(env!("CARGO_PKG_VERSION")));
    }

    #[tokio::test]
    async fn a_historical_migration_recorded_with_other_line_endings_opens() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let initial = MIGRATOR.iter().next().unwrap();
        let [lf, crlf] = line_ending_variants_checksums(&initial.sql);
        let other = if initial.checksum.as_ref() == lf.as_slice() { crlf } else { lf };
        sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
            .bind(&other)
            .bind(initial.version)
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(MIGRATOR.run(&pool).await, Err(MigrateError::VersionMismatch(_))));

        DatabaseManager::run_migrations(&pool).await.unwrap();

        let stored: Vec<u8> =
            sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = ?")
                .bind(initial.version)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored, initial.checksum.as_ref());
    }

    #[tokio::test]
    async fn a_migration_that_differs_in_more_than_line_endings_is_still_refused() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let initial = MIGRATOR.iter().next().unwrap();
        let edited = line_ending_variants_checksums(&format!("{} -- edited", initial.sql))[0].clone();
        sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
            .bind(&edited)
            .bind(initial.version)
            .execute(&pool)
            .await
            .unwrap();

        let error = DatabaseManager::run_migrations(&pool).await.unwrap_err();
        assert!(
            matches!(&error, sqlx::Error::Migrate(inner) if matches!(**inner, MigrateError::VersionMismatch(v) if v == initial.version)),
            "{error}"
        );
    }

    #[test]
    fn copies_older_than_a_week_are_removed_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join(format!("{BACKUP_PREFIX}0.2.34-20261001T000000Z"));
        let fresh = dir.path().join(format!("{BACKUP_PREFIX}0.2.35-20261020T000000Z"));
        let unrelated = dir.path().join("recovery");
        for folder in [&old, &fresh, &unrelated] {
            fs::create_dir_all(folder).unwrap();
            fs::write(folder.join("meeting_minutes.sqlite"), b"x").unwrap();
        }
        let now = std::time::SystemTime::now();
        let ten_days_ago = now - std::time::Duration::from_secs(10 * 24 * 60 * 60);
        for folder in [&old, &unrelated] {
            fs::File::options()
                .write(true)
                .open(folder.join("meeting_minutes.sqlite"))
                .unwrap()
                .set_modified(ten_days_ago)
                .unwrap();
        }

        assert_eq!(prune_migration_backups(dir.path(), now), 1);
        assert!(!old.exists());
        assert!(fresh.exists());
        assert!(unrelated.exists(), "recovery copies are not this function's");
    }

    #[tokio::test]
    async fn a_copy_that_cannot_be_written_is_an_error_not_a_silent_skip() {
        let dir = tempfile::tempdir().unwrap();
        let path = archive_file(dir.path(), &previous_release()).await;
        let pool = SqlitePool::connect(&path).await.unwrap();
        // A file where the copy's folder has to go.
        let blocked = dir.path().join("not-a-folder");
        fs::write(&blocked, b"x").unwrap();

        assert!(DatabaseManager::copy_before_migrating(&pool, &blocked).await.is_err());
        pool.close().await;
    }

    #[test]
    fn versions_survive_the_round_trip_through_user_version() {
        assert_eq!(version_number("0.2.34"), Some(2_034));
        assert_eq!(version_text(2_034).as_deref(), Some("0.2.34"));
        assert_eq!(version_number("1.10.3-beta"), Some(1_010_003));
        assert_eq!(version_text(0), None);
        assert_eq!(version_number("not a version"), None);
    }
}
