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

        // 1. Получаем расширенные метаданные (Title|Uploader|Thumbnail|Duration)
        let info_output = Command::new("yt-dlp")
            .args([
                "--print",
                "%(title)s|%(uploader)s|%(thumbnail)s|%(duration)s",
                "--no-warnings",
                url,
            ])
            .output()
            .await
            .map_err(|e| AudioError::DownloadError(e.to_string()))?;

        let info_str = String::from_utf8_lossy(&info_output.stdout);
        let parts: Vec<&str> = info_str.split('|').collect();

        // Парсим длительность в секундах
        let duration: u64 = parts.get(3).unwrap_or(&"0").trim().parse().unwrap_or(0);

        // ПРОВЕРКА ДЛИТЕЛЬНОСТИ (Лимит 45 минут = 2700 секунд)
        // Это важно, чтобы файл не превысил лимит Telegram в 50МБ при 320kbps
        if duration > 2700 {
            return Err(AudioError::DownloadError(
                "Видео слишком длинное (макс. 45 мин). Telegram не примет такой тяжелый файл!"
                    .into(),
            ));
        }

        let metadata = AudioMetadata {
            title: clean_title(parts.get(0).unwrap_or(&"Unknown Track")),
            artist: parts.get(1).unwrap_or(&"Unknown Artist").trim().to_string(),
            thumbnail_url: parts.get(2).map(|s| s.trim().to_string()),
            duration, // Новое поле
        };

        // 2. Скачивание (Audio Only)
        let dl_status = Command::new("yt-dlp")
            .args(["-x", "--audio-format", "mp3", "-o", &input, url])
            .status()
            .await
            .map_err(|e| AudioError::DownloadError(e.to_string()))?;

        if !dl_status.success() {
            return Err(AudioError::DownloadError(
                "Не удалось скачать аудио с YouTube".into(),
            ));
        }

        // 3. Выбор фильтра в зависимости от пресета
        let filter = match preset {
            AudioPreset::CarBass => "loudnorm=I=-14:TP=-1.5:LRA=11,bass=g=3,treble=g=1",
            AudioPreset::PureHiFi => "loudnorm=I=-16:TP=-1.5:LRA=11",
            AudioPreset::ExtremeLow => "loudnorm=I=-12:TP=-1.0:LRA=11,bass=g=6,treble=g=2",
        };

        // 4. Обработка FFmpeg
        let ff_status = Command::new("ffmpeg")
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

        if !ff_status.success() {
            return Err(AudioError::ProcessingError(
                "Ошибка при обработке звука в FFmpeg".into(),
            ));
        }

        // 5. Вшиваем ID3 теги и Обложку (асинхронно скачиваем картинку)
        let mut tag = Tag::new();
        tag.set_title(&metadata.title);
        tag.set_artist(&metadata.artist);

        if let Some(thumb_url) = &metadata.thumbnail_url {
            let client = reqwest::Client::new();
            if let Ok(resp) = client.get(thumb_url).send().await {
                if let Ok(bytes) = resp.bytes().await {
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

// Функция для очистки названий от мусора YouTube
fn clean_title(title: &str) -> String {
    title
        .replace("(Official Video)", "")
        .replace("(Official Audio)", "")
        .replace("(Official Music Video)", "")
        .replace("[Official Video]", "")
        .replace("[HQ]", "")
        .replace("(Lyric Video)", "")
        .replace("[Lyrics]", "")
        .replace("(High Quality)", "")
        .replace("4K", "")
        .replace("8K", "")
        .trim()
        .to_string()
}
