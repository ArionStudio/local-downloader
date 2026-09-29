use crate::download::{Job, JobLog, JobStatus, SiteKind};
use chrono::Utc;
use rusqlite::{params, Connection, Row};
use serde_json::Value;
use std::{fs, path::Path, sync::Mutex};

pub struct Storage {
    conn: Mutex<Connection>,
}

impl Storage {
    pub fn new(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        let conn = Connection::open(path).map_err(|error| error.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        let storage = Self {
            conn: Mutex::new(conn),
        };
        storage.init()?;
        Ok(storage)
    }

    pub fn list_jobs(&self) -> Result<Vec<Job>, String> {
        let conn = self.conn.lock().map_err(|error| error.to_string())?;
        let mut statement = conn
      .prepare(
        "SELECT id, created_at, updated_at, status, site, preset_id, source_url, output_path, progress, phase, speed, eta, error_message, ready_paths_json
         FROM jobs
         ORDER BY updated_at DESC",
      )
      .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map([], row_to_job)
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn recover_interrupted_jobs(&self) -> Result<usize, String> {
        let mut conn = self.conn.lock().map_err(|error| error.to_string())?;
        let transaction = conn.transaction().map_err(|error| error.to_string())?;
        let job_ids = {
            let mut statement = transaction
                .prepare(
                    "SELECT id
                     FROM jobs
                     WHERE status IN ('queued', 'resolving', 'downloading', 'postprocessing')",
                )
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        };

        if job_ids.is_empty() {
            return Ok(0);
        }

        let now = Utc::now().to_rfc3339();
        for job_id in &job_ids {
            transaction
                .execute(
                    "INSERT INTO job_logs (job_id, created_at, level, message)
                     VALUES (?1, ?2, 'warn', 'App restarted before this job finished; marked as interrupted.')",
                    params![job_id, &now],
                )
                .map_err(|error| error.to_string())?;
        }

        transaction
            .execute(
                "UPDATE jobs
                 SET updated_at = ?1,
                     status = 'canceled',
                     phase = 'Interrupted',
                     speed = NULL,
                     eta = NULL,
                     error_message = NULL
                 WHERE status IN ('queued', 'resolving', 'downloading', 'postprocessing')",
                params![&now],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;

        Ok(job_ids.len())
    }

    pub fn get_job(&self, id: &str) -> Result<Option<Job>, String> {
        let conn = self.conn.lock().map_err(|error| error.to_string())?;
        let result = conn.query_row(
      "SELECT id, created_at, updated_at, status, site, preset_id, source_url, output_path, progress, phase, speed, eta, error_message, ready_paths_json
       FROM jobs
       WHERE id = ?1",
      params![id],
      row_to_job,
    );

        match result {
            Ok(job) => Ok(Some(job)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn upsert_job(&self, job: &Job) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|error| error.to_string())?;
        conn
      .execute(
        "INSERT INTO jobs (id, created_at, updated_at, status, site, preset_id, source_url, output_path, progress, phase, speed, eta, error_message, ready_paths_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT(id) DO UPDATE SET
           updated_at = excluded.updated_at,
           status = excluded.status,
           output_path = excluded.output_path,
           progress = excluded.progress,
           phase = excluded.phase,
           speed = excluded.speed,
           eta = excluded.eta,
           error_message = excluded.error_message,
           ready_paths_json = excluded.ready_paths_json",
        params![
          &job.id,
          &job.created_at,
          &job.updated_at,
          job.status.as_str(),
          job.site.as_str(),
          &job.preset_id,
          &job.source_url,
          job.output_path.as_deref(),
          job.progress,
          &job.phase,
          job.speed.as_deref(),
          job.eta.as_deref(),
          job.error_message.as_deref(),
          serde_json::to_string(&job.ready_paths).map_err(|e| e.to_string())?,
        ],
      )
      .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn append_log(&self, job_id: &str, level: &str, message: &str) -> Result<JobLog, String> {
        let created_at = Utc::now().to_rfc3339();
        let conn = self.conn.lock().map_err(|error| error.to_string())?;
        conn.execute(
            "INSERT INTO job_logs (job_id, created_at, level, message) VALUES (?1, ?2, ?3, ?4)",
            params![job_id, created_at, level, message],
        )
        .map_err(|error| error.to_string())?;

        Ok(JobLog {
            id: conn.last_insert_rowid(),
            job_id: job_id.to_string(),
            created_at,
            level: level.to_string(),
            message: message.to_string(),
        })
    }

    pub fn logs_for_job(&self, job_id: &str) -> Result<Vec<JobLog>, String> {
        let conn = self.conn.lock().map_err(|error| error.to_string())?;
        let mut statement = conn
            .prepare(
                "SELECT id, job_id, created_at, level, message
         FROM job_logs
         WHERE job_id = ?1
         ORDER BY id ASC",
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map(params![job_id], |row| {
                Ok(JobLog {
                    id: row.get(0)?,
                    job_id: row.get(1)?,
                    created_at: row.get(2)?,
                    level: row.get(3)?,
                    message: row.get(4)?,
                })
            })
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn get_json(&self, key: &str) -> Result<Option<Value>, String> {
        let conn = self.conn.lock().map_err(|error| error.to_string())?;
        let result = conn.query_row(
            "SELECT value_json FROM settings WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        );

        match result {
            Ok(raw) => serde_json::from_str(&raw)
                .map(Some)
                .map_err(|error| error.to_string()),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn set_json(&self, key: &str, value: &Value) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|error| error.to_string())?;
        let raw = serde_json::to_string(value).map_err(|error| error.to_string())?;
        conn.execute(
            "INSERT INTO settings (key, value_json)
             VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
            params![key, raw],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn init(&self) -> Result<(), String> {
        let mut connection = self.conn.lock().map_err(|error| error.to_string())?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let conn = &transaction;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS jobs (
           id TEXT PRIMARY KEY,
           created_at TEXT NOT NULL,
           updated_at TEXT NOT NULL,
           status TEXT NOT NULL,
           site TEXT NOT NULL,
           preset_id TEXT NOT NULL,
           source_url TEXT NOT NULL,
           output_path TEXT,
           progress REAL NOT NULL,
           phase TEXT NOT NULL,
           speed TEXT,
           eta TEXT,
           error_message TEXT
         );

         CREATE TABLE IF NOT EXISTS job_logs (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           job_id TEXT NOT NULL,
           created_at TEXT NOT NULL,
           level TEXT NOT NULL,
           message TEXT NOT NULL
         );

         CREATE INDEX IF NOT EXISTS idx_job_logs_job_id ON job_logs(job_id);",
        )
        .map_err(|error| error.to_string())?;
        let has_ready_paths = conn
            .prepare("PRAGMA table_info(jobs)")
            .map_err(|e| e.to_string())?
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|name| name == "ready_paths_json");
        if !has_ready_paths {
            conn.execute(
                "ALTER TABLE jobs ADD COLUMN ready_paths_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )
            .map_err(|e| e.to_string())?;
            let mut query = conn
                .prepare("SELECT id, output_path FROM jobs")
                .map_err(|e| e.to_string())?;
            let jobs = query
                .query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            for (id, last_path) in jobs {
                let mut paths = Vec::new();
                let mut logs = conn
                    .prepare("SELECT message FROM job_logs WHERE job_id = ?1 ORDER BY id")
                    .map_err(|e| e.to_string())?;
                for message in logs
                    .query_map([&id], |r| r.get::<_, String>(0))
                    .map_err(|e| e.to_string())?
                {
                    if let Some(path) = crate::download::engine::parse_output_path(
                        &message.map_err(|e| e.to_string())?,
                    ) {
                        paths.push(path);
                    }
                }
                paths.extend(last_path);
                paths.retain(|p| Path::new(p).is_file() && !p.ends_with(".part"));
                paths.sort();
                paths.dedup();
                conn.execute(
                    "UPDATE jobs SET ready_paths_json = ?1 WHERE id = ?2",
                    params![
                        serde_json::to_string(&paths).map_err(|e| e.to_string())?,
                        id
                    ],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (
               key TEXT PRIMARY KEY,
               value_json TEXT NOT NULL
             );",
        )
        .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(())
    }
}

fn row_to_job(row: &Row<'_>) -> rusqlite::Result<Job> {
    let status: String = row.get(3)?;
    let site: String = row.get(4)?;

    Ok(Job {
        id: row.get(0)?,
        created_at: row.get(1)?,
        updated_at: row.get(2)?,
        status: JobStatus::from_str(&status),
        site: SiteKind::from_str(&site),
        preset_id: row.get(5)?,
        source_url: row.get(6)?,
        output_path: row.get(7)?,
        progress: row.get(8)?,
        phase: row.get(9)?,
        speed: row.get(10)?,
        eta: row.get(11)?,
        error_message: row.get(12)?,
        ready_paths: serde_json::from_str(&row.get::<_, String>(13)?).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migrates_finished_files_from_canceled_legacy_jobs() {
        let root =
            std::env::temp_dir().join(format!("downloader-migration-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let db = root.join("jobs.sqlite");
        drop(Storage::new(&db).unwrap());
        let first = root.join("first.mp4");
        let second = root.join("second.mp4");
        let partial = root.join("third.mp4.part");
        for file in [&first, &second, &partial] {
            fs::write(file, b"fixture").unwrap();
        }
        let conn = Connection::open(&db).unwrap();
        conn.execute("ALTER TABLE jobs DROP COLUMN ready_paths_json", [])
            .unwrap();
        conn.execute("INSERT INTO jobs (id,created_at,updated_at,status,site,preset_id,source_url,output_path,progress,phase) VALUES ('legacy','2026-09-29','2026-09-29','canceled','generic','generic','https://example.com',?1,66,'Canceled')", [partial.to_str().unwrap()]).unwrap();
        for path in [&first, &second, &partial, &root.join("missing.mp4")] {
            conn.execute("INSERT INTO job_logs (job_id,created_at,level,message) VALUES ('legacy','2026-09-29','info',?1)", [format!("[download] Destination: {}", path.display())]).unwrap();
        }
        drop(conn);
        for _ in 0..2 {
            let storage = Storage::new(&db).unwrap();
            let job = storage.get_job("legacy").unwrap().unwrap();
            assert!(matches!(job.status, JobStatus::Canceled));
            assert_eq!(
                job.ready_paths,
                [first.display().to_string(), second.display().to_string()]
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ready_files_survive_cancellation_and_reopening() {
        let root = std::env::temp_dir().join(format!("downloader-ready-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let db = root.join("jobs.sqlite");
        let storage = Storage::new(&db).unwrap();
        let mut job = Job {
            id: "batch".into(),
            created_at: "2026-09-29".into(),
            updated_at: "2026-09-29".into(),
            status: JobStatus::Downloading,
            site: SiteKind::Generic,
            preset_id: "generic".into(),
            source_url: "https://example.com".into(),
            output_path: Some("unfinished.mp4".into()),
            ready_paths: vec!["first.mp4".into(), "second.mp4".into()],
            progress: 42.0,
            phase: "Downloading".into(),
            speed: None,
            eta: None,
            error_message: None,
        };
        storage.upsert_job(&job).unwrap();
        job.status = JobStatus::Canceled;
        storage.upsert_job(&job).unwrap();
        drop(storage);
        let reopened = Storage::new(&db).unwrap();
        let saved = reopened.get_job("batch").unwrap().unwrap();
        assert!(matches!(saved.status, JobStatus::Canceled));
        assert_eq!(saved.ready_paths, ["first.mp4", "second.mp4"]);
        assert_eq!(
            reopened.list_jobs().unwrap()[0].ready_paths,
            saved.ready_paths
        );
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
}
