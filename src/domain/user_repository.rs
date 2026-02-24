use async_trait::async_trait;

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn get_balance(&self, user_id: i64) -> i32;

    async fn use_credit(&self, user_id: i64) -> bool;

    async fn add_balance(&self, user_id: i64, amount: i32) -> Result<(), sqlx::Error>;
}
