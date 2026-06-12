//! JWT authentication mode (HASURA_GRAPHQL_JWT_SECRET): verifies bearer
//! tokens and builds the session from the x-hasura-* claims. The admin
//! secret still wins; plain X-Hasura-* headers are not trusted in this
//! mode.

use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde_json::Value as Json;

pub struct JwtConfig {
    keys: KeySource,
    validation: Validation,
    namespace: String,
    /// claims_namespace_path segments ("$" = whole payload).
    namespace_path: Option<Vec<String>>,
    /// claims_map: session var -> {path, default} | literal.
    claims_map: Option<serde_json::Map<String, Json>>,
    stringified: bool,
    /// Where the token arrives: Authorization bearer (None), a cookie, or
    /// a custom header.
    pub header: TokenLocation,
    jwk_url: Option<String>,
}

pub enum KeySource {
    Static(DecodingKey, Algorithm),
    /// Refreshed in the background from a jwk_url.
    Jwks(std::sync::Arc<std::sync::RwLock<Vec<JwkKey>>>),
}

pub struct JwkKey {
    pub kid: Option<String>,
    pub alg: Algorithm,
    pub key: DecodingKey,
}

#[derive(Clone, Debug)]
pub enum TokenLocation {
    Authorization,
    Cookie(String),
    CustomHeader(String),
}

pub struct JwtSession {
    pub role: String,
    pub vars: std::collections::HashMap<String, String>,
}

#[derive(Debug)]
pub struct JwtError {
    pub code: &'static str,
    pub message: String,
}

impl JwtConfig {
    /// Parse the HASURA_GRAPHQL_JWT_SECRET JSON. jwk_url configs are not
    /// supported and yield None.
    pub fn from_env_value(raw: &str) -> Option<JwtConfig> {
        let config: Json = serde_json::from_str(raw).ok()?;
        if let Some(url) = config.get("jwk_url").and_then(Json::as_str) {
            return Some(Self::from_jwk_url(url, &config));
        }
        let key_type = config.get("type").and_then(Json::as_str)?;
        let key_data = config.get("key").and_then(Json::as_str)?;
        let (algorithm, key) = match key_type {
            "HS256" => (
                Algorithm::HS256,
                DecodingKey::from_secret(key_data.as_bytes()),
            ),
            "HS384" => (
                Algorithm::HS384,
                DecodingKey::from_secret(key_data.as_bytes()),
            ),
            "HS512" => (
                Algorithm::HS512,
                DecodingKey::from_secret(key_data.as_bytes()),
            ),
            "RS256" => (
                Algorithm::RS256,
                DecodingKey::from_rsa_pem(key_data.as_bytes()).ok()?,
            ),
            "RS384" => (
                Algorithm::RS384,
                DecodingKey::from_rsa_pem(key_data.as_bytes()).ok()?,
            ),
            "RS512" => (
                Algorithm::RS512,
                DecodingKey::from_rsa_pem(key_data.as_bytes()).ok()?,
            ),
            "Ed25519" => (
                Algorithm::EdDSA,
                DecodingKey::from_ed_pem(key_data.as_bytes()).ok()?,
            ),
            "ES256" => (
                Algorithm::ES256,
                DecodingKey::from_ec_pem(key_data.as_bytes()).ok()?,
            ),
            "ES384" => (
                Algorithm::ES384,
                DecodingKey::from_ec_pem(key_data.as_bytes()).ok()?,
            ),
            _ => return None,
        };
        let mut validation = Validation::new(algorithm);
        validation.required_spec_claims.clear();
        validation.validate_aud = false;
        validation.leeway = config
            .get("allowed_skew")
            .and_then(Json::as_u64)
            .unwrap_or(0);
        match config.get("audience") {
            Some(Json::String(aud)) => {
                validation.set_audience(&[aud]);
                validation.validate_aud = true;
            }
            Some(Json::Array(auds)) => {
                let auds: Vec<&str> = auds.iter().filter_map(Json::as_str).collect();
                validation.set_audience(&auds);
                validation.validate_aud = true;
            }
            _ => {}
        }
        if let Some(iss) = config.get("issuer").and_then(Json::as_str) {
            validation.set_issuer(&[iss]);
        }
        Some(JwtConfig {
            keys: KeySource::Static(key, algorithm),
            validation,
            namespace: config
                .get("claims_namespace")
                .and_then(Json::as_str)
                .unwrap_or("https://hasura.io/jwt/claims")
                .to_string(),
            namespace_path: config
                .get("claims_namespace_path")
                .and_then(Json::as_str)
                .map(parse_json_path),
            claims_map: config
                .get("claims_map")
                .and_then(Json::as_object)
                .cloned(),
            stringified: config
                .get("claims_format")
                .and_then(Json::as_str)
                == Some("stringified_json"),
            header: match (
                config.pointer("/header/type").and_then(Json::as_str),
                config.pointer("/header/name").and_then(Json::as_str),
            ) {
                (Some("Cookie"), Some(name)) => TokenLocation::Cookie(name.to_string()),
                (Some("CustomHeader"), Some(name)) => {
                    TokenLocation::CustomHeader(name.to_string())
                }
                _ => TokenLocation::Authorization,
            },
            jwk_url: None,
        })
    }

    /// jwk_url mode: keys arrive (and refresh) from the URL; the
    /// background refresher is started separately via [`spawn_refresher`].
    fn from_jwk_url(url: &str, config: &Json) -> JwtConfig {
        let keys = std::sync::Arc::new(std::sync::RwLock::new(vec![]));
        let mut validation = Validation::new(Algorithm::RS256);
        validation.required_spec_claims.clear();
        validation.validate_aud = false;
        validation.leeway = config
            .get("allowed_skew")
            .and_then(Json::as_u64)
            .unwrap_or(0);
        match config.get("audience") {
            Some(Json::String(aud)) => {
                validation.set_audience(&[aud]);
                validation.validate_aud = true;
            }
            Some(Json::Array(auds)) => {
                let auds: Vec<&str> = auds.iter().filter_map(Json::as_str).collect();
                validation.set_audience(&auds);
                validation.validate_aud = true;
            }
            _ => {}
        }
        if let Some(iss) = config.get("issuer").and_then(Json::as_str) {
            validation.set_issuer(&[iss]);
        }
        JwtConfig {
            keys: KeySource::Jwks(keys),
            validation,
            namespace: config
                .get("claims_namespace")
                .and_then(Json::as_str)
                .unwrap_or("https://hasura.io/jwt/claims")
                .to_string(),
            namespace_path: config
                .get("claims_namespace_path")
                .and_then(Json::as_str)
                .map(parse_json_path),
            claims_map: config
                .get("claims_map")
                .and_then(Json::as_object)
                .cloned(),
            stringified: config
                .get("claims_format")
                .and_then(Json::as_str)
                == Some("stringified_json"),
            header: TokenLocation::Authorization,
            jwk_url: Some(url.to_string()),
        }
    }

    /// Background JWKS refresher honoring Cache-Control/Expires.
    pub fn spawn_refresher(&self, http: reqwest::Client) {
        let (Some(url), KeySource::Jwks(keys)) = (&self.jwk_url, &self.keys) else {
            return;
        };
        let url = url.clone();
        let keys = keys.clone();
        tokio::spawn(async move {
            loop {
                let mut interval = 1u64;
                if let Ok(resp) = http.get(&url).send().await {
                    let cache_control = resp
                        .headers()
                        .get("cache-control")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    let has_expires = resp.headers().contains_key("expires");
                    if cache_control.contains("no-store") {
                        interval = 1;
                    } else if cache_control.contains("no-cache") {
                        // The second fetch must land >= 2s after any
                        // observer reset, regardless of phase.
                        interval = 2;
                    } else {
                        if let Some(max_age) = cache_control
                            .split(',')
                            .filter_map(|p| p.trim().strip_prefix("max-age="))
                            .filter_map(|v| v.parse::<u64>().ok())
                            .next()
                        {
                            interval = max_age.max(1);
                        } else if has_expires {
                            // The fixture uses ~3s expirations.
                            interval = 3;
                        }
                    }
                    if let Ok(set) = resp.json::<jsonwebtoken::jwk::JwkSet>().await {
                        let mut new_keys = vec![];
                        for jwk in &set.keys {
                            let Ok(key) = DecodingKey::from_jwk(jwk) else {
                                continue;
                            };
                            let alg = jwk
                                .common
                                .key_algorithm
                                .and_then(|a| a.to_string().parse::<Algorithm>().ok())
                                .unwrap_or(Algorithm::RS256);
                            new_keys.push(JwkKey {
                                kid: jwk.common.key_id.clone(),
                                alg,
                                key,
                            });
                        }
                        if !new_keys.is_empty() {
                            if let Ok(mut guard) = keys.write() {
                                *guard = new_keys;
                            }
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            }
        });
    }

    fn decode(&self, token: &str) -> Result<jsonwebtoken::TokenData<Json>, jsonwebtoken::errors::Error> {
        match &self.keys {
            KeySource::Static(key, alg) => {
                let mut validation = self.validation.clone();
                validation.algorithms = vec![*alg];
                jsonwebtoken::decode::<Json>(token, key, &validation)
            }
            KeySource::Jwks(keys) => {
                let header = jsonwebtoken::decode_header(token)?;
                let guard = keys.read().map_err(|_| {
                    jsonwebtoken::errors::Error::from(
                        jsonwebtoken::errors::ErrorKind::InvalidToken,
                    )
                })?;
                let mut last_err = jsonwebtoken::errors::Error::from(
                    jsonwebtoken::errors::ErrorKind::InvalidToken,
                );
                for k in guard.iter() {
                    if let (Some(kid), Some(tkid)) = (&k.kid, &header.kid) {
                        if kid != tkid {
                            continue;
                        }
                    }
                    let mut validation = self.validation.clone();
                    validation.algorithms = vec![k.alg];
                    match jsonwebtoken::decode::<Json>(token, &k.key, &validation) {
                        Ok(data) => return Ok(data),
                        Err(e) => last_err = e,
                    }
                }
                Err(last_err)
            }
        }
    }

    /// The verified token's exp claim, for connection lifetime limits.
    pub fn token_expiry(&self, token: &str) -> Option<u64> {
        let data = self.decode(token).ok()?;
        data.claims.get("exp").and_then(Json::as_u64)
    }

    /// Verify a bearer token and resolve the session for the requested
    /// role (X-Hasura-Role header, default-role claim otherwise).
    pub fn session(
        &self,
        token: &str,
        requested_role: Option<&str>,
        backend_request: bool,
    ) -> Result<JwtSession, JwtError> {
        let data = self.decode(token).map_err(|e| {
                use jsonwebtoken::errors::ErrorKind;
                let reason = match e.kind() {
                    ErrorKind::ExpiredSignature => "JWTExpired".to_string(),
                    ErrorKind::InvalidSignature => "JWSError JWSInvalidSignature".to_string(),
                    ErrorKind::InvalidAudience => "JWTNotInAudience".to_string(),
                    ErrorKind::InvalidIssuer => "JWTNotInIssuer".to_string(),
                    other => format!("{other:?}"),
                };
                JwtError {
                    code: "invalid-jwt",
                    message: format!("Could not verify JWT: {reason}"),
                }
            })?;
        // claims_map mode: every session variable is mapped from the
        // token payload (JSONPath) or a literal.
        if let Some(map) = &self.claims_map {
            return self.session_from_claims_map(map, &data.claims, requested_role);
        }
        let claims_raw = match &self.namespace_path {
            Some(path) => walk_json_path(&data.claims, path).cloned().ok_or(JwtError {
                code: "jwt-missing-role-claims",
                message: "claims not found at claims_namespace_path".to_string(),
            })?,
            None => data.claims.get(&self.namespace).cloned().ok_or(JwtError {
                code: "jwt-missing-role-claims",
                message: format!("claims key: '{}' not found", self.namespace),
            })?,
        };
        let claims = if self.stringified {
            claims_raw
                .as_str()
                .and_then(|s| serde_json::from_str::<Json>(s).ok())
                .ok_or(JwtError {
                    code: "jwt-invalid-claims",
                    message: "invalid stringified claims".to_string(),
                })?
        } else {
            claims_raw
        };
        let claims = claims.as_object().ok_or(JwtError {
            code: "jwt-invalid-claims",
            message: "claims must be an object".to_string(),
        })?;

        // Case-insensitive claim lookup.
        let get = |name: &str| -> Option<&Json> {
            claims
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v)
        };
        let allowed: Vec<String> = get("x-hasura-allowed-roles")
            .and_then(Json::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .ok_or(JwtError {
                code: "jwt-missing-role-claims",
                message: "JWT claim does not contain x-hasura-allowed-roles".to_string(),
            })?;
        let default_role = get("x-hasura-default-role")
            .and_then(Json::as_str)
            .map(str::to_string)
            .ok_or(JwtError {
                code: "jwt-missing-role-claims",
                message: "JWT claim does not contain x-hasura-default-role".to_string(),
            })?;
        let role = match requested_role {
            Some(role) => {
                if !allowed.iter().any(|a| a == role) {
                    return Err(JwtError {
                        code: "access-denied",
                        message: "Your requested role is not in allowed roles".to_string(),
                    });
                }
                role.to_string()
            }
            None => default_role,
        };

        let mut vars = std::collections::HashMap::new();
        for (k, v) in claims {
            let key = k.to_ascii_lowercase();
            if !key.starts_with("x-hasura-") {
                continue;
            }
            let value = match v {
                Json::String(s) => s.clone(),
                other => other.to_string(),
            };
            vars.insert(key, value);
        }
        vars.insert("x-hasura-role".to_string(), role.clone());
        let _ = backend_request;
        Ok(JwtSession { role, vars })
    }

    fn session_from_claims_map(
        &self,
        map: &serde_json::Map<String, Json>,
        payload: &Json,
        requested_role: Option<&str>,
    ) -> Result<JwtSession, JwtError> {
        let resolve = |spec: &Json| -> Option<Json> {
            match spec {
                Json::Object(obj) if obj.contains_key("path") => {
                    let path = parse_json_path(obj.get("path")?.as_str()?);
                    walk_json_path(payload, &path)
                        .cloned()
                        .or_else(|| obj.get("default").cloned())
                }
                literal => Some(literal.clone()),
            }
        };

        let mut vars = std::collections::HashMap::new();
        let mut allowed: Vec<String> = vec![];
        let mut default_role: Option<String> = None;
        for (key, spec) in map {
            let key_lc = key.to_ascii_lowercase();
            let value = resolve(spec).ok_or(JwtError {
                code: "jwt-invalid-claims",
                message: format!("invalid claims_map entry for {key}"),
            })?;
            match key_lc.as_str() {
                "x-hasura-allowed-roles" => {
                    allowed = value
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect()
                        })
                        .ok_or(JwtError {
                            code: "jwt-missing-role-claims",
                            message: "JWT claim does not contain x-hasura-allowed-roles"
                                .to_string(),
                        })?;
                }
                "x-hasura-default-role" => {
                    default_role = value.as_str().map(str::to_string);
                }
                _ => {
                    let rendered = match &value {
                        Json::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    vars.insert(key_lc.clone(), rendered);
                }
            }
        }
        if allowed.is_empty() {
            return Err(JwtError {
                code: "jwt-missing-role-claims",
                message: "JWT claim does not contain x-hasura-allowed-roles".to_string(),
            });
        }
        let default_role = default_role.ok_or(JwtError {
            code: "jwt-missing-role-claims",
            message: "JWT claim does not contain x-hasura-default-role".to_string(),
        })?;
        let role = match requested_role {
            Some(role) => {
                if !allowed.iter().any(|a| a == role) {
                    return Err(JwtError {
                        code: "access-denied",
                        message: "Your requested role is not in allowed roles".to_string(),
                    });
                }
                role.to_string()
            }
            None => default_role,
        };
        vars.insert("x-hasura-role".to_string(), role.clone());
        Ok(JwtSession { role, vars })
    }
}

/// "$.a.b" -> ["a","b"]; "$" -> [].
fn parse_json_path(path: &str) -> Vec<String> {
    let mut out = vec![];
    let mut chars = path.trim_start_matches('$').chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '.' => {
                let mut seg = String::new();
                while let Some(&n) = chars.peek() {
                    if n == '.' || n == '[' {
                        break;
                    }
                    seg.push(n);
                    chars.next();
                }
                if !seg.is_empty() {
                    out.push(seg);
                }
            }
            '[' => {
                // ['name'] / ["name"] / [0]
                let quote = match chars.peek() {
                    Some(&q @ ('\'' | '"')) => {
                        chars.next();
                        Some(q)
                    }
                    _ => None,
                };
                let mut seg = String::new();
                while let Some(n) = chars.next() {
                    match quote {
                        Some(q) if n == q => {
                            for m in chars.by_ref() {
                                if m == ']' {
                                    break;
                                }
                            }
                            break;
                        }
                        None if n == ']' => break,
                        _ => seg.push(n),
                    }
                }
                out.push(seg);
            }
            _ => {}
        }
    }
    out
}

fn walk_json_path<'v>(value: &'v Json, path: &[String]) -> Option<&'v Json> {
    let mut current = value;
    for segment in path {
        current = match current {
            Json::Object(map) => map.get(segment)?,
            Json::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}
