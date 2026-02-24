use async_trait::async_trait;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AudioError {
    #[error("Download failed: {0}")]
    DownloadError(String),

    #[error("Processing failef: {0}")]
    ProcessingError(String),
}

#[derive(Debug, Clone, Copy)]
pub enum AudioPreset {
    CarBass,
    PureHiFi,
    ExtremeLow,
    Surround8D,
}

pub struct AudioMetadata {
    pub title: String,
    pub artist: String,
    pub thumbnail_url: Option<String>,
    pub duration: u64,
}

#[async_trait]
pub trait AudioService: Send + Sync {
    async fn process_track(
        &self,
        url: &str,
        preset: AudioPreset,
    ) -> Result<(PathBuf, AudioMetadata), AudioError>;
}
