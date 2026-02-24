#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::audio_service::{AudioError, AudioService};
    use async_trait::async_trait;
    use std::path::PathBuf;

    struct MockAudioService;
    #[async_trait]
    impl AudioService for MockAudioService {
        async fn process_track(&self, _url: &str) -> Result<PathBuf, AudioError> {
            Ok(PathBuf::from("test_output.mp3"))
        }
    }

    #[tokio::test]
    async fn test_logic_flow() {
        let service = MockAudioService;
        let result = service.process_track("https://youtube.com...").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().to_str().unwrap(), "test_output.mp3");
    }
}
