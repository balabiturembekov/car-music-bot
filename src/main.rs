mod domain;
mod infrastructure;

use crate::domain::audio_service::{AudioPreset, AudioService};
use crate::domain::user_repository::UserRepository;
use crate::infrastructure::ffmpeg_processor::FFmpegProcessor;
use crate::infrastructure::sqlite_user_repo::SqliteUserRepo;
use sqlx::sqlite::SqlitePoolOptions;
use std::sync::Arc;
use teloxide::prelude::*;
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
                .filter(|msg: Message| msg.successful_payment().is_some())
                .endpoint(handle_successful_payment),
        )
        .branch(Update::filter_pre_checkout_query().endpoint(handle_pre_checkout))
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback));

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
        let user_id = msg.chat.id.0;

        // 1. ОБРАБОТКА РЕФЕРАЛЬНОЙ ССЫЛКИ И КОМАНДЫ /START
        if text.starts_with("/start") {
            let parts: Vec<&str> = text.split_whitespace().collect();

            // Если есть аргумент после /start (например, /start 12345678)
            if parts.len() > 1 {
                if let Ok(inviter_id) = parts[1].parse::<i64>() {
                    // Пытаемся зарегистрировать реферала (бонус обоим)
                    if user_id != inviter_id && repo.register_referral(user_id, inviter_id).await {
                        bot.send_message(msg.chat.id, "🎁 <b>Добро пожаловать!</b>\n\nТы зашел по приглашению: тебе начислено 3 стартовых трека, а твоему другу +2 бонуса!")
                            .parse_mode(teloxide::types::ParseMode::Html)
                            .await?;
                    }
                }
            }

            // После обработки реферала или если его нет — показываем профиль
            let balance = repo.get_balance(user_id).await;
            let ref_link = format!("https://t.me{}", user_id);

            bot.send_message(
                msg.chat.id,
                format!(
                    "<b>🏎 Привет в DeepDrive AI!</b>\n\n\
                    💳 Твой баланс: <b>{}</b> кредитов.\n\n\
                    🔗 Твоя ссылка для друзей:\n<code>{}</code>\n\n\
                    <i>Пришли ссылку на YouTube, чтобы прокачать звук!</i>",
                    balance, ref_link
                ),
            )
            .parse_mode(teloxide::types::ParseMode::Html)
            .await?;
            return Ok(());
        }

        // 2. ОБРАБОТКА КОМАНДЫ /PROFILE
        if text == "/profile" {
            let balance = repo.get_balance(user_id).await;
            let ref_link = format!("https://t.me{}", user_id);

            bot.send_message(
                msg.chat.id,
                format!(
                    "<b>👤 Твой профиль</b>\n\n\
                    🆔 ID: <code>{}</code>\n\
                    ⛽️ Баланс: <b>{}</b> треков\n\n\
                    🔗 Реферальная ссылка:\n<code>{}</code>\n\n\
                    <i>За каждого друга даем +2 трека!</i>",
                    user_id, balance, ref_link
                ),
            )
            .parse_mode(teloxide::types::ParseMode::Html)
            .reply_markup(make_payment_keyboard())
            .await?;
            return Ok(());
        }

        // 3. ОБРАБОТКА ССЫЛОК YOUTUBE
        if text.contains("youtu") {
            let balance = repo.get_balance(user_id).await;
            bot.send_message(
                msg.chat.id,
                format!(
                    "💳 Твой баланс: <b>{}</b> кредитов.\n\nВыбери режим прокачки:",
                    balance
                ),
            )
            .parse_mode(teloxide::types::ParseMode::Html)
            .reply_markup(make_keyboard(text))
            .await?;
        }
        // Если просто текст — подсказываем, что делать
        else {
            bot.send_message(msg.chat.id, "📥 Пришли ссылку на YouTube видео или Shorts!")
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
    let chat_id = q
        .message
        .as_ref()
        .map(|m| m.chat().id)
        .unwrap_or(q.from.id.into());

    if let Some(data) = q.data {
        // ОБРАБОТКА ОПЛАТЫ
        if data == "buy_10_credits" {
            bot.answer_callback_query(q.id).await?;
            handle_buy_credits(bot, chat_id).await?;
            return Ok(());
        }

        // ОБРАБОТКА ПРЕСЕТОВ
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

        // Проверка баланса ПЕРЕД запуском скачивания
        if !repo.use_credit(user_id).await {
            bot.answer_callback_query(q.id).await?;
            bot.send_message(
                chat_id,
                "⚠️ У тебя 0 кредитов. Пополни баланс для продолжения! ⭐️",
            )
            .reply_markup(make_payment_keyboard())
            .await?;
            return Ok(());
        }

        if let Some(msg) = q.message {
            let _permit = semaphore.acquire().await.unwrap();
            let _ = bot.answer_callback_query(q.id).await;

            bot.edit_message_text(chat_id, msg.id(), "🏎 Запускаю двигатели... Процесс пошел!")
                .await?;

            match service.process_track(url, preset).await {
                Ok((path, meta)) => {
                    let mins = meta.duration / 60;
                    let secs = meta.duration % 60;
                    let duration_str = format!("{:02}:{:02}", mins, secs);

                    let file = teloxide::types::InputFile::file(&path)
                        .file_name(format!("{}.mp3", meta.title));

                    let _ = bot.send_audio(chat_id, file)
                        .caption(format!(
                            "✅ <b>Готово для авто!</b>\n\n🎵 {}\n👤 {}\n⏱ Длительность: <code>{}</code>", 
                            meta.title, meta.artist, duration_str
                        ))
                        .parse_mode(teloxide::types::ParseMode::Html)
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
