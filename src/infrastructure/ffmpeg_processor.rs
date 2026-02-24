use crate::domain::audio_service::{AudioError, AudioMetadata, AudioPreset, AudioService};
use async_trait::async_trait;
use id3::{Tag, TagLike, Version};
use std::path::PathBuf;
use tokio::process::Command;
use uuid::Uuid;

pub struct FFmpegProcessor;

#[async_trait]
impl AudioService for FFmpegProcessor {
    async fn process_track(
        &self,
        url: &str,
        preset: AudioPreset,
    ) -> Result<(PathBuf, AudioMetadata), AudioError> {
        let id = Uuid::new_v4().to_string();
        let input = format!("{}_in.mp3", id);
        let output = format!("{}_out.mp3", id);

        // 1. Получаем метаданные через yt-dlp перед скачиванием
        let info_output = Command::new("yt-dlp")
            .args([
                "--print",
                "%(title)s|%(uploader)s|%(thumbnail)s",
                "--no-warnings",
                url,
            ])
            .output()
            .await
            .map_err(|e| AudioError::DownloadError(e.to_string()))?;

        let info_str = String::from_utf8_lossy(&info_output.stdout);
        let parts: Vec<&str> = info_str.split('|').collect();

        let metadata = AudioMetadata {
            title: parts.get(0).unwrap_or(&"Unknown Track").trim().to_string(),
            artist: parts.get(1).unwrap_or(&"Unknown Artist").trim().to_string(),
            thumbnail_url: parts.get(2).map(|s| s.trim().to_string()),
        };

        // 2. Скачивание
        Command::new("yt-dlp")
            .args(["-x", "--audio-format", "mp3", "-o", &input, url])
            .status()
            .await
            .map_err(|e| AudioError::DownloadError(e.to_string()))?;

        let filter = match preset {
            AudioPreset::CarBass => "loudnorm=I=-14:TP=-1.5:LRA=11,bass=g=3,treble=g=1",
            AudioPreset::PureHiFi => "loudnorm=I=-16:TP=-1.5:LRA=11",
            AudioPreset::ExtremeLow => "loudnorm=I=-12:TP=-1.0:LRA=11,bass=g=6,treble=g=2",
        };

        // 3. Обработка FFmpeg
        Command::new("ffmpeg")
            .args([
                "-i",
                &input,
                "-nostdin",
                "-loglevel",
                "error",
                "-af",
                filter,
                "-b:a",
                "320k",
                "-y",
                &output,
            ])
            .status()
            .await
            .map_err(|e| AudioError::ProcessingError(e.to_string()))?;

        let _ = tokio::fs::remove_file(&input).await;

        // 4. Вшиваем ID3 теги и Обложку
        let mut tag = Tag::new();
        tag.set_title(&metadata.title);
        tag.set_artist(&metadata.artist);

        if let Some(thumb_url) = &metadata.thumbnail_url {
            // Используем асинхронный клиент
            let client = reqwest::Client::new();
            if let Ok(resp) = client.get(thumb_url).send().await {
                if let Ok(bytes) = resp.bytes().await {
                    // Библиотека id3 работает синхронно, это ок,
                    // так как мы уже получили данные (bytes) асинхронно
                    tag.add_frame(id3::frame::Picture {
                        mime_type: "image/jpeg".to_string(),
                        picture_type: id3::frame::PictureType::CoverFront,
                        description: "Cover".to_string(),
                        data: bytes.to_vec(),
                    });
                }
            }
        }

        let _ = tag.write_to_path(&output, Version::Id3v24);

        Ok((PathBuf::from(output), metadata))
    }
}
