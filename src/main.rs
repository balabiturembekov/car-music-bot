mod domain;
mod infrastructure;

use crate::domain::audio_service::{AudioPreset, AudioService};
use crate::domain::user_repository::UserRepository;
use crate::infrastructure::ffmpeg_processor::FFmpegProcessor;
use crate::infrastructure::sqlite_user_repo::SqliteUserRepo;
use sqlx::sqlite::SqlitePoolOptions;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::BotCommand;
use teloxide::types::{
    CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, LabeledPrice, PreCheckoutQuery,
};

use tokio::sync::Semaphore;

// Клавиатура выбора режима
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

// Клавиатура оплаты
fn make_payment_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new([[InlineKeyboardButton::callback(
        "💳 Купить 10 треков (50 ⭐️)",
        "buy_10_credits",
    )]])
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    pretty_env_logger::init();

    // 1. Инициализация БД (SQLite)
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect("sqlite:users.db?mode=rwc")
        .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (user_id INTEGER PRIMARY KEY, balance INTEGER DEFAULT 3)",
    )
    .execute(&pool)
    .await?;

    // 2. Инициализация сервисов (DI)
    let semaphore = Arc::new(Semaphore::new(3));
    let audio_service: Arc<dyn AudioService> = Arc::new(FFmpegProcessor);
    let user_repo: Arc<dyn UserRepository> = Arc::new(SqliteUserRepo::new(pool));

    let bot = Bot::from_env();

    // 3. Дерево обработчиков
    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter(|msg: Message| {
                    msg.text()
                        .map_or(false, |t| t == "/profile" || t == "/start")
                })
                .endpoint(handle_profile),
        )
        .branch(
            Update::filter_message()
                .filter(|msg: Message| msg.successful_payment().is_some())
                .endpoint(handle_successful_payment),
        )
        .branch(Update::filter_pre_checkout_query().endpoint(handle_pre_checkout))
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback));

    let commands = vec![
        BotCommand::new("start", "🚀 Запустить бота"),
        BotCommand::new("profile", "👤 Мой баланс и ID"),
        BotCommand::new("help", "❓ Как пользоваться"),
    ];

    bot.set_my_commands(commands).await?;
    log::info!("🚀 Команды зарегистрированы, бот запущен!");

    log::info!("🚀 Бот DeepDrive AI запущен!");

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![audio_service, semaphore, user_repo])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

async fn handle_message(
    bot: Bot,
    msg: Message,
    repo: Arc<dyn UserRepository>,
) -> ResponseResult<()> {
    if let Some(text) = msg.text() {
        // Проверяем, является ли текст ссылкой на YouTube (обычная, мобильная или Shorts)
        if text.contains("://youtube.com") || 
           text.contains("youtu.be/") || 
           text.contains("://youtube.com") 
        {
            let user_id = msg.chat.id.0;
            let balance = repo.get_balance(user_id).await;

            // Формируем текст в зависимости от типа ссылки
            let msg_text = if text.contains("shorts") {
                format!(
                    "🎬 <b>О, это Shorts!</b> Сейчас вытяну из него полный звук.\n\n💳 Твой баланс: <b>{}</b> кредитов.\nВыбери режим прокачки:", 
                    balance
                )
            } else {
                format!(
                    "💳 Твой баланс: <b>{}</b> кредитов.\n\nВыбери режим прокачки для этого видео:", 
                    balance
                )
            };

            // Отправляем сообщение с клавиатурой
            bot.send_message(msg.chat.id, msg_text)
                .parse_mode(teloxide::types::ParseMode::Html)
                .reply_markup(make_keyboard(text)) // Используем 'text' как URL для кнопок
                .await?;
        } 
        // Если это не ссылка и не команда (команды обрабатываются в дереве выше), 
        // можно добавить подсказку:
        else if !text.starts_with('/') {
            bot.send_message(
                msg.chat.id, 
                "📥 Пришли мне ссылку на <b>YouTube</b> видео или <b>Shorts</b>, и я прокачаю звук для твоей машины! 🏎💨"
            )
            .parse_mode(teloxide::types::ParseMode::Html)
            .await?;
        }
    }
    Ok(())
}

async fn handle_callback(
    bot: Bot,
    q: CallbackQuery,
    service: Arc<dyn AudioService>,
    repo: Arc<dyn UserRepository>,
    semaphore: Arc<Semaphore>,
) -> ResponseResult<()> {
    let user_id = q.from.id.0 as i64;
    let chat_id = q.message.as_ref().map(|m| m.chat().id).unwrap_or(q.from.id.into());

    if let Some(data) = q.data {
        // 1. ОБРАБОТКА НАЖАТИЯ КНОПКИ ОПЛАТЫ
        if data == "buy_10_credits" {
            let _ = bot.answer_callback_query(q.id).await;
            handle_buy_credits(bot, chat_id).await?;
            return Ok(());
        }

        // 2. РАЗБОР ДАННЫХ ПРЕСЕТА (формат "preset|url")
        let parts: Vec<&str> = data.split('|').collect();
        if parts.len() < 2 {
            return Ok(());
        }

        let preset_raw = parts[0];
        let url = parts[1];

        let preset = match preset_raw {
            "bass" => AudioPreset::CarBass,
            "hifi" => AudioPreset::PureHiFi,
            "extreme" => AudioPreset::ExtremeLow,
            _ => return Ok(()),
        };

        // 3. ПРОВЕРКА БАЛАНСА
        if !repo.use_credit(user_id).await {
            let _ = bot.answer_callback_query(q.id).await;
            bot.send_message(
                chat_id,
                "⚠️ У тебя 0 кредитов. Пополни баланс для продолжения! ⭐️",
            )
            .reply_markup(make_payment_keyboard())
            .await?;
            return Ok(());
        }

        // 4. ЗАПУСК ОБРАБОТКИ
        if let Some(msg) = q.message {
            // Ограничиваем количество одновременных задач
            let _permit = semaphore.acquire().await.unwrap();
            let _ = bot.answer_callback_query(q.id).await;

            // Уведомляем пользователя о начале
            let _ = bot.edit_message_text(chat_id, msg.id(), "🏎 Запускаю двигатели... Процесс пошел!")
                .await;

            match service.process_track(url, preset).await {
                Ok((path, meta)) => {
                    // Форматируем время: 04:20
                    let mins = meta.duration / 60;
                    let secs = meta.duration % 60;
                    let duration_str = format!("{:02}:{:02}", mins, secs);

                    let file = teloxide::types::InputFile::file(&path)
                        .file_name(format!("{}.mp3", meta.title));

                    // Отправляем готовое аудио
                    let _ = bot.send_audio(chat_id, file)
                        .caption(format!(
                            "✅ <b>Готово для авто!</b>\n\n🎵 {}\n👤 {}\n⏱ Длительность: <code>{}</code>", 
                            meta.title, meta.artist, duration_str
                        ))
                        .parse_mode(teloxide::types::ParseMode::Html)
                        .await;

                    // Удаляем временный файл
                    let _ = tokio::fs::remove_file(path).await;
                }
                Err(e) => {
                    // Если произошла ошибка (например, видео > 45 мин)
                    let _ = bot.send_message(chat_id, format!("❌ Ошибка: {}", e)).await;
                    
                    // Возвращаем кредит пользователю, так как услуга не оказана
                    let _ = repo.add_balance(user_id, 1).await;
                }
            }
        }
    }
    Ok(())
}

async fn handle_buy_credits(bot: Bot, chat_id: ChatId) -> ResponseResult<()> {
    bot.send_invoice(
        chat_id,
        "10 Премиум-загрузок",
        "Добавляет 10 кредитов для прокачки музыки в 320kbps",
        "payload_10_credits",
        "XTR",
        vec![LabeledPrice::new("10 кредитов", 50)],
    )
    .await?;
    Ok(())
}

async fn handle_pre_checkout(bot: Bot, q: PreCheckoutQuery) -> ResponseResult<()> {
    bot.answer_pre_checkout_query(q.id, true).await?;
    Ok(())
}

async fn handle_successful_payment(
    bot: Bot,
    msg: Message,
    repo: Arc<dyn UserRepository>,
) -> ResponseResult<()> {
    let user_id = msg.chat.id.0;
    let _ = repo.add_balance(user_id, 10).await;
    bot.send_message(
        msg.chat.id,
        "🎉 Успешно! Вам начислено 10 кредитов. Погнали! 🏎💨",
    )
    .await?;
    Ok(())
}

async fn handle_profile(
    bot: Bot,
    msg: Message,
    repo: Arc<dyn UserRepository>,
) -> ResponseResult<()> {
    let user_id = msg.chat.id.0;
    let balance = repo.get_balance(user_id).await;

    // Используем HTML-теги, они не требуют экранирования точек
    let text = format!(
        "<b>👤 Твой профиль DeepDrive AI</b>\n\n\
        🆔 ID: <code>{}</code>\n\
        ⛽️ Баланс: <b>{}</b> треков\n\n\
        <i>Используй эти кредиты для улучшения музыки.</i>",
        user_id, balance
    );

    bot.send_message(msg.chat.id, text)
        .parse_mode(teloxide::types::ParseMode::Html) // МЕНЯЕМ ЗДЕСЬ
        .reply_markup(make_payment_keyboard())
        .await?;

    Ok(())
}
