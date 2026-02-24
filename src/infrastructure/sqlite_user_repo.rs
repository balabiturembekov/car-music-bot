use crate::domain::user_repository::UserRepository;
use async_trait::async_trait;
use sqlx::{Row, SqlitePool};

pub struct SqliteUserRepo {
    pub pool: SqlitePool,
}

impl SqliteUserRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserRepository for SqliteUserRepo {
    async fn get_balance(&self, user_id: i64) -> i32 {
        // ИСПОЛЬЗУЕМ self.pool
        let row = sqlx::query("SELECT balance FROM users WHERE user_id = ?")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await;

        match row {
            Ok(Some(record)) => record.get::<i64, _>(0) as i32,
            _ => {
                // ИСПОЛЬЗУЕМ self.pool для вставки нового юзера
                let _ = sqlx::query("INSERT OR IGNORE INTO users (user_id, balance) VALUES (?, 3)")
                    .bind(user_id)
                    .execute(&self.pool)
                    .await;
                3
            }
        }
    }

    async fn use_credit(&self, user_id: i64) -> bool {
        // ИСПОЛЬЗУЕМ self.pool для обновления
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
        // ИСПОЛЬЗУЕМ self.pool для пополнения
        sqlx::query("UPDATE users SET balance = balance + ? WHERE user_id = ?")
            .bind(amount)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn register_referral(&self, target_id: i64, inviter_id: i64) -> bool {
        let exists = sqlx::query("SELECT 1 FROM users WHERE user_id = ?")
            .bind(target_id)
            .fetch_optional(&self.pool)
            .await;

        if let Ok(None) = exists {
            let _ =
                sqlx::query("INSERT INTO users (user_id, balance, referrer_by) VALUES (?, 3, ?)")
                    .bind(target_id)
                    .bind(inviter_id)
                    .execute(&self.pool)
                    .await;

            let _ = sqlx::query("UPDATE users SET balance = balance + 2 WHERE user_id = ?")
                .bind(inviter_id)
                .execute(&self.pool)
                .await;

            true
        } else {
            false
        }
    }
}
