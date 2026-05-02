use crate::domain::user_repository::UserRepository;
use async_trait::async_trait;
use sqlx::{Row, SqlitePool};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub struct SqliteUserRepo {
    pub pool: SqlitePool,
}

impl SqliteUserRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn ensure_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS users (
                user_id INTEGER PRIMARY KEY,
                balance INTEGER NOT NULL DEFAULT 3,
                referrer_by INTEGER
            )",
        )
        .execute(pool)
        .await?;

        let columns = sqlx::query("PRAGMA table_info(users)")
            .fetch_all(pool)
            .await?;

        let has_referrer = columns
            .iter()
            .any(|row| row.get::<String, _>("name") == "referrer_by");

        if !has_referrer {
            sqlx::query("ALTER TABLE users ADD COLUMN referrer_by INTEGER")
                .execute(pool)
                .await?;
        }

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS track_requests (
                id TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL,
                url TEXT NOT NULL,
                created_at INTEGER NOT NULL
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_track_requests_user_id ON track_requests(user_id)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_track_requests_created_at ON track_requests(created_at)",
        )
        .execute(pool)
        .await?;

        Ok(())
    }
}

#[async_trait]
impl UserRepository for SqliteUserRepo {
    async fn get_balance(&self, user_id: i64) -> i32 {
        let row = sqlx::query("SELECT balance FROM users WHERE user_id = ?")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await;

        match row {
            Ok(Some(record)) => record.get::<i64, _>(0) as i32,
            _ => {
                let _ = sqlx::query("INSERT OR IGNORE INTO users (user_id, balance) VALUES (?, 3)")
                    .bind(user_id)
                    .execute(&self.pool)
                    .await;
                3
            }
        }
    }

    async fn use_credit(&self, user_id: i64) -> bool {
        let result =
            sqlx::query("UPDATE users SET balance = balance - 1 WHERE user_id = ? AND balance > 0")
                .bind(user_id)
                .execute(&self.pool)
                .await;

        match result {
            Ok(res) => res.rows_affected() > 0,
            Err(_) => false,
        }
    }

    async fn add_balance(&self, user_id: i64, amount: i32) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO users (user_id, balance) VALUES (?, ?)
             ON CONFLICT(user_id) DO UPDATE SET balance = balance + excluded.balance",
        )
        .bind(user_id)
        .bind(amount)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn register_referral(&self, target_id: i64, inviter_id: i64) -> bool {
        if target_id == inviter_id {
            return false;
        }

        let mut tx = match self.pool.begin().await {
            Ok(tx) => tx,
            Err(_) => return false,
        };

        let target_exists = sqlx::query("SELECT 1 FROM users WHERE user_id = ?")
            .bind(target_id)
            .fetch_optional(&mut *tx)
            .await
            .ok()
            .flatten()
            .is_some();

        if target_exists {
            let _ = tx.rollback().await;
            return false;
        }

        let inviter_exists = sqlx::query("SELECT 1 FROM users WHERE user_id = ?")
            .bind(inviter_id)
            .fetch_optional(&mut *tx)
            .await
            .ok()
            .flatten()
            .is_some();

        if !inviter_exists {
            let _ = tx.rollback().await;
            return false;
        }

        let inserted =
            sqlx::query("INSERT INTO users (user_id, balance, referrer_by) VALUES (?, 3, ?)")
                .bind(target_id)
                .bind(inviter_id)
                .execute(&mut *tx)
                .await
                .map(|res| res.rows_affected() == 1)
                .unwrap_or(false);

        if !inserted {
            let _ = tx.rollback().await;
            return false;
        }

        let credited = sqlx::query("UPDATE users SET balance = balance + 2 WHERE user_id = ?")
            .bind(inviter_id)
            .execute(&mut *tx)
            .await
            .map(|res| res.rows_affected() == 1)
            .unwrap_or(false);

        if !credited {
            let _ = tx.rollback().await;
            return false;
        }

        tx.commit().await.is_ok()
    }

    async fn save_track_request(&self, user_id: i64, url: &str) -> Result<String, sqlx::Error> {
        let id = Uuid::new_v4().to_string();
        let now = unix_timestamp();
        let cutoff = now - 86_400;

        sqlx::query("DELETE FROM track_requests WHERE created_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "INSERT INTO track_requests (id, user_id, url, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(url)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    async fn get_track_request(
        &self,
        user_id: i64,
        request_id: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        let row = sqlx::query("SELECT url FROM track_requests WHERE id = ? AND user_id = ?")
            .bind(request_id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(|record| record.get::<String, _>(0)))
    }
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::user_repository::UserRepository;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_repo() -> SqliteUserRepo {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        SqliteUserRepo::ensure_schema(&pool).await.unwrap();
        SqliteUserRepo::new(pool)
    }

    #[tokio::test]
    async fn creates_new_users_with_three_credits() {
        let repo = test_repo().await;

        assert_eq!(repo.get_balance(1).await, 3);
        assert_eq!(repo.get_balance(1).await, 3);
    }

    #[tokio::test]
    async fn add_balance_upserts_users() {
        let repo = test_repo().await;

        repo.add_balance(10, 5).await.unwrap();
        assert_eq!(repo.get_balance(10).await, 5);
    }

    #[tokio::test]
    async fn referral_only_applies_to_new_users() {
        let repo = test_repo().await;

        assert_eq!(repo.get_balance(100).await, 3);
        assert!(repo.register_referral(200, 100).await);
        assert_eq!(repo.get_balance(200).await, 3);
        assert_eq!(repo.get_balance(100).await, 5);
        assert!(!repo.register_referral(200, 100).await);
    }

    #[tokio::test]
    async fn stores_track_requests_by_user() {
        let repo = test_repo().await;

        let request_id = repo
            .save_track_request(1, "https://youtube.com/watch?v=test")
            .await
            .unwrap();

        assert_eq!(
            repo.get_track_request(1, &request_id)
                .await
                .unwrap()
                .as_deref(),
            Some("https://youtube.com/watch?v=test")
        );
        assert!(
            repo.get_track_request(2, &request_id)
                .await
                .unwrap()
                .is_none()
        );
    }
}
