use std::{
    env,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Multipart, Path as AxumPath, Query, State,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use futures_util::{SinkExt, StreamExt};
use rand::{Rng, distr::Alphanumeric};
use rand_core::OsRng;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    events: broadcast::Sender<String>,
    uploads: PathBuf,
}

#[derive(Debug, Serialize)]
struct ApiError {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(self)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiError>;

fn fail(message: impl Into<String>) -> ApiError {
    ApiError {
        error: message.into(),
    }
}

#[derive(Clone, Debug, Serialize)]
struct User {
    id: i64,
    username: String,
    display_name: String,
    role: String,
    active: bool,
}

#[derive(Deserialize)]
struct Credentials {
    username: String,
    password: String,
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct NewChannel {
    name: String,
    description: Option<String>,
}

#[derive(Deserialize)]
struct NewMessage {
    body: String,
    file_url: Option<String>,
}

#[derive(Deserialize)]
struct HistoryQuery {
    before: Option<i64>,
    q: Option<String>,
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

#[derive(Deserialize)]
struct AdminMessagesQuery {
    page: Option<i64>,
    q: Option<String>,
    scope: Option<String>,
    kind: Option<String>,
    focus: Option<i64>,
}

#[derive(Deserialize)]
struct WsQuery {
    token: String,
}

#[derive(Deserialize)]
struct UserUpdate {
    role: Option<String>,
    active: Option<bool>,
}

#[derive(Deserialize)]
struct ProfileUpdate {
    display_name: String,
}

#[derive(Deserialize)]
struct MessageUpdate {
    body: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lumichat=info,tower_http=info".into()),
        )
        .compact()
        .init();

    let database = env::var("LUMICHAT_DATABASE").unwrap_or_else(|_| "data/lumichat.db".into());
    let uploads =
        PathBuf::from(env::var("LUMICHAT_UPLOADS").unwrap_or_else(|_| "data/uploads".into()));
    let web = PathBuf::from(env::var("LUMICHAT_WEB").unwrap_or_else(|_| "web".into()));
    let bind = env::var("LUMICHAT_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());

    if let Some(parent) = Path::new(&database).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir_all(&uploads)?;
    let conn = Connection::open(&database).context("open SQLite database")?;
    initialize(&conn)?;
    let (events, _) = broadcast::channel(256);
    let state = AppState {
        db: Arc::new(Mutex::new(conn)),
        events,
        uploads,
    };

    let api = Router::new()
        .route("/health", get(health))
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me).patch(update_profile))
        .route("/channels", get(list_channels).post(create_channel))
        .route(
            "/channels/{id}/messages",
            get(channel_history).post(send_channel_message),
        )
        .route("/dm/{user_id}/messages", get(dm_history).post(send_dm))
        .route(
            "/messages/{id}",
            patch(update_message).delete(delete_message),
        )
        .route("/users", get(list_users))
        .route("/users/{id}", patch(update_user))
        .route(
            "/admin/messages",
            get(admin_messages).delete(clear_all_messages),
        )
        .route("/search", get(search))
        .route("/upload", post(upload))
        .route("/ws", get(ws_handler))
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024));

    let app = Router::new()
        .nest("/api", api)
        .nest_service("/uploads", ServeDir::new(&state.uploads))
        .fallback_service(ServeDir::new(&web).fallback(ServeFile::new(web.join("index.html"))))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "LumiChat ready");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA foreign_keys = ON;
        PRAGMA busy_timeout = 5000;
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY,
            username TEXT NOT NULL UNIQUE COLLATE NOCASE,
            display_name TEXT NOT NULL,
            password_hash TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'member' CHECK(role IN ('admin','member')),
            active INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL DEFAULT (unixepoch())
        );
        CREATE TABLE IF NOT EXISTS sessions (
            token TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            created_at INTEGER NOT NULL DEFAULT (unixepoch())
        );
        CREATE TABLE IF NOT EXISTS channels (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE COLLATE NOCASE,
            description TEXT NOT NULL DEFAULT '',
            created_by INTEGER NOT NULL REFERENCES users(id),
            created_at INTEGER NOT NULL DEFAULT (unixepoch())
        );
        CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY,
            sender_id INTEGER NOT NULL REFERENCES users(id),
            channel_id INTEGER REFERENCES channels(id) ON DELETE CASCADE,
            recipient_id INTEGER REFERENCES users(id) ON DELETE CASCADE,
            body TEXT NOT NULL DEFAULT '',
            file_url TEXT,
            created_at INTEGER NOT NULL DEFAULT (unixepoch()),
            CHECK ((channel_id IS NOT NULL) != (recipient_id IS NOT NULL))
        );
        CREATE INDEX IF NOT EXISTS idx_messages_channel_id ON messages(channel_id, id DESC);
        CREATE INDEX IF NOT EXISTS idx_messages_recipient_sender ON messages(recipient_id, sender_id, id DESC);
        CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions(user_id);
        PRAGMA optimize;
    "#)?;
    Ok(())
}

async fn health() -> Json<Value> {
    Json(json!({"status":"ok"}))
}

fn token_from(headers: &HeaderMap) -> ApiResult<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| fail("请先登录"))
}

fn user_for_token(state: &AppState, token: &str) -> ApiResult<User> {
    let db = state.db.lock().map_err(|_| fail("数据库暂不可用"))?;
    db.query_row(
        "SELECT u.id,u.username,u.display_name,u.role,u.active FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token=?1 AND u.active=1",
        [token],
        |r| Ok(User { id:r.get(0)?, username:r.get(1)?, display_name:r.get(2)?, role:r.get(3)?, active:r.get::<_,i64>(4)? != 0 }),
    ).optional().map_err(|_| fail("身份验证失败"))?.ok_or_else(|| fail("登录已失效"))
}

fn auth(state: &AppState, headers: &HeaderMap) -> ApiResult<User> {
    user_for_token(state, token_from(headers)?)
}

fn clean_username(value: &str) -> ApiResult<String> {
    let v = value.trim().to_lowercase();
    if !(3..=24).contains(&v.len())
        || !v
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(fail("用户名需为 3–24 位字母、数字、下划线或连字符"));
    }
    Ok(v)
}

fn hash_password(password: &str) -> ApiResult<String> {
    if password.len() < 8 {
        return Err(fail("密码至少需要 8 位"));
    }
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| fail("无法加密密码"))
}

fn new_token() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect()
}

async fn register(
    State(state): State<AppState>,
    Json(input): Json<Credentials>,
) -> ApiResult<Json<Value>> {
    let username = clean_username(&input.username)?;
    let display = input
        .display_name
        .unwrap_or_else(|| username.clone())
        .trim()
        .chars()
        .take(40)
        .collect::<String>();
    if display.is_empty() {
        return Err(fail("显示名称不能为空"));
    }
    let password_hash = hash_password(&input.password)?;
    let token = new_token();
    let db = state.db.lock().map_err(|_| fail("数据库暂不可用"))?;
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
        .map_err(|_| fail("无法读取用户"))?;
    let role = if count == 0 { "admin" } else { "member" };
    db.execute(
        "INSERT INTO users(username,display_name,password_hash,role) VALUES(?1,?2,?3,?4)",
        params![username, display, password_hash, role],
    )
    .map_err(|_| fail("用户名已被使用"))?;
    let id = db.last_insert_rowid();
    db.execute(
        "INSERT INTO sessions(token,user_id) VALUES(?1,?2)",
        params![token, id],
    )
    .map_err(|_| fail("无法创建会话"))?;
    if count == 0 {
        db.execute("INSERT INTO channels(name,description,created_by) VALUES('general','欢迎来到 LumiChat',?1)", [id]).map_err(|_| fail("无法创建默认频道"))?;
    }
    Ok(Json(
        json!({"token":token,"user":{"id":id,"username":username,"display_name":display,"role":role,"active":true}}),
    ))
}

async fn login(
    State(state): State<AppState>,
    Json(input): Json<Credentials>,
) -> ApiResult<Json<Value>> {
    let username = clean_username(&input.username)?;
    let db = state.db.lock().map_err(|_| fail("数据库暂不可用"))?;
    let row: Option<(User,String)> = db.query_row("SELECT id,username,display_name,role,active,password_hash FROM users WHERE username=?1", [username], |r| {
        Ok((User{id:r.get(0)?,username:r.get(1)?,display_name:r.get(2)?,role:r.get(3)?,active:r.get::<_,i64>(4)? != 0}, r.get(5)?))
    }).optional().map_err(|_| fail("登录失败"))?;
    let (user, stored) = row.ok_or_else(|| fail("用户名或密码不正确"))?;
    if !user.active {
        return Err(fail("账号已停用"));
    }
    let parsed = PasswordHash::new(&stored).map_err(|_| fail("登录失败"))?;
    Argon2::default()
        .verify_password(input.password.as_bytes(), &parsed)
        .map_err(|_| fail("用户名或密码不正确"))?;
    let token = new_token();
    db.execute(
        "INSERT INTO sessions(token,user_id) VALUES(?1,?2)",
        params![token, user.id],
    )
    .map_err(|_| fail("无法创建会话"))?;
    Ok(Json(json!({"token":token,"user":user})))
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<StatusCode> {
    let token = token_from(&headers)?;
    state
        .db
        .lock()
        .map_err(|_| fail("数据库暂不可用"))?
        .execute("DELETE FROM sessions WHERE token=?1", [token])
        .map_err(|_| fail("退出失败"))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn me(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<User>> {
    Ok(Json(auth(&state, &headers)?))
}

async fn update_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ProfileUpdate>,
) -> ApiResult<Json<User>> {
    let mut user = auth(&state, &headers)?;
    let display_name = input
        .display_name
        .trim()
        .chars()
        .take(40)
        .collect::<String>();
    if display_name.is_empty() {
        return Err(fail("显示名称不能为空"));
    }
    state
        .db
        .lock()
        .map_err(|_| fail("数据库暂不可用"))?
        .execute(
            "UPDATE users SET display_name=?1 WHERE id=?2",
            params![display_name, user.id],
        )
        .map_err(|_| fail("无法更新资料"))?;
    user.display_name = display_name;
    Ok(Json(user))
}

async fn list_channels(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    auth(&state, &headers)?;
    let db = state.db.lock().map_err(|_| fail("数据库暂不可用"))?;
    let mut stmt = db
        .prepare("SELECT id,name,description,created_at FROM channels ORDER BY name")
        .map_err(|_| fail("无法读取频道"))?;
    let rows = stmt.query_map([], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"name":r.get::<_,String>(1)?,"description":r.get::<_,String>(2)?,"created_at":r.get::<_,i64>(3)?}))).map_err(|_| fail("无法读取频道"))?;
    Ok(Json(Value::Array(rows.filter_map(Result::ok).collect())))
}

async fn create_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<NewChannel>,
) -> ApiResult<Json<Value>> {
    let user = auth(&state, &headers)?;
    let name = input.name.trim().to_lowercase();
    if name.is_empty() || name.len() > 32 {
        return Err(fail("频道名称需为 1–32 个字符"));
    }
    let desc = input
        .description
        .unwrap_or_default()
        .trim()
        .chars()
        .take(120)
        .collect::<String>();
    let db = state.db.lock().map_err(|_| fail("数据库暂不可用"))?;
    db.execute(
        "INSERT INTO channels(name,description,created_by) VALUES(?1,?2,?3)",
        params![name, desc, user.id],
    )
    .map_err(|_| fail("频道名称已存在"))?;
    let id = db.last_insert_rowid();
    drop(db);
    let _ = state.events.send(
        json!({"type":"channel_created","channel":{"id":id,"name":name,"description":desc}})
            .to_string(),
    );
    Ok(Json(json!({"id":id,"name":name,"description":desc})))
}

fn messages(db: &Connection, sql: &str, args: &[&dyn rusqlite::ToSql]) -> ApiResult<Value> {
    let mut stmt = db.prepare(sql).map_err(|_| fail("无法读取消息"))?;
    let rows = stmt.query_map(args, |r| Ok(json!({
        "id":r.get::<_,i64>(0)?, "body":r.get::<_,String>(1)?, "file_url":r.get::<_,Option<String>>(2)?, "created_at":r.get::<_,i64>(3)?,
        "sender":{"id":r.get::<_,i64>(4)?,"username":r.get::<_,String>(5)?,"display_name":r.get::<_,String>(6)?}
    }))).map_err(|_| fail("无法读取消息"))?;
    let mut values: Vec<Value> = rows.filter_map(Result::ok).collect();
    values.reverse();
    Ok(Value::Array(values))
}

async fn channel_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Query(query): Query<HistoryQuery>,
) -> ApiResult<Json<Value>> {
    auth(&state, &headers)?;
    let before = query.before.unwrap_or(i64::MAX);
    let pattern = format!("%{}%", query.q.unwrap_or_default());
    let db = state.db.lock().map_err(|_| fail("数据库暂不可用"))?;
    Ok(Json(messages(
        &db,
        "SELECT m.id,m.body,m.file_url,m.created_at,u.id,u.username,u.display_name FROM messages m JOIN users u ON u.id=m.sender_id WHERE m.channel_id=?1 AND m.id<?2 AND m.body LIKE ?3 ORDER BY m.id DESC LIMIT 60",
        &[&id, &before, &pattern],
    )?))
}

async fn send_channel_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(input): Json<NewMessage>,
) -> ApiResult<Json<Value>> {
    let user = auth(&state, &headers)?;
    let body = input.body.trim().chars().take(4000).collect::<String>();
    if body.is_empty() && input.file_url.is_none() {
        return Err(fail("消息不能为空"));
    }
    let db = state.db.lock().map_err(|_| fail("数据库暂不可用"))?;
    db.execute(
        "INSERT INTO messages(sender_id,channel_id,body,file_url) VALUES(?1,?2,?3,?4)",
        params![user.id, id, body, input.file_url],
    )
    .map_err(|_| fail("频道不存在"))?;
    let message_id = db.last_insert_rowid();
    let created_at: i64 = db
        .query_row(
            "SELECT created_at FROM messages WHERE id=?1",
            [message_id],
            |r| r.get(0),
        )
        .unwrap_or_default();
    drop(db);
    let event = json!({"type":"message","scope":"channel","channel_id":id,"message":{"id":message_id,"body":body,"file_url":input.file_url,"created_at":created_at,"sender":user}});
    let _ = state.events.send(event.to_string());
    Ok(Json(event))
}

async fn dm_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(other): AxumPath<i64>,
    Query(query): Query<HistoryQuery>,
) -> ApiResult<Json<Value>> {
    let user = auth(&state, &headers)?;
    let before = query.before.unwrap_or(i64::MAX);
    let db = state.db.lock().map_err(|_| fail("数据库暂不可用"))?;
    Ok(Json(messages(
        &db,
        "SELECT m.id,m.body,m.file_url,m.created_at,u.id,u.username,u.display_name FROM messages m JOIN users u ON u.id=m.sender_id WHERE m.id<?1 AND ((m.sender_id=?2 AND m.recipient_id=?3) OR (m.sender_id=?3 AND m.recipient_id=?2)) ORDER BY m.id DESC LIMIT 60",
        &[&before, &user.id, &other],
    )?))
}

async fn send_dm(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(recipient): AxumPath<i64>,
    Json(input): Json<NewMessage>,
) -> ApiResult<Json<Value>> {
    let user = auth(&state, &headers)?;
    let body = input.body.trim().chars().take(4000).collect::<String>();
    if body.is_empty() && input.file_url.is_none() {
        return Err(fail("消息不能为空"));
    }
    let db = state.db.lock().map_err(|_| fail("数据库暂不可用"))?;
    db.execute(
        "INSERT INTO messages(sender_id,recipient_id,body,file_url) VALUES(?1,?2,?3,?4)",
        params![user.id, recipient, body, input.file_url],
    )
    .map_err(|_| fail("收件人不存在"))?;
    let id = db.last_insert_rowid();
    let created_at: i64 = db
        .query_row("SELECT created_at FROM messages WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap_or_default();
    drop(db);
    let event = json!({"type":"message","scope":"dm","recipient_id":recipient,"message":{"id":id,"body":body,"file_url":input.file_url,"created_at":created_at,"sender":user}});
    let _ = state.events.send(event.to_string());
    Ok(Json(event))
}

async fn update_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(input): Json<MessageUpdate>,
) -> ApiResult<Json<Value>> {
    let user = auth(&state, &headers)?;
    let body = input.body.trim().chars().take(4000).collect::<String>();
    if body.is_empty() {
        return Err(fail("消息不能为空"));
    }
    let db = state.db.lock().map_err(|_| fail("数据库暂不可用"))?;
    let row: Option<(i64, Option<i64>, Option<i64>)> = db
        .query_row(
            "SELECT sender_id,channel_id,recipient_id FROM messages WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|_| fail("无法读取消息"))?;
    let (sender_id, channel_id, recipient_id) = row.ok_or_else(|| fail("消息不存在"))?;
    if sender_id != user.id {
        return Err(fail("只能编辑自己的消息"));
    }
    db.execute("UPDATE messages SET body=?1 WHERE id=?2", params![body, id])
        .map_err(|_| fail("编辑失败"))?;
    drop(db);
    let event = if let Some(channel_id) = channel_id {
        json!({"type":"message_updated","scope":"channel","channel_id":channel_id,"message_id":id,"body":body})
    } else {
        json!({"type":"message_updated","scope":"dm","recipient_id":recipient_id,"message_id":id,"body":body,"sender_id":sender_id})
    };
    let _ = state.events.send(event.to_string());
    Ok(Json(event))
}

async fn delete_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> ApiResult<Json<Value>> {
    let user = auth(&state, &headers)?;
    let db = state.db.lock().map_err(|_| fail("数据库暂不可用"))?;
    let row: Option<(i64, Option<i64>, Option<i64>)> = db
        .query_row(
            "SELECT sender_id,channel_id,recipient_id FROM messages WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|_| fail("无法读取消息"))?;
    let (sender_id, channel_id, recipient_id) = row.ok_or_else(|| fail("消息不存在"))?;
    if sender_id != user.id && user.role != "admin" {
        return Err(fail("没有权限删除这条消息"));
    }
    db.execute("DELETE FROM messages WHERE id=?1", [id])
        .map_err(|_| fail("删除失败"))?;
    drop(db);
    let event = if let Some(channel_id) = channel_id {
        json!({"type":"message_deleted","scope":"channel","channel_id":channel_id,"message_id":id})
    } else {
        json!({"type":"message_deleted","scope":"dm","recipient_id":recipient_id,"message_id":id,"sender_id":sender_id})
    };
    let _ = state.events.send(event.to_string());
    Ok(Json(event))
}

async fn list_users(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    auth(&state, &headers)?;
    let db = state.db.lock().map_err(|_| fail("数据库暂不可用"))?;
    let mut stmt = db
        .prepare("SELECT id,username,display_name,role,active FROM users ORDER BY display_name")
        .map_err(|_| fail("无法读取用户"))?;
    let rows = stmt.query_map([], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"username":r.get::<_,String>(1)?,"display_name":r.get::<_,String>(2)?,"role":r.get::<_,String>(3)?,"active":r.get::<_,i64>(4)? != 0}))).map_err(|_| fail("无法读取用户"))?;
    Ok(Json(Value::Array(rows.filter_map(Result::ok).collect())))
}

async fn update_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(input): Json<UserUpdate>,
) -> ApiResult<Json<Value>> {
    let admin = auth(&state, &headers)?;
    if admin.role != "admin" {
        return Err(fail("需要管理员权限"));
    }
    if id == admin.id && input.active == Some(false) {
        return Err(fail("不能停用自己的账号"));
    }
    let role = input.role.unwrap_or_else(|| "member".into());
    if role != "admin" && role != "member" {
        return Err(fail("无效角色"));
    }
    let active = input.active.unwrap_or(true);
    state
        .db
        .lock()
        .map_err(|_| fail("数据库暂不可用"))?
        .execute(
            "UPDATE users SET role=?1,active=?2 WHERE id=?3",
            params![role, active, id],
        )
        .map_err(|_| fail("更新失败"))?;
    Ok(Json(json!({"id":id,"role":role,"active":active})))
}

async fn admin_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AdminMessagesQuery>,
) -> ApiResult<Json<Value>> {
    let admin = auth(&state, &headers)?;
    if admin.role != "admin" {
        return Err(fail("需要管理员权限"));
    }
    let q = query
        .q
        .unwrap_or_default()
        .trim()
        .chars()
        .take(100)
        .collect::<String>();
    let pattern = format!("%{q}%");
    let scope = query.scope.unwrap_or_else(|| "all".into());
    if !matches!(scope.as_str(), "all" | "channel" | "dm") {
        return Err(fail("无效记录类型"));
    }
    let kind = query.kind.unwrap_or_else(|| "all".into());
    if !matches!(kind.as_str(), "all" | "images") {
        return Err(fail("无效内容类型"));
    }
    let page_size = if kind == "images" { 30i64 } else { 50i64 };
    let db = state.db.lock().map_err(|_| fail("数据库暂不可用"))?;
    let filter = "(?1='' OR m.body LIKE ?2)
        AND (?3='all' OR (?3='channel' AND m.channel_id IS NOT NULL) OR (?3='dm' AND m.recipient_id IS NOT NULL))
        AND (?4='all' OR (m.file_url IS NOT NULL AND (
            LOWER(m.file_url) LIKE '%.png' OR LOWER(m.file_url) LIKE '%.jpg' OR
            LOWER(m.file_url) LIKE '%.jpeg' OR LOWER(m.file_url) LIKE '%.gif' OR
            LOWER(m.file_url) LIKE '%.webp' OR LOWER(m.file_url) LIKE '%.avif'
        )))";
    let total: i64 = db
        .query_row(
            &format!("SELECT COUNT(*) FROM messages m WHERE {filter}"),
            params![q, pattern, scope, kind],
            |r| r.get(0),
        )
        .map_err(|_| fail("无法统计全部记录"))?;
    let total_pages = ((total + page_size - 1) / page_size).max(1);
    let mut page = query.page.unwrap_or(1).clamp(1, total_pages);
    if let Some(focus) = query.focus {
        let focus_filter = "(?2='' OR m.body LIKE ?3)
            AND (?4='all' OR (?4='channel' AND m.channel_id IS NOT NULL) OR (?4='dm' AND m.recipient_id IS NOT NULL))
            AND (?5='all' OR (m.file_url IS NOT NULL AND (
                LOWER(m.file_url) LIKE '%.png' OR LOWER(m.file_url) LIKE '%.jpg' OR
                LOWER(m.file_url) LIKE '%.jpeg' OR LOWER(m.file_url) LIKE '%.gif' OR
                LOWER(m.file_url) LIKE '%.webp' OR LOWER(m.file_url) LIKE '%.avif'
            )))";
        let newer: i64 = db
            .query_row(
                &format!("SELECT COUNT(*) FROM messages m WHERE m.id>?1 AND {focus_filter}"),
                params![focus, q, pattern, scope, kind],
                |r| r.get(0),
            )
            .map_err(|_| fail("无法定位聊天记录"))?;
        page = (newer / page_size + 1).clamp(1, total_pages);
    }
    let offset = (page - 1) * page_size;
    let mut stmt = db
        .prepare(&format!(
            "SELECT m.id,m.body,m.file_url,m.created_at,
                s.id,s.username,s.display_name,
                c.id,c.name,
                r.id,r.username,r.display_name
         FROM messages m
         JOIN users s ON s.id=m.sender_id
         LEFT JOIN channels c ON c.id=m.channel_id
         LEFT JOIN users r ON r.id=m.recipient_id
         WHERE {filter}
         ORDER BY m.id DESC LIMIT ?5 OFFSET ?6"
        ))
        .map_err(|_| fail("无法读取全部记录"))?;
    let rows = stmt.query_map(params![q, pattern, scope, kind, page_size, offset], |r| {
        let channel_id: Option<i64> = r.get(7)?;
        let recipient_id: Option<i64> = r.get(9)?;
        Ok(json!({
            "id":r.get::<_,i64>(0)?, "body":r.get::<_,String>(1)?, "file_url":r.get::<_,Option<String>>(2)?, "created_at":r.get::<_,i64>(3)?,
            "scope":if channel_id.is_some() { "channel" } else { "dm" },
            "sender":{"id":r.get::<_,i64>(4)?,"username":r.get::<_,String>(5)?,"display_name":r.get::<_,String>(6)?},
            "channel":channel_id.map(|id| json!({"id":id,"name":r.get::<_,String>(8).unwrap_or_default()})),
            "recipient":recipient_id.map(|id| json!({"id":id,"username":r.get::<_,String>(10).unwrap_or_default(),"display_name":r.get::<_,String>(11).unwrap_or_default()}))
        }))
    }).map_err(|_| fail("无法读取全部记录"))?;
    Ok(Json(json!({
        "items": rows.filter_map(Result::ok).collect::<Vec<_>>(),
        "page": page,
        "page_size": page_size,
        "total": total,
        "total_pages": total_pages
    })))
}

async fn clear_all_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let admin = auth(&state, &headers)?;
    if admin.role != "admin" {
        return Err(fail("需要管理员权限"));
    }

    let deleted_messages = {
        let db = state.db.lock().map_err(|_| fail("数据库暂不可用"))?;
        let count = db
            .execute("DELETE FROM messages", [])
            .map_err(|_| fail("清空聊天记录失败"))?;
        let _ = db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;");
        count
    };

    let mut deleted_files = 0usize;
    let mut failed_files = 0usize;
    if let Ok(mut entries) = tokio::fs::read_dir(&state.uploads).await {
        loop {
            match entries.next_entry().await {
                Ok(Some(entry)) => match entry.file_type().await {
                    Ok(kind) if kind.is_file() || kind.is_symlink() => {
                        if tokio::fs::remove_file(entry.path()).await.is_ok() {
                            deleted_files += 1;
                        } else {
                            failed_files += 1;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => failed_files += 1,
                },
                Ok(None) => break,
                Err(_) => {
                    failed_files += 1;
                    break;
                }
            }
        }
    }

    let event = json!({"type":"messages_cleared","by_user_id":admin.id});
    let _ = state.events.send(event.to_string());
    Ok(Json(json!({
        "deleted_messages": deleted_messages,
        "deleted_files": deleted_files,
        "failed_files": failed_files
    })))
}

async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> ApiResult<Json<Value>> {
    let user = auth(&state, &headers)?;
    let q = query.q.trim();
    if q.len() < 2 {
        return Err(fail("至少输入 2 个字符"));
    }
    let pattern = format!("%{}%", q);
    let db = state.db.lock().map_err(|_| fail("数据库暂不可用"))?;
    Ok(Json(messages(
        &db,
        "SELECT m.id,m.body,m.file_url,m.created_at,u.id,u.username,u.display_name FROM messages m JOIN users u ON u.id=m.sender_id WHERE m.body LIKE ?1 AND (m.channel_id IS NOT NULL OR m.sender_id=?2 OR m.recipient_id=?2) ORDER BY m.id DESC LIMIT 50",
        &[&pattern, &user.id],
    )?))
}

async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> ApiResult<Json<Value>> {
    auth(&state, &headers)?;
    let field = multipart
        .next_field()
        .await
        .map_err(|_| fail("无法读取上传内容"))?
        .ok_or_else(|| fail("请选择文件"))?;
    let original = field.file_name().unwrap_or("file").to_string();
    let safe: String = original
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || ".-_".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    let prefix: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .take(12)
        .map(char::from)
        .collect();
    let name = format!("{prefix}-{safe}");
    let bytes = field.bytes().await.map_err(|_| fail("无法读取文件"))?;
    tokio::fs::write(state.uploads.join(&name), &bytes)
        .await
        .map_err(|_| fail("无法保存文件"))?;
    Ok(Json(
        json!({"url":format!("/uploads/{name}"),"name":original,"size":bytes.len()}),
    ))
}

async fn ws_handler(
    State(state): State<AppState>,
    Query(query): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    let user = user_for_token(&state, &query.token)?;
    Ok(ws
        .on_upgrade(move |socket| ws_loop(socket, state, user))
        .into_response())
}

async fn ws_loop(socket: WebSocket, state: AppState, user: User) {
    let (mut sender, mut receiver) = socket.split();
    let mut events = state.events.subscribe();
    let welcome = json!({"type":"ready","user_id":user.id}).to_string();
    if sender.send(WsMessage::Text(welcome.into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(text) => {
                    let visible = serde_json::from_str::<Value>(&text).ok().is_none_or(|event| {
                        let event_type = event.get("type").and_then(Value::as_str).unwrap_or_default();
                        if event_type.starts_with("call_") || event_type == "ice_candidate" {
                            return event.get("to_user_id").and_then(Value::as_i64) == Some(user.id)
                                || event.pointer("/from/id").and_then(Value::as_i64) == Some(user.id);
                        }
                        event.get("scope").and_then(Value::as_str) != Some("dm")
                            || event.get("recipient_id").and_then(Value::as_i64) == Some(user.id)
                            || event.pointer("/message/sender/id").and_then(Value::as_i64) == Some(user.id)
                    });
                    if visible && sender.send(WsMessage::Text(text.into())).await.is_err() { break; }
                },
                Err(_) => break,
            },
            incoming = receiver.next() => match incoming {
                Some(Ok(WsMessage::Ping(v))) => if sender.send(WsMessage::Pong(v)).await.is_err() { break; },
                Some(Ok(WsMessage::Text(text))) if text.len() <= 64 * 1024 => {
                    if let Ok(mut signal) = serde_json::from_str::<Value>(&text) {
                        let event_type = signal.get("type").and_then(Value::as_str).unwrap_or_default();
                        let allowed = matches!(event_type, "call_offer" | "call_answer" | "ice_candidate" | "call_end" | "call_reject");
                        let target = signal.get("to_user_id").and_then(Value::as_i64).unwrap_or_default();
                        if allowed && target > 0 && target != user.id {
                            let target_active = state.db.lock().ok().and_then(|db| {
                                db.query_row("SELECT active FROM users WHERE id=?1", [target], |r| r.get::<_, i64>(0)).optional().ok().flatten()
                            }) == Some(1);
                            if target_active {
                                signal["from"] = serde_json::to_value(&user).unwrap_or(Value::Null);
                                let _ = state.events.send(signal.to_string());
                            }
                        }
                    }
                },
                Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            }
        }
    }
}
