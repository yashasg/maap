use super::*;
use aes_gcm::Aes256Gcm;
use sqlx::{Postgres, Transaction};
use totp_rs::{Algorithm as TotpAlgorithm, Secret, TOTP};

const PARTIAL_AUTH_TOKEN_TYPE: &str = "partial-auth";
const API_KEY_PREFIX: &str = "umami_";
const API_KEY_DISPLAY_LENGTH: usize = API_KEY_PREFIX.len() + 8;
const MAX_TWO_FACTOR_ATTEMPTS: i32 = 5;
const TWO_FACTOR_LOCKOUT_MINUTES: i32 = 15;
const OTP_REPLAY_SECONDS: i32 = 90;
const PARTIAL_TOKEN_SECONDS: i64 = 5 * 60;
const BACKUP_CODE_COUNT: usize = 10;
const SYSTEM_USER_ID: &str = "41e2b680-648e-4b09-bcd7-3e2b10c06264";
const SYSTEM_USER_PASSWORD_HASH: &str =
    "$2b$10$BUli0c.muyCW1ErNJc3jL.vFRFtFJWrT8/GcR4A.sUdCznaXiqFXa";

pub fn create_secure_token(claims: &Value, secret: &str) -> Option<String> {
    let mut claims_obj = claims.as_object()?.clone();
    if !claims_obj.contains_key("iat") {
        claims_obj.insert("iat".to_string(), json!(Utc::now().timestamp()));
    }
    let jwt = encode(
        &Header::new(Algorithm::HS256),
        &Value::Object(claims_obj),
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .ok()?;

    let mut salt = [0u8; 64];
    let mut iv = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut iv);

    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha512>(secret.as_bytes(), &salt, 10_000, &mut key);
    let cipher = Aes256Gcm16::new_from_slice(&key).ok()?;
    let ciphertext_with_tag = cipher
        .encrypt(Nonce::from_slice(&iv), jwt.as_bytes())
        .ok()?;
    let split = ciphertext_with_tag.len().checked_sub(16)?;
    let (encrypted, tag) = ciphertext_with_tag.split_at(split);

    let mut out = Vec::with_capacity(64 + 16 + 16 + encrypted.len());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&iv);
    out.extend_from_slice(tag);
    out.extend_from_slice(encrypted);
    Some(BASE64.encode(out))
}

pub fn parse_secure_token(token: &str, secret: &str) -> Option<Value> {
    let raw = BASE64.decode(token.trim()).ok()?;
    if raw.len() < 96 {
        return None;
    }
    let salt = &raw[0..64];
    let iv = &raw[64..80];
    let tag = &raw[80..96];
    let encrypted = &raw[96..];

    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha512>(secret.as_bytes(), salt, 10_000, &mut key);
    let cipher = Aes256Gcm16::new_from_slice(&key).ok()?;

    let mut combined = Vec::with_capacity(encrypted.len() + 16);
    combined.extend_from_slice(encrypted);
    combined.extend_from_slice(tag);

    let decrypted = cipher
        .decrypt(Nonce::from_slice(iv), combined.as_ref())
        .ok()?;
    let jwt_str = String::from_utf8(decrypted).ok()?;

    let mut validation = Validation::new(Algorithm::HS256);
    validation.required_spec_claims.clear();
    validation.validate_exp = true;
    validation.leeway = 0;
    decode::<Value>(
        &jwt_str,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .ok()
    .map(|decoded| decoded.claims)
}

fn api_error(status: StatusCode, code: &str, message: &str) -> Response {
    with_rust_header(
        (
            status,
            Json(json!({
                "error": {
                    "code": code,
                    "message": message,
                    "status": status.as_u16()
                }
            })),
        )
            .into_response(),
    )
}

fn rate_limit_error(status: StatusCode, locked_until: Option<DateTime<Utc>>) -> Response {
    let mut error = json!({
        "code": "two-factor-error-too-many-attempts",
        "message": "Too many failed attempts",
        "status": status.as_u16()
    });
    if let Some(locked_until) = locked_until {
        error["lockedUntil"] = json!(locked_until);
    }
    with_rust_header((status, Json(json!({ "error": error }))).into_response())
}

fn cloud_mode() -> bool {
    env::var("CLOUD_MODE").is_ok_and(|value| !value.is_empty())
}

fn two_factor_key() -> Option<[u8; 32]> {
    let value = env::var("TWO_FACTOR_ENCRYPTION_KEY").ok()?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let decoded = hex::decode(value).ok()?;
    decoded.try_into().ok()
}

fn two_factor_configuration_error() -> Response {
    api_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "two-factor-error-not-configured",
        "TWO_FACTOR_ENCRYPTION_KEY is missing or invalid",
    )
}

fn encrypt_two_factor_secret_with_key(plaintext: &str, key: &[u8; 32]) -> Option<String> {
    let mut iv = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut iv);
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let encrypted_with_tag = cipher
        .encrypt(aes_gcm::Nonce::from_slice(&iv), plaintext.as_bytes())
        .ok()?;
    let split = encrypted_with_tag.len().checked_sub(16)?;
    let (ciphertext, tag) = encrypted_with_tag.split_at(split);
    Some(format!(
        "{}:{}:{}",
        hex::encode(ciphertext),
        hex::encode(iv),
        hex::encode(tag)
    ))
}

fn decrypt_two_factor_secret_with_key(stored: &str, key: &[u8; 32]) -> Option<String> {
    let mut parts = stored.split(':');
    let ciphertext = hex::decode(parts.next()?).ok()?;
    let iv = hex::decode(parts.next()?).ok()?;
    let tag = hex::decode(parts.next()?).ok()?;
    if parts.next().is_some() || iv.len() != 12 || tag.len() != 16 {
        return None;
    }
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let mut combined = ciphertext;
    combined.extend_from_slice(&tag);
    String::from_utf8(
        cipher
            .decrypt(aes_gcm::Nonce::from_slice(&iv), combined.as_ref())
            .ok()?,
    )
    .ok()
}

fn new_totp(secret: &str, username: &str) -> Option<TOTP> {
    TOTP::new(
        TotpAlgorithm::SHA1,
        6,
        1,
        30,
        Secret::Encoded(secret.to_string()).to_bytes().ok()?,
        Some("Umami".to_string()),
        username.to_string(),
    )
    .ok()
}

fn is_api_key_blocked_path(path: &str) -> bool {
    const BLOCKED: [&str; 6] = [
        "/api/me/password",
        "/api/me/api-keys",
        "/api/2fa",
        "/api/auth",
        "/api/users",
        "/api/admin",
    ];
    BLOCKED
        .iter()
        .any(|blocked| path == *blocked || path.starts_with(&format!("{blocked}/")))
}

fn is_setup_route(path: &str) -> bool {
    matches!(
        path,
        "/api/2fa/status"
            | "/api/2fa/setup/initiate"
            | "/api/2fa/setup/confirm"
            | "/api/2fa/setup/cancel"
            | "/api/auth/verify"
            | "/api/auth/logout"
    )
}

pub(super) fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

async fn session_auth(pool: &PgPool, token: &str, path: &str) -> Option<(Uuid, String)> {
    let claims = parse_secure_token(token, &secret_hash())?;
    if claims.get("type").and_then(Value::as_str) == Some(PARTIAL_AUTH_TOKEN_TYPE) {
        return None;
    }
    let user_id = Uuid::parse_str(claims.get("userId")?.as_str()?).ok()?;
    let row = sqlx::query(
        r#"
        SELECT user_id, role, password
        FROM "user"
        WHERE user_id = $1 AND deleted_at IS NULL
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .ok()??;
    let password: String = row.get("password");
    if claims
        .get("pwd")
        .and_then(Value::as_str)
        .is_some_and(|fingerprint| sha512_hex(&[&password]) != fingerprint)
    {
        return None;
    }

    if claims
        .get("twoFactorSetup")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let enabled = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT COALESCE(
                (SELECT is_enabled FROM two_factor_auth WHERE user_id = $1),
                false
            )
            "#,
        )
        .bind(user_id)
        .fetch_one(pool)
        .await
        .ok()?;
        if !enabled && !is_setup_route(path) {
            return None;
        }
    }

    Some((row.get("user_id"), row.get("role")))
}

async fn api_key_auth(pool: &PgPool, token: &str, path: &str) -> Option<(Uuid, String)> {
    if cloud_mode() || is_api_key_blocked_path(path) {
        return None;
    }
    let row = sqlx::query(
        r#"
        SELECT k.api_key_id, k.last_used_at, u.user_id, u.role
        FROM api_key k
        JOIN "user" u ON u.user_id = k.user_id
        WHERE k.key_hash = $1 AND u.deleted_at IS NULL
        "#,
    )
    .bind(sha512_hex(&[token]))
    .fetch_optional(pool)
    .await
    .ok()??;

    let api_key_id: Uuid = row.get("api_key_id");
    let last_used_at: Option<DateTime<Utc>> = row.get("last_used_at");
    if last_used_at.is_none_or(|last| Utc::now().signed_duration_since(last).num_minutes() >= 5) {
        let _ = sqlx::query("UPDATE api_key SET last_used_at = NOW() WHERE api_key_id = $1")
            .bind(api_key_id)
            .execute(pool)
            .await;
    }
    Some((row.get("user_id"), row.get("role")))
}

pub(super) async fn check_bearer_auth(
    pool: &PgPool,
    headers: &HeaderMap,
    path: &str,
) -> Option<(Uuid, String)> {
    let token = bearer_token(headers)?;
    if token.starts_with(API_KEY_PREFIX) {
        api_key_auth(pool, token, path).await
    } else {
        session_auth(pool, token, path).await
    }
}

async fn required_reason(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Option<&'static str>, sqlx::Error> {
    let global = sqlx::query_scalar::<_, String>(
        "SELECT value FROM app_setting WHERE key = 'twoFactorRequiredGlobal'",
    )
    .fetch_optional(pool)
    .await?
    .as_deref()
        == Some("true");
    if global {
        return Ok(Some("global"));
    }

    let user_required = sqlx::query_scalar::<_, bool>(
        r#"SELECT two_factor_required FROM "user" WHERE user_id = $1"#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    .unwrap_or(false);
    if user_required {
        return Ok(Some("user"));
    }

    let team_required = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM team_user tu
            JOIN team t ON t.team_id = tu.team_id
            WHERE tu.user_id = $1
              AND t.deleted_at IS NULL
              AND t.two_factor_required = true
        )
        "#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(team_required.then_some("team"))
}

async fn global_required(pool: &PgPool) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT value FROM app_setting WHERE key = 'twoFactorRequiredGlobal'",
    )
    .fetch_optional(pool)
    .await?
    .as_deref()
        == Some("true"))
}

async fn user_json(
    pool: &PgPool,
    user_id: Uuid,
    response_username: Option<&str>,
) -> Result<Value, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT username, password, role, two_factor_required, created_at
        FROM "user"
        WHERE user_id = $1 AND deleted_at IS NULL
        "#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    let stored_username: String = row.get("username");
    let password: String = row.get("password");
    let role: String = row.get("role");
    let created_at: Option<DateTime<Utc>> = row.get("created_at");
    let two_factor_required: bool = row.get("two_factor_required");
    let teams = sqlx::query(
        r#"
        SELECT t.team_id, t.name, t.logo_url
        FROM team t
        JOIN team_user tu ON tu.team_id = t.team_id
        WHERE tu.user_id = $1 AND t.deleted_at IS NULL
        ORDER BY t.name
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|team| {
        json!({
            "id": team.get::<Uuid, _>("team_id"),
            "name": team.get::<String, _>("name"),
            "logoUrl": team.get::<Option<String>, _>("logo_url")
        })
    })
    .collect::<Vec<_>>();

    Ok(json!({
        "id": user_id,
        "username": response_username.unwrap_or(&stored_username),
        "role": role,
        "createdAt": created_at,
        "isAdmin": role == "admin",
        "twoFactorRequired": two_factor_required,
        "passwordChangeRequired": !cloud_mode()
            && user_id.to_string() == SYSTEM_USER_ID
            && password == SYSTEM_USER_PASSWORD_HASH,
        "teams": teams
    }))
}

fn me_payload(token: &str, user: Value, api_key: Option<(Uuid, String)>) -> Value {
    if let Some((id, name)) = api_key {
        json!({
            "token": token,
            "user": user,
            "authType": "api-key",
            "apiKey": { "id": id, "name": name }
        })
    } else {
        let password_change_required = user
            .get("passwordChangeRequired")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        json!({
            "token": token,
            "user": user,
            "authType": "session",
            "twoFactorSetupPending": false,
            "passwordChangeRequired": password_change_required
        })
    }
}

pub(super) async fn current_auth_response(pool: &PgPool, headers: &HeaderMap) -> Response {
    let Some(token) = bearer_token(headers) else {
        return with_rust_header(StatusCode::UNAUTHORIZED.into_response());
    };
    let Some((user_id, _)) = check_bearer_auth(pool, headers, "/api/me").await else {
        return with_rust_header(StatusCode::UNAUTHORIZED.into_response());
    };
    let user = match user_json(pool, user_id, None).await {
        Ok(user) => user,
        Err(sqlx::Error::RowNotFound) => {
            return with_rust_header(StatusCode::NOT_FOUND.into_response());
        }
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    let api_key = if token.starts_with(API_KEY_PREFIX) {
        match sqlx::query(
            "SELECT api_key_id, name FROM api_key WHERE key_hash = $1 AND user_id = $2",
        )
        .bind(sha512_hex(&[token]))
        .bind(user_id)
        .fetch_optional(pool)
        .await
        {
            Ok(Some(row)) => Some((
                row.get::<Uuid, _>("api_key_id"),
                row.get::<String, _>("name"),
            )),
            Ok(None) => return with_rust_header(StatusCode::UNAUTHORIZED.into_response()),
            Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        }
    } else {
        None
    };
    json_response(me_payload(token, user, api_key))
}

pub(super) async fn verify_auth_response(pool: &PgPool, headers: &HeaderMap) -> Response {
    let Some((user_id, _)) = check_bearer_auth(pool, headers, "/api/auth/verify").await else {
        return with_rust_header(StatusCode::UNAUTHORIZED.into_response());
    };
    match user_json(pool, user_id, None).await {
        Ok(user) => json_response(user),
        Err(sqlx::Error::RowNotFound) => with_rust_header(StatusCode::NOT_FOUND.into_response()),
        Err(_) => with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
}

fn session_token(user_id: Uuid, role: &str, password: &str, setup: bool) -> Option<String> {
    let mut claims = json!({
        "userId": user_id.to_string(),
        "role": role,
        "pwd": sha512_hex(&[password])
    });
    if setup {
        claims["twoFactorSetup"] = json!(true);
    }
    create_secure_token(&claims, &secret_hash())
}

pub(super) async fn handle_login(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Response {
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    let Some(username) = body.get("username").and_then(Value::as_str) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "bad-request",
            "Invalid request body",
        );
    };
    let Some(password) = body.get("password").and_then(Value::as_str) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "bad-request",
            "Invalid request body",
        );
    };

    let row = match sqlx::query(
        r#"
        SELECT user_id, password, role
        FROM "user"
        WHERE LOWER(username) = LOWER($1) AND deleted_at IS NULL
        "#,
    )
    .bind(username)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return api_error(
                StatusCode::UNAUTHORIZED,
                "incorrect-username-password",
                "Incorrect username or password",
            );
        }
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };

    let stored_hash: String = row.get("password");
    if !bcrypt::verify(password, &stored_hash).unwrap_or(false) {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "incorrect-username-password",
            "Incorrect username or password",
        );
    }

    let user_id: Uuid = row.get("user_id");
    let role: String = row.get("role");
    if !cloud_mode() {
        let enabled = match sqlx::query_scalar::<_, bool>(
            r#"
            SELECT COALESCE(
                (SELECT is_enabled FROM two_factor_auth WHERE user_id = $1),
                false
            )
            "#,
        )
        .bind(user_id)
        .fetch_one(pool)
        .await
        {
            Ok(enabled) => enabled,
            Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        };
        if enabled {
            if two_factor_key().is_none() {
                return two_factor_configuration_error();
            }
            let claims = json!({
                "userId": user_id.to_string(),
                "type": PARTIAL_AUTH_TOKEN_TYPE,
                "exp": Utc::now().timestamp() + PARTIAL_TOKEN_SECONDS
            });
            let Some(partial_token) = create_secure_token(&claims, &secret_hash()) else {
                return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
            };
            return json_response(json!({
                "requiresTwoFactor": true,
                "partialToken": partial_token
            }));
        }
    }

    let setup_required = if cloud_mode() {
        false
    } else {
        let reason = match required_reason(pool, user_id).await {
            Ok(reason) => reason,
            Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        };
        if reason.is_some() && two_factor_key().is_none() {
            return two_factor_configuration_error();
        }
        reason.is_some()
    };

    let Some(token) = session_token(user_id, &role, &stored_hash, setup_required) else {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    };
    let user = match user_json(pool, user_id, Some(username)).await {
        Ok(user) => user,
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    json_response(json!({ "token": token, "user": user }))
}

pub(super) async fn handle_create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if cloud_mode() {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    }
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    let Some((user_id, _)) = check_bearer_auth(pool, &headers, "/api/me/api-keys").await else {
        return with_rust_header(StatusCode::UNAUTHORIZED.into_response());
    };
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if name.is_empty() || name.chars().count() > 255 {
        return api_error(
            StatusCode::BAD_REQUEST,
            "bad-request",
            "Invalid API key name",
        );
    }
    let random_part: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    let key = format!("{API_KEY_PREFIX}{random_part}");
    let key_hash = sha512_hex(&[&key]);
    let key_prefix: String = key.chars().take(API_KEY_DISPLAY_LENGTH).collect();
    let id = Uuid::new_v4();
    let now = Utc::now();

    let result = sqlx::query(
        r#"
        INSERT INTO api_key (api_key_id, user_id, name, key_hash, key_prefix, created_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .bind(&key_hash)
    .bind(&key_prefix)
    .bind(now)
    .execute(pool)
    .await;
    if result.is_err() {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }

    json_response(json!({
        "id": id,
        "userId": user_id,
        "name": name,
        "keyPrefix": key_prefix,
        "createdAt": now,
        "key": key
    }))
}

async fn setup_auth(pool: &PgPool, headers: &HeaderMap, path: &str) -> Result<Uuid, Response> {
    check_bearer_auth(pool, headers, path)
        .await
        .map(|(user_id, _)| user_id)
        .ok_or_else(|| with_rust_header(StatusCode::UNAUTHORIZED.into_response()))
}

async fn reserve_attempt(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<(bool, Option<DateTime<Utc>>), sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let row = sqlx::query(
        r#"
        INSERT INTO two_factor_rate_limit
            (id, user_id, attempts, locked_until, updated_at)
        VALUES ($1, $2, 1, NULL, NOW())
        ON CONFLICT (user_id) DO UPDATE SET
            attempts = CASE
                WHEN two_factor_rate_limit.locked_until IS NOT NULL
                 AND two_factor_rate_limit.locked_until <= NOW()
                    THEN 1
                ELSE two_factor_rate_limit.attempts + 1
            END,
            locked_until = CASE
                WHEN two_factor_rate_limit.locked_until > NOW()
                    THEN two_factor_rate_limit.locked_until
                WHEN two_factor_rate_limit.locked_until IS NOT NULL
                 AND two_factor_rate_limit.locked_until <= NOW()
                    THEN NULL
                WHEN two_factor_rate_limit.attempts + 1 >= $3
                    THEN NOW() + ($4::int * INTERVAL '1 minute')
                ELSE NULL
            END,
            updated_at = NOW()
        RETURNING attempts, locked_until
        "#,
    )
    .bind(id)
    .bind(user_id)
    .bind(MAX_TWO_FACTOR_ATTEMPTS)
    .bind(TWO_FACTOR_LOCKOUT_MINUTES)
    .fetch_one(pool)
    .await?;
    let attempts: i32 = row.get("attempts");
    Ok((attempts <= MAX_TWO_FACTOR_ATTEMPTS, row.get("locked_until")))
}

async fn claim_otp(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    otp: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query(
        "DELETE FROM two_factor_otp_used WHERE user_id = $1 AND otp = $2 AND expires_at < NOW()",
    )
    .bind(user_id)
    .bind(otp)
    .execute(&mut **tx)
    .await?;
    Ok(sqlx::query(
        r#"
        INSERT INTO two_factor_otp_used (id, user_id, otp, expires_at)
        VALUES ($1, $2, $3, NOW() + ($4::int * INTERVAL '1 second'))
        ON CONFLICT (user_id, otp) DO NOTHING
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(otp)
    .bind(OTP_REPLAY_SECONDS)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        == 1)
}

async fn reset_rate_limit(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM two_factor_rate_limit WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn invalid_code_response(
    code: &str,
    message: &str,
    locked_until: Option<DateTime<Utc>>,
) -> Response {
    let mut error = json!({
        "code": code,
        "message": message,
        "status": 400
    });
    if let Some(locked_until) = locked_until {
        error["lockedUntil"] = json!(locked_until);
    }
    with_rust_header((StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response())
}

async fn handle_two_factor_status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if cloud_mode() {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    }
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    let user_id = match setup_auth(pool, &headers, "/api/2fa/status").await {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let enabled = match sqlx::query_scalar::<_, bool>(
        r#"
        SELECT COALESCE(
            (SELECT is_enabled FROM two_factor_auth WHERE user_id = $1),
            false
        )
        "#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    {
        Ok(enabled) => enabled,
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    let global = match global_required(pool).await {
        Ok(global) => global,
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    let configured = two_factor_key().is_some();
    let reason = if configured {
        match required_reason(pool, user_id).await {
            Ok(reason) => reason,
            Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        }
    } else {
        None
    };
    json_response(json!({
        "isEnabled": enabled,
        "isRequired": reason.is_some(),
        "requiredReason": reason,
        "isConfigured": configured,
        "globalRequired": global
    }))
}

async fn handle_two_factor_setup_initiate(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if cloud_mode() {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    }
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    let user_id = match setup_auth(pool, &headers, "/api/2fa/setup/initiate").await {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let Some(key) = two_factor_key() else {
        return two_factor_configuration_error();
    };
    let existing_enabled = match sqlx::query_scalar::<_, bool>(
        r#"
        SELECT COALESCE(
            (SELECT is_enabled FROM two_factor_auth WHERE user_id = $1),
            false
        )
        "#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    {
        Ok(enabled) => enabled,
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    if existing_enabled {
        return api_error(
            StatusCode::BAD_REQUEST,
            "two-factor-error-already-enabled",
            "2FA is already enabled",
        );
    }
    let username = match sqlx::query_scalar::<_, String>(
        r#"SELECT username FROM "user" WHERE user_id = $1 AND deleted_at IS NULL"#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(username)) => username,
        Ok(None) => return api_error(StatusCode::BAD_REQUEST, "bad-request", "User not found"),
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };

    let generated = Secret::generate_secret();
    let secret = generated.to_encoded().to_string();
    let Some(encrypted) = encrypt_two_factor_secret_with_key(&secret, &key) else {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    };
    let Some(totp) = new_totp(&secret, &username) else {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    };
    let Ok(qr_code) = totp.get_qr_base64() else {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    };
    let qr_code_data_url = format!("data:image/png;base64,{qr_code}");
    let result = sqlx::query(
        r#"
        INSERT INTO two_factor_auth
            (id, user_id, secret, is_enabled, created_at, updated_at)
        VALUES ($1, $2, $3, false, NOW(), NOW())
        ON CONFLICT (user_id) DO UPDATE SET
            secret = EXCLUDED.secret,
            is_enabled = false,
            updated_at = NOW()
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(encrypted)
    .execute(pool)
    .await;
    if result.is_err() {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    json_response(json!({
        "qrCodeDataUrl": qr_code_data_url,
        "manualKey": secret
    }))
}

async fn handle_two_factor_setup_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if cloud_mode() {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    }
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    let user_id = match setup_auth(pool, &headers, "/api/2fa/setup/cancel").await {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    if sqlx::query("DELETE FROM two_factor_auth WHERE user_id = $1 AND is_enabled = false")
        .bind(user_id)
        .execute(pool)
        .await
        .is_err()
    {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    json_response(json!({ "ok": true }))
}

fn token_body(body: &Value) -> Option<&str> {
    let object = body.as_object()?;
    if object.len() != 1 {
        return None;
    }
    let token = object.get("token")?.as_str()?;
    (token.len() == 6).then_some(token)
}

fn generate_backup_codes() -> Option<(Vec<String>, Vec<String>)> {
    let mut plaintext = Vec::with_capacity(BACKUP_CODE_COUNT);
    let mut hashed = Vec::with_capacity(BACKUP_CODE_COUNT);
    for _ in 0..BACKUP_CODE_COUNT {
        let mut first = [0u8; 8];
        let mut second = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut first);
        rand::thread_rng().fill_bytes(&mut second);
        let code = format!("{}-{}", hex::encode_upper(first), hex::encode_upper(second));
        hashed.push(bcrypt::hash(&code, 10).ok()?);
        plaintext.push(code);
    }
    Some((plaintext, hashed))
}

async fn handle_two_factor_setup_confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if cloud_mode() {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    }
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    let user_id = match setup_auth(pool, &headers, "/api/2fa/setup/confirm").await {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let Some(token) = token_body(&body) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "bad-request",
            "Invalid request body",
        );
    };
    let Some(key) = two_factor_key() else {
        return two_factor_configuration_error();
    };
    let record = match sqlx::query(
        "SELECT secret FROM two_factor_auth WHERE user_id = $1 AND is_enabled = false",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(record)) => record,
        Ok(None) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "two-factor-error-no-pending-setup",
                "No pending 2FA setup found",
            );
        }
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    let (allowed, locked_until) = match reserve_attempt(pool, user_id).await {
        Ok(result) => result,
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    if !allowed {
        return rate_limit_error(StatusCode::TOO_MANY_REQUESTS, locked_until);
    }
    let Some(secret) =
        decrypt_two_factor_secret_with_key(record.get::<String, _>("secret").as_str(), &key)
    else {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    };
    if !new_totp(&secret, "user").is_some_and(|totp| totp.check_current(token).unwrap_or(false)) {
        return invalid_code_response(
            "two-factor-error-invalid-code",
            "Invalid verification code",
            locked_until,
        );
    }

    let Some((plaintext, hashed)) = generate_backup_codes() else {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    };
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    match claim_otp(&mut tx, user_id, token).await {
        Ok(true) => {}
        Ok(false) => {
            return invalid_code_response("two-factor-error-code-used", "Code already used", None);
        }
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
    let writes = async {
        let enabled = sqlx::query(
            "UPDATE two_factor_auth SET is_enabled = true, updated_at = NOW() WHERE user_id = $1 AND is_enabled = false",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if enabled.rows_affected() != 1 {
            return Err(sqlx::Error::RowNotFound);
        }
        sqlx::query("DELETE FROM two_factor_backup_code WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        for code_hash in &hashed {
            sqlx::query(
                r#"
                INSERT INTO two_factor_backup_code
                    (id, user_id, code_hash, used, created_at)
                VALUES ($1, $2, $3, false, NOW())
                "#,
            )
            .bind(Uuid::new_v4().to_string())
            .bind(user_id)
            .bind(code_hash)
            .execute(&mut *tx)
            .await?;
        }
        reset_rate_limit(&mut tx, user_id).await?;
        Ok::<(), sqlx::Error>(())
    }
    .await;
    if writes.is_err() || tx.commit().await.is_err() {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    json_response(json!({ "backupCodes": plaintext }))
}

async fn handle_two_factor_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if cloud_mode() {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    }
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    let Some(auth) = bearer_token(&headers) else {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "two-factor-error-missing-token",
            "Missing partial authentication token",
        );
    };
    if auth.starts_with(API_KEY_PREFIX) {
        return with_rust_header(StatusCode::UNAUTHORIZED.into_response());
    }
    let Some(claims) = parse_secure_token(auth, &secret_hash()) else {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "two-factor-error-invalid-partial-token",
            "Invalid partial authentication token",
        );
    };
    if claims.get("type").and_then(Value::as_str) != Some(PARTIAL_AUTH_TOKEN_TYPE) {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "two-factor-error-invalid-partial-token",
            "Invalid partial authentication token",
        );
    }
    let Some(user_id) = claims
        .get("userId")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
    else {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "two-factor-error-invalid-partial-token",
            "Invalid partial authentication token",
        );
    };
    let Some(key) = two_factor_key() else {
        return two_factor_configuration_error();
    };
    let object = match body.as_object() {
        Some(object) if object.len() == 1 => object,
        _ => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "bad-request",
                "Invalid request body",
            )
        }
    };
    let token = object
        .get("token")
        .and_then(Value::as_str)
        .filter(|token| token.len() == 6);
    let backup_code = object
        .get("backupCode")
        .and_then(Value::as_str)
        .filter(|code| !code.is_empty());
    if token.is_none() == backup_code.is_none() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "bad-request",
            "Invalid request body",
        );
    }

    let user = match sqlx::query(
        r#"
        SELECT password, role
        FROM "user"
        WHERE user_id = $1 AND deleted_at IS NULL
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(user)) => user,
        Ok(None) => return with_rust_header(StatusCode::UNAUTHORIZED.into_response()),
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    let two_factor = match sqlx::query(
        "SELECT secret FROM two_factor_auth WHERE user_id = $1 AND is_enabled = true",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(record)) => record,
        Ok(None) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "two-factor-error-not-enabled",
                "2FA not enabled for this user",
            );
        }
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    let (allowed, locked_until) = match reserve_attempt(pool, user_id).await {
        Ok(result) => result,
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    if !allowed {
        return rate_limit_error(StatusCode::TOO_MANY_REQUESTS, locked_until);
    }

    if let Some(backup_code) = backup_code {
        let records = match sqlx::query(
            "SELECT id, code_hash FROM two_factor_backup_code WHERE user_id = $1 AND used = false",
        )
        .bind(user_id)
        .fetch_all(pool)
        .await
        {
            Ok(records) => records,
            Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        };
        let mut matched_id = None;
        let normalized = backup_code.to_uppercase();
        for record in records {
            if bcrypt::verify(&normalized, record.get::<String, _>("code_hash").as_str())
                .unwrap_or(false)
            {
                matched_id = Some(record.get::<String, _>("id"));
                break;
            }
        }
        let consumed = if let Some(id) = matched_id {
            sqlx::query(
                "UPDATE two_factor_backup_code SET used = true WHERE id = $1 AND used = false",
            )
            .bind(id)
            .execute(pool)
            .await
            .map(|result| result.rows_affected() == 1)
            .unwrap_or(false)
        } else {
            false
        };
        if !consumed {
            return invalid_code_response(
                "two-factor-error-invalid-backup-code",
                "Invalid backup code",
                locked_until,
            );
        }
    } else if let Some(token) = token {
        let Some(secret) = decrypt_two_factor_secret_with_key(
            two_factor.get::<String, _>("secret").as_str(),
            &key,
        ) else {
            return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        };
        if !new_totp(&secret, "user").is_some_and(|totp| totp.check_current(token).unwrap_or(false))
        {
            return invalid_code_response(
                "two-factor-error-invalid-code",
                "Invalid verification code",
                locked_until,
            );
        }
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        };
        match claim_otp(&mut tx, user_id, token).await {
            Ok(true) => {
                if tx.commit().await.is_err() {
                    return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
                }
            }
            Ok(false) => {
                return invalid_code_response(
                    "two-factor-error-code-used",
                    "Code already used",
                    None,
                );
            }
            Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        }
    }

    if sqlx::query("DELETE FROM two_factor_rate_limit WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .is_err()
    {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    let role: String = user.get("role");
    let password: String = user.get("password");
    let Some(full_token) = session_token(user_id, &role, &password, false) else {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    };
    let user = match user_json(pool, user_id, None).await {
        Ok(user) => user,
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    json_response(json!({ "token": full_token, "user": user }))
}

async fn handle_two_factor_disable(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if cloud_mode() {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    }
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    let Some((user_id, _)) = check_bearer_auth(pool, &headers, "/api/2fa/disable").await else {
        return with_rust_header(StatusCode::UNAUTHORIZED.into_response());
    };
    let Some(password) = body.get("password").and_then(Value::as_str) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "bad-request",
            "Invalid request body",
        );
    };
    let Some(token) = body
        .get("token")
        .and_then(Value::as_str)
        .filter(|token| token.len() == 6)
    else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "bad-request",
            "Invalid request body",
        );
    };
    let Some(key) = two_factor_key() else {
        return two_factor_configuration_error();
    };
    let reason = match required_reason(pool, user_id).await {
        Ok(reason) => reason,
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    if reason.is_some() {
        return api_error(
            StatusCode::FORBIDDEN,
            "two-factor-error-disable-not-allowed",
            "2FA is required and cannot be disabled",
        );
    }
    let user = match sqlx::query(
        r#"SELECT password FROM "user" WHERE user_id = $1 AND deleted_at IS NULL"#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(user)) => user,
        Ok(None) => return with_rust_header(StatusCode::UNAUTHORIZED.into_response()),
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    if !bcrypt::verify(password, user.get::<String, _>("password").as_str()).unwrap_or(false) {
        return api_error(
            StatusCode::BAD_REQUEST,
            "two-factor-error-incorrect-password",
            "Incorrect password",
        );
    }
    let record = match sqlx::query(
        "SELECT secret FROM two_factor_auth WHERE user_id = $1 AND is_enabled = true",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(record)) => record,
        Ok(None) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "two-factor-error-not-enabled",
                "2FA is not enabled",
            );
        }
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    let (allowed, locked_until) = match reserve_attempt(pool, user_id).await {
        Ok(result) => result,
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    if !allowed {
        return rate_limit_error(StatusCode::TOO_MANY_REQUESTS, locked_until);
    }
    let Some(secret) =
        decrypt_two_factor_secret_with_key(record.get::<String, _>("secret").as_str(), &key)
    else {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    };
    if !new_totp(&secret, "user").is_some_and(|totp| totp.check_current(token).unwrap_or(false)) {
        return invalid_code_response(
            "two-factor-error-invalid-code",
            "Invalid verification code",
            locked_until,
        );
    }

    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    match claim_otp(&mut tx, user_id, token).await {
        Ok(true) => {}
        Ok(false) => {
            return invalid_code_response("two-factor-error-code-used", "Code already used", None);
        }
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
    let writes = async {
        let deleted =
            sqlx::query("DELETE FROM two_factor_auth WHERE user_id = $1 AND is_enabled = true")
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        if deleted.rows_affected() != 1 {
            return Err(sqlx::Error::RowNotFound);
        }
        sqlx::query("DELETE FROM two_factor_backup_code WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        reset_rate_limit(&mut tx, user_id).await?;
        Ok::<(), sqlx::Error>(())
    }
    .await;
    if writes.is_err() || tx.commit().await.is_err() {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    json_response(json!({ "ok": true }))
}

async fn require_admin(pool: &PgPool, headers: &HeaderMap, path: &str) -> Result<(), Response> {
    match check_bearer_auth(pool, headers, path).await {
        Some((_, role)) if role == "admin" => Ok(()),
        _ => Err(with_rust_header(StatusCode::UNAUTHORIZED.into_response())),
    }
}

fn required_body(body: &Value) -> Option<bool> {
    body.as_object()?.get("required")?.as_bool()
}

async fn handle_admin_two_factor_global(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if cloud_mode() {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    }
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    if let Err(response) = require_admin(pool, &headers, "/api/admin/2fa/global").await {
        return response;
    }
    let Some(required) = required_body(&body) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "bad-request",
            "Invalid request body",
        );
    };
    if required && two_factor_key().is_none() {
        return two_factor_configuration_error();
    }
    if sqlx::query(
        r#"
        INSERT INTO app_setting ("key", value)
        VALUES ('twoFactorRequiredGlobal', $1)
        ON CONFLICT ("key") DO UPDATE SET value = EXCLUDED.value
        "#,
    )
    .bind(required.to_string())
    .execute(pool)
    .await
    .is_err()
    {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    json_response(json!({ "ok": true, "required": required }))
}

async fn handle_admin_team_two_factor(
    State(state): State<AppState>,
    Path(team_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if cloud_mode() {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    }
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    if let Err(response) = require_admin(pool, &headers, "/api/admin/teams/2fa").await {
        return response;
    }
    let Some(required) = required_body(&body) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "bad-request",
            "Invalid request body",
        );
    };
    if required && two_factor_key().is_none() {
        return two_factor_configuration_error();
    }
    let updated = match sqlx::query(
        "UPDATE team SET two_factor_required = $1, updated_at = NOW() WHERE team_id = $2 AND deleted_at IS NULL",
    )
    .bind(required)
    .bind(team_id)
    .execute(pool)
    .await
    {
        Ok(result) => result.rows_affected(),
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    if updated != 1 {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    }
    json_response(json!({
        "ok": true,
        "teamId": team_id,
        "twoFactorRequired": required
    }))
}

async fn handle_admin_user_two_factor_status(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    if cloud_mode() {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    }
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    if let Err(response) = require_admin(pool, &headers, "/api/admin/users/2fa").await {
        return response;
    }
    let enabled = match sqlx::query_scalar::<_, bool>(
        r#"
        SELECT COALESCE(
            (SELECT is_enabled FROM two_factor_auth WHERE user_id = $1),
            false
        )
        "#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    {
        Ok(enabled) => enabled,
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    json_response(json!({ "isEnabled": enabled }))
}

async fn handle_admin_user_two_factor_required(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if cloud_mode() {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    }
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    if let Err(response) = require_admin(pool, &headers, "/api/admin/users/2fa").await {
        return response;
    }
    let Some(required) = required_body(&body) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "bad-request",
            "Invalid request body",
        );
    };
    if required && two_factor_key().is_none() {
        return two_factor_configuration_error();
    }
    let updated = match sqlx::query(
        r#"
        UPDATE "user"
        SET two_factor_required = $1, updated_at = NOW()
        WHERE user_id = $2 AND deleted_at IS NULL
        "#,
    )
    .bind(required)
    .bind(user_id)
    .execute(pool)
    .await
    {
        Ok(result) => result.rows_affected(),
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    if updated != 1 {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    }
    json_response(json!({
        "ok": true,
        "userId": user_id,
        "twoFactorRequired": required
    }))
}

async fn handle_admin_user_two_factor_reset(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    if cloud_mode() {
        return with_rust_header(StatusCode::NOT_FOUND.into_response());
    }
    let Some(pool) = &state.db else {
        return with_rust_header(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    if let Err(response) = require_admin(pool, &headers, "/api/admin/users/2fa").await {
        return response;
    }
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };
    let reset = async {
        let two_factor_auth = sqlx::query("DELETE FROM two_factor_auth WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let backup_codes = sqlx::query("DELETE FROM two_factor_backup_code WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let otp_used = sqlx::query("DELETE FROM two_factor_otp_used WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let rate_limit = sqlx::query("DELETE FROM two_factor_rate_limit WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        Ok::<_, sqlx::Error>((two_factor_auth, backup_codes, otp_used, rate_limit))
    }
    .await;
    let Ok((two_factor_auth, backup_codes, otp_used, rate_limit)) = reset else {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    };
    if tx.commit().await.is_err() {
        return with_rust_header(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    json_response(json!({
        "ok": true,
        "userId": user_id,
        "reset": {
            "twoFactorAuth": two_factor_auth,
            "backupCodes": backup_codes,
            "otpUsed": otp_used,
            "rateLimit": rate_limit
        }
    }))
}

pub(super) fn register_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route("/api/2fa/status", get(handle_two_factor_status))
        .route("/api/2fa/verify", post(handle_two_factor_verify))
        .route(
            "/api/2fa/setup/initiate",
            post(handle_two_factor_setup_initiate),
        )
        .route(
            "/api/2fa/setup/confirm",
            post(handle_two_factor_setup_confirm),
        )
        .route(
            "/api/2fa/setup/cancel",
            post(handle_two_factor_setup_cancel),
        )
        .route("/api/2fa/disable", post(handle_two_factor_disable))
        .route(
            "/api/admin/2fa/global",
            post(handle_admin_two_factor_global),
        )
        .route(
            "/api/admin/teams/{team_id}/2fa",
            post(handle_admin_team_two_factor),
        )
        .route(
            "/api/admin/users/{user_id}/2fa",
            get(handle_admin_user_two_factor_status)
                .post(handle_admin_user_two_factor_required)
                .delete(handle_admin_user_two_factor_reset),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_tokens_reject_tampering_and_expiry() {
        let secret = "test-secret";
        let user_id = Uuid::nil().to_string();
        let token = create_secure_token(
            &json!({ "userId": user_id.clone(), "exp": Utc::now().timestamp() + 60 }),
            secret,
        )
        .unwrap();
        assert_eq!(
            parse_secure_token(&token, secret)
                .unwrap()
                .get("userId")
                .and_then(Value::as_str),
            Some(user_id.as_str())
        );

        let mut tampered = token.into_bytes();
        tampered[20] ^= 1;
        assert!(parse_secure_token(&String::from_utf8(tampered).unwrap(), secret).is_none());

        let expired = create_secure_token(
            &json!({ "userId": Uuid::nil(), "exp": Utc::now().timestamp() - 1 }),
            secret,
        )
        .unwrap();
        assert!(parse_secure_token(&expired, secret).is_none());
    }

    #[test]
    fn two_factor_secret_crypto_matches_node_storage_shape() {
        let key = [7u8; 32];
        let encrypted = encrypt_two_factor_secret_with_key("BASE32SECRET", &key).unwrap();
        assert_eq!(encrypted.split(':').count(), 3);
        assert_eq!(
            decrypt_two_factor_secret_with_key(&encrypted, &key).as_deref(),
            Some("BASE32SECRET")
        );
        assert!(decrypt_two_factor_secret_with_key(&encrypted, &[8u8; 32]).is_none());
    }

    #[test]
    fn api_key_and_setup_path_policies_are_bounded() {
        for path in [
            "/api/me/password",
            "/api/me/api-keys",
            "/api/2fa/status",
            "/api/auth/logout",
            "/api/users/abc",
            "/api/admin/users",
        ] {
            assert!(is_api_key_blocked_path(path), "{path}");
        }
        assert!(!is_api_key_blocked_path("/api/me"));
        assert!(!is_api_key_blocked_path("/api/websites/abc/stats"));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bearer token"),
        );
        assert_eq!(bearer_token(&headers), Some("token"));
        assert!(is_setup_route("/api/2fa/status"));
        assert!(is_setup_route("/api/2fa/setup/confirm"));
        assert!(is_setup_route("/api/auth/verify"));
        assert!(!is_setup_route("/api/2fa/disable"));
    }

    #[test]
    fn me_payload_matches_node_session_and_api_key_shapes() {
        let user_id = Uuid::new_v4();
        let user = json!({
            "id": user_id,
            "username": "alice",
            "twoFactorRequired": true,
            "passwordChangeRequired": false
        });
        let session = me_payload("session-token", user.clone(), None);
        assert_eq!(session["token"], "session-token");
        assert_eq!(session["user"]["id"], user_id.to_string());
        assert_eq!(session["user"]["twoFactorRequired"], true);
        assert!(session.get("apiKey").is_none());

        let key_id = Uuid::new_v4();
        let api_key = me_payload("umami_key", user, Some((key_id, "CI pipeline".to_string())));
        assert_eq!(api_key["user"]["id"], user_id.to_string());
        assert_eq!(api_key["apiKey"]["id"], key_id.to_string());
        assert_eq!(api_key["apiKey"]["name"], "CI pipeline");
    }

    #[test]
    fn generated_totp_and_backup_codes_are_usable() {
        let secret = Secret::generate_secret().to_encoded().to_string();
        let totp = new_totp(&secret, "alice").unwrap();
        let token = totp.generate_current().unwrap();
        assert!(totp.check_current(&token).unwrap());
        let qr = format!("data:image/png;base64,{}", totp.get_qr_base64().unwrap());
        assert!(qr.starts_with("data:image/png;base64,"));

        let (plaintext, hashed) = generate_backup_codes().unwrap();
        assert_eq!(plaintext.len(), BACKUP_CODE_COUNT);
        assert_eq!(hashed.len(), BACKUP_CODE_COUNT);
        assert!(bcrypt::verify(&plaintext[0], &hashed[0]).unwrap());
    }
}
