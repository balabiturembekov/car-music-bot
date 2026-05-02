use async_trait::async_trait;

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn get_balance(&self, user_id: i64) -> i32;

    async fn use_credit(&self, user_id: i64) -> bool;

    async fn add_balance(&self, user_id: i64, amount: i32) -> Result<(), sqlx::Error>;

    async fn register_referral(&self, target_id: i64, inviter_id: i64) -> bool;

    async fn save_track_request(&self, user_id: i64, url: &str) -> Result<String, sqlx::Error>;

    async fn get_track_request(
        &self,
        user_id: i64,
        request_id: &str,
    ) -> Result<Option<String>, sqlx::Error>;
}
