mod domain;
mod infrastructure;

use crate::domain::audio_service::AudioPreset;
use crate::domain::audio_service::AudioService;
use crate::infrastructure::ffmpeg_processor::FFmpegProcessor;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup};
use tokio::sync::Semaphore;

fn make_keyboard(url: &str) -> InlineKeyboardMarkup {
    let buttons = [
        [InlineKeyboardButton::callback(
            "🏎 Car Bass",
            format!("bass|{}", url),
        )],
        [InlineKeyboardButton::callback(
            "🎧 Pure Hi-Fi",
            format!("hifi|{}", url),
        )],
        [InlineKeyboardButton::callback(
            "🔥 Extreme Low",
            format!("extreme|{}", url),
        )],
    ];
    InlineKeyboardMarkup::new(buttons)
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let semaphore = Arc::new(Semaphore::new(3));
    let audio_service: Arc<dyn AudioService> = Arc::new(FFmpegProcessor);
    let bot = Bot::from_env();

    // Создаем дерево обработчиков
    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![audio_service, semaphore])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

// 1. Когда прислали ссылку — показываем кнопки
async fn handle_message(bot: Bot, msg: Message) -> ResponseResult<()> {
    if let Some(url) = msg.text().filter(|t| t.contains("youtu")) {
        bot.send_message(msg.chat.id, "Выбери режим прокачки:")
            .reply_markup(make_keyboard(url))
            .await?;
    }
    Ok(())
}

// 2. Когда нажали кнопку — качаем с нужным пресетом
async fn handle_callback(
    bot: Bot,
    q: CallbackQuery,
    service: Arc<dyn AudioService>,
    semaphore: Arc<Semaphore>,
) -> ResponseResult<()> {
    if let Some(data) = q.data {
        let parts: Vec<&str> = data.split('|').collect();
        let preset_raw = parts[0];
        let url = parts[1];

        let preset = match preset_raw {
            "bass" => AudioPreset::CarBass,
            "hifi" => AudioPreset::PureHiFi,
            "extreme" => AudioPreset::ExtremeLow,
            _ => return Ok(()),
        };

        // Используем методы .chat() и .id() для MaybeInaccessibleMessage
        if let Some(msg) = q.message {
            let _permit = semaphore.acquire().await.unwrap();

            bot.edit_message_text(msg.chat().id, msg.id(), "⏳ В очереди... Скоро начнем!")
                .await?;
            let chat_id = msg.chat().id;
            let message_id = msg.id();

            let _ = bot.answer_callback_query(q.id).await;
            let _ = bot
                .edit_message_text(
                    chat_id,
                    message_id,
                    "🏎 Запускаю двигатели... Процесс пошел!",
                )
                .await;

            match service.process_track(url, preset).await {
                Ok((path, meta)) => {
                    let file = teloxide::types::InputFile::file(&path)
                        .file_name(format!("{}.mp3", meta.title));
                    let _ = bot
                        .send_audio(chat_id, file)
                        .caption(format!("✅ Готово!\n🎵 {}\n👤 {}", meta.title, meta.artist))
                        .await;
                    let _ = tokio::fs::remove_file(path).await;
                }
                Err(e) => {
                    let _ = bot.send_message(chat_id, format!("❌ Ошибка: {}", e)).await;
                }
            }
        }
    }
    Ok(())
}
