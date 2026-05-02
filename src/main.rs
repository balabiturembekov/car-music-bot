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
use url::Url;
use urlencoding::encode;

const STARTING_CREDITS: i32 = 3;
const PACKAGE_CREDITS: i32 = 10;
const PACKAGE_STARS: u32 = 150;
const CREDIT_PACKAGE_PAYLOAD: &str = "payload_10_credits";

// Клавиатура выбора режима
fn make_keyboard(request_id: &str) -> InlineKeyboardMarkup {
    let buttons = [
        [InlineKeyboardButton::callback(
            "🏎 Car Bass",
            format!("bass|{}", request_id),
        )],
        [InlineKeyboardButton::callback(
            "🎧 Pure Hi-Fi",
            format!("hifi|{}", request_id),
        )],
        [InlineKeyboardButton::callback(
            "🔥 Extreme Low",
            format!("extreme|{}", request_id),
        )],
        [InlineKeyboardButton::callback(
            "🌀 8D Surround",
            format!("8d|{}", request_id),
        )],
    ];
    InlineKeyboardMarkup::new(buttons)
}

// Клавиатура оплаты
fn make_payment_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new([[InlineKeyboardButton::callback(
        format!(
            "💳 Купить {} треков ({} ⭐️)",
            PACKAGE_CREDITS, PACKAGE_STARS
        ),
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

    sqlx::migrate!("./migrations").run(&pool).await?;
    SqliteUserRepo::ensure_schema(&pool).await?;

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
    let me = bot.get_me().await?;
    let bot_username = me.user.username.expect("Bot must have username");

    if let Some(text) = msg.text() {
        let user_id = msg.chat.id.0;

        // 1. ОБРАБОТКА РЕФЕРАЛЬНОЙ ССЫЛКИ И КОМАНДЫ /START
        if text.starts_with("/start") {
            let parts: Vec<&str> = text.split_whitespace().collect();

            // Если есть аргумент после /start (например, /start 12345678)
            if parts.len() > 1
                && let Ok(inviter_id) = parts[1].parse::<i64>()
                && user_id != inviter_id
                && repo.register_referral(user_id, inviter_id).await
            {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "🎁 <b>Добро пожаловать!</b>\n\nТы зашел по приглашению: тебе начислено {} стартовых трека, а твоему другу +2 бонуса!",
                        STARTING_CREDITS
                    ),
                )
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
            }

            // После обработки реферала или если его нет — показываем профиль
            let balance = repo.get_balance(user_id).await;
            let ref_link = format!("https://t.me/{}?start={}", bot_username, user_id);

            let share_url = format!(
                "https://t.me/share/url?url={}&text={}",
                encode(&ref_link),
                encode("Смотри, этот бот качает 8D звук для машины! 🏎🔊"),
            );

            let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::url(
                "🚀 Переслать другу",
                Url::parse(&share_url).expect("Invalid share url"),
            )]]);

            bot.send_message(
                msg.chat.id,
                format!(
                    "<b>🏎 Привет в DeepDrive AI!</b>\n\n\
                    💳 Твой баланс: <b>{}</b> кредитов.\n\n\
                    🔗 <b>Твоя ссылка для друзей:</b>\n{}\n\n\
                    <i>Пригласи друга и получи <b>+2 трека</b> на баланс!</i>",
                    balance, ref_link
                ),
            )
            .parse_mode(teloxide::types::ParseMode::Html)
            // Добавляем кнопку "Поделиться", это самый удобный способ распространения
            .reply_markup(keyboard)
            .await?;
            return Ok(());
        }

        // 2. ОБРАБОТКА КОМАНДЫ /PROFILE
        if text == "/profile" {
            let balance = repo.get_balance(user_id).await;
            let ref_link = format!("https://t.me/{}?start={}", bot_username, user_id);

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
            let request_id = match repo.save_track_request(user_id, text.trim()).await {
                Ok(request_id) => request_id,
                Err(e) => {
                    bot.send_message(
                        msg.chat.id,
                        format!("❌ Не удалось сохранить ссылку: {}", e),
                    )
                    .await?;
                    return Ok(());
                }
            };

            bot.send_message(
                msg.chat.id,
                format!(
                    "💳 Твой баланс: <b>{}</b> кредитов.\n\nВыбери режим прокачки:",
                    balance
                ),
            )
            .parse_mode(teloxide::types::ParseMode::Html)
            .reply_markup(make_keyboard(&request_id))
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

        let Some(msg) = q.message else {
            bot.answer_callback_query(q.id).await?;
            return Ok(());
        };

        // ОБРАБОТКА ПРЕСЕТОВ
        let Some((preset_raw, request_id)) = data.split_once('|') else {
            return Ok(());
        };

        let preset = match preset_raw {
            "bass" => AudioPreset::CarBass,
            "hifi" => AudioPreset::PureHiFi,
            "extreme" => AudioPreset::ExtremeLow,
            "8d" => AudioPreset::Surround8D,
            _ => return Ok(()),
        };

        let url = match repo.get_track_request(user_id, request_id).await {
            Ok(Some(url)) => url,
            Ok(None) => {
                bot.answer_callback_query(q.id).await?;
                bot.send_message(
                    chat_id,
                    "⚠️ Ссылка устарела. Пришли YouTube-ссылку еще раз.",
                )
                .await?;
                return Ok(());
            }
            Err(e) => {
                bot.answer_callback_query(q.id).await?;
                bot.send_message(chat_id, format!("❌ Ошибка БД: {}", e))
                    .await?;
                return Ok(());
            }
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

        let _permit = match semaphore.acquire().await {
            Ok(permit) => permit,
            Err(_) => {
                let _ = repo.add_balance(user_id, 1).await;
                bot.answer_callback_query(q.id).await?;
                bot.send_message(chat_id, "❌ Внутренняя ошибка очереди. Кредит возвращен.")
                    .await?;
                return Ok(());
            }
        };

        let _ = bot.answer_callback_query(q.id).await;
        let _ = bot
            .edit_message_text(chat_id, msg.id(), "🏎 Запускаю двигатели... Процесс пошел!")
            .await;

        match service.process_track(&url, preset).await {
            Ok((path, meta)) => {
                let mins = meta.duration / 60;
                let secs = meta.duration % 60;
                let duration_str = format!("{:02}:{:02}", mins, secs);
                let title = html_escape(&meta.title);
                let artist = html_escape(&meta.artist);

                let file = teloxide::types::InputFile::file(&path)
                    .file_name(format!("{}.mp3", safe_audio_filename(&meta.title)));

                let send_result = bot
                    .send_audio(chat_id, file)
                    .caption(format!(
                        "✅ <b>Готово для авто!</b>\n\n🎵 {}\n👤 {}\n⏱ Длительность: <code>{}</code>",
                        title, artist, duration_str
                    ))
                    .parse_mode(teloxide::types::ParseMode::Html)
                    .await;

                let _ = tokio::fs::remove_file(path).await;

                if let Err(e) = send_result {
                    let _ = repo.add_balance(user_id, 1).await;
                    let _ = bot
                        .send_message(
                            chat_id,
                            format!("❌ Не удалось отправить аудио: {}. Кредит возвращен.", e),
                        )
                        .await;
                }
            }
            Err(e) => {
                let _ = repo.add_balance(user_id, 1).await;
                let _ = bot
                    .send_message(chat_id, format!("❌ Ошибка: {}. Кредит возвращен.", e))
                    .await;
            }
        }
    }
    Ok(())
}

async fn handle_buy_credits(bot: Bot, chat_id: ChatId) -> ResponseResult<()> {
    bot.send_invoice(
        chat_id,
        "10 Премиум-загрузок",
        "Добавляет 10 кредитов для прокачки музыки (включая 8D эффект)",
        CREDIT_PACKAGE_PAYLOAD,
        "XTR",
        vec![LabeledPrice::new(
            format!("{} кредитов", PACKAGE_CREDITS),
            PACKAGE_STARS,
        )],
    )
    .await?;
    Ok(())
}

async fn handle_pre_checkout(bot: Bot, q: PreCheckoutQuery) -> ResponseResult<()> {
    if is_credit_package(&q.currency, q.total_amount, &q.invoice_payload) {
        bot.answer_pre_checkout_query(q.id, true).await?;
    } else {
        bot.answer_pre_checkout_query(q.id, false)
            .error_message("Некорректный платеж. Попробуй заново.")
            .await?;
    }

    Ok(())
}

async fn handle_successful_payment(
    bot: Bot,
    msg: Message,
    repo: Arc<dyn UserRepository>,
) -> ResponseResult<()> {
    let Some(payment) = msg.successful_payment() else {
        return Ok(());
    };

    if !is_credit_package(
        &payment.currency,
        payment.total_amount,
        &payment.invoice_payload,
    ) {
        log::warn!(
            "Rejected successful payment with unexpected package: currency={}, total_amount={}, payload={}",
            payment.currency,
            payment.total_amount,
            payment.invoice_payload
        );
        bot.send_message(
            msg.chat.id,
            "⚠️ Платеж получен, но пакет не распознан. Напиши в поддержку.",
        )
        .await?;
        return Ok(());
    }

    let user_id = msg.chat.id.0;
    let _ = repo.add_balance(user_id, PACKAGE_CREDITS).await;
    bot.send_message(
        msg.chat.id,
        format!(
            "🎉 Успешно! Вам начислено {} кредитов. Погнали! 🏎💨",
            PACKAGE_CREDITS
        ),
    )
    .await?;
    Ok(())
}

fn is_credit_package(currency: &str, total_amount: u32, invoice_payload: &str) -> bool {
    currency == "XTR" && total_amount == PACKAGE_STARS && invoice_payload == CREDIT_PACKAGE_PAYLOAD
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn safe_audio_filename(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect();

    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "track".to_string()
    } else {
        trimmed.chars().take(80).collect()
    }
}
