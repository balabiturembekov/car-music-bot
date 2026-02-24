#[cfg(test)]
mod tests {
    use tokio::process::Command;

    #[tokio::test]
    async fn test_tools_installed() {
        // Проверяем, что ffmpeg доступен на твоем Mac
        let ffmpeg = Command::new("ffmpeg").arg("-version").status().await;
        assert!(ffmpeg.is_ok(), "FFmpeg должен быть установлен!");

        // Проверяем yt-dlp
        let ytdlp = Command::new("yt-dlp").arg("--version").status().await;
        assert!(ytdlp.is_ok(), "yt-dlp должен быть установлен!");
    }
}
