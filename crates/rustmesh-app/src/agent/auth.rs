//! rustmesh-owned public OAuth client. Tokens deliberately remain in memory.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use rand::{rngs::OsRng, RngCore};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use url::Url;

const ISSUER: &str = "https://auth.openai.com";
const RESOURCE: &str = "https://api.openai.com/v1";
const TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
const DIRECT: &str = "chatgpt.tokens.use.direct";

#[derive(Clone, Serialize, Deserialize)]
pub struct Registration {
    pub client_id: String,
    pub subject: String,
    pub email: String,
}

// Never derive Debug or Serialize for credentials.
pub struct Credentials {
    pub registration: Registration,
    pub access_token: String,
    refresh_token: String,
    id_token: String,
    pub expires_at: u64,
    pub direct: bool,
}

#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    id_token: String,
    token_type: String,
    expires_in: u64,
    scope: Option<String>,
}

#[derive(Clone, Deserialize)]
struct Claims {
    sub: String,
    nonce: Option<String>,
    #[serde(default)]
    email: String,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn random() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
fn client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Cannot initialize HTTPS client".into())
}
pub fn load_registrations(dir: &Path) -> Vec<Registration> {
    read_registrations(dir).unwrap_or_default()
}
fn read_registrations(dir: &Path) -> Result<Vec<Registration>, String> {
    match fs::read(dir.join("registrations.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| "Saved account metadata is invalid; repair it before signing in".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(_) => Err("Cannot read saved account metadata".into()),
    }
}
pub fn save_registration(dir: &Path, registration: &Registration) -> Result<(), String> {
    let mut all = read_registrations(dir)?;
    if let Some(old) = all
        .iter_mut()
        .find(|r| r.client_id == registration.client_id)
    {
        *old = registration.clone();
    } else {
        all.push(registration.clone());
    }
    let bytes = serde_json::to_vec_pretty(&all).map_err(|_| "Cannot encode registrations")?;
    atomic_write(dir, "registrations.json", &bytes)
}
fn atomic_write(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|_| "Cannot create rustmesh account directory")?;
    let tmp = dir.join(format!("{name}.{}.tmp", random()));
    fs::write(&tmp, bytes).map_err(|_| "Cannot write rustmesh account metadata")?;
    // fs::rename replaces the destination atomically on supported native filesystems.
    fs::rename(tmp, dir.join(name)).map_err(|_| "Cannot commit rustmesh account metadata".into())
}
fn host_id(dir: &Path) -> Result<String, String> {
    let registrations = read_registrations(dir)?;
    let path = dir.join("host-id");
    match fs::read_to_string(&path) {
        Ok(value) if valid_host_id(value.trim()) => return Ok(value.trim().into()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("Cannot read rustmesh host identity".into()),
    }
    if !registrations.is_empty() {
        return Err("Saved accounts have a missing or unsupported host ID. Explicit host-identity migration is required; rustmesh will not silently replace an existing account's host.".into());
    }
    let value = format!("urn:uuid:{}", uuid::Uuid::new_v4());
    atomic_write(dir, "host-id", value.as_bytes())?;
    Ok(value)
}
fn valid_host_id(value: &str) -> bool {
    if let Some(id) = value.strip_prefix("urn:uuid:") {
        if let Ok(uuid) = uuid::Uuid::parse_str(id) {
            return uuid.get_version_num() == 4
                && uuid.get_variant() == uuid::Variant::RFC4122
                && uuid.hyphenated().to_string().eq_ignore_ascii_case(id);
        }
    }
    // rustmesh currently generates UUIDv4 URIs only. Do not silently reinterpret another scheme.
    false
}

fn callback_request(reader: &mut impl Read, authority: &str) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut bytes = Vec::new();
    loop {
        if Instant::now() > deadline || bytes.len() >= 16_384 {
            return Err("Incomplete or oversized OAuth callback".into());
        }
        let mut part = [0u8; 1024];
        let n = reader
            .read(&mut part)
            .map_err(|_| "Cannot read OAuth callback")?;
        if n == 0 {
            return Err("Incomplete OAuth callback".into());
        }
        bytes.extend_from_slice(&part[..n]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request = std::str::from_utf8(&bytes).map_err(|_| "Malformed OAuth callback")?;
    let mut lines = request.lines();
    let first = lines.next().ok_or("Missing OAuth callback request")?;
    let target = first
        .strip_prefix("GET ")
        .and_then(|v| v.strip_suffix(" HTTP/1.1"))
        .ok_or("Unsupported OAuth callback request")?;
    let hosts = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect::<Vec<_>>();
    if hosts.as_slice() != [authority] {
        return Err("OAuth callback Host mismatch".into());
    }
    Ok(target.into())
}

struct Attempt {
    state: String,
    nonce: String,
    verifier: String,
    redirect: String,
    registration: Option<Registration>,
}
impl Attempt {
    fn url(&self, host: &str, id_hint: Option<&str>) -> String {
        let mut url = Url::parse("https://auth.openai.com/api/accounts/authorize").unwrap();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()));
        {
            let mut q = url.query_pairs_mut();
            q.append_pair(
                "client_id",
                self.registration
                    .as_ref()
                    .map_or("dynamic_agent_client", |r| r.client_id.as_str()),
            )
            .append_pair("ext_agent_host_id", host)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &self.redirect)
            .append_pair(
                "scope",
                "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct",
            )
            .append_pair("resource", RESOURCE)
            .append_pair("state", &self.state)
            .append_pair("nonce", &self.nonce)
            .append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", &challenge);
            if self.registration.is_none() {
                q.append_pair("agent_name_hint", "rustmesh");
            }
            if let Some(r) = &self.registration {
                if !r.email.is_empty() {
                    q.append_pair("login_hint", &r.email);
                }
            }
            if let Some(hint) = id_hint {
                q.append_pair("id_token_hint", hint);
            }
        }
        url.into()
    }
    fn callback(&self, target: &str) -> Result<(String, String), String> {
        if !target.starts_with("/auth/callback?") {
            return Err("Unexpected callback path".into());
        }
        let url =
            Url::parse(&format!("http://127.0.0.1{target}")).map_err(|_| "Malformed callback")?;
        let mut fields = std::collections::HashMap::new();
        for (k, v) in url.query_pairs() {
            if fields.insert(k.into_owned(), v.into_owned()).is_some() {
                return Err("Duplicate callback parameter".into());
            }
        }
        if fields.get("state") != Some(&self.state) {
            return Err("OAuth state mismatch".into());
        }
        if fields.contains_key("error") {
            return Err(
                "ChatGPT authorization declined or unavailable; no code was exchanged".into(),
            );
        }
        let issued = match (&self.registration, fields.get("client_id")) {
            (Some(r), Some(id)) if id != &r.client_id => {
                return Err("Callback changed the selected registration".into())
            }
            (Some(r), _) => r.client_id.clone(),
            (None, Some(id)) if id.starts_with("oaiapp_") => id.clone(),
            _ => return Err("Registration callback did not issue a client ID".into()),
        };
        let code = fields
            .get("code")
            .filter(|s| !s.is_empty())
            .ok_or("Missing authorization code")?
            .clone();
        Ok((issued, code))
    }
}

/// Binds loopback before opening the system browser. No authorization URL is logged.
pub fn sign_in(
    dir: &Path,
    registration: Option<Registration>,
    previous: Option<&Credentials>,
    cancelled: &AtomicBool,
) -> Result<Credentials, String> {
    let host = host_id(dir)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| "Cannot bind OAuth callback")?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "Cannot configure OAuth callback")?;
    let port = listener
        .local_addr()
        .map_err(|_| "Cannot inspect OAuth callback")?
        .port();
    let attempt = Attempt {
        state: random(),
        nonce: random(),
        verifier: random(),
        redirect: format!("http://127.0.0.1:{port}/auth/callback"),
        registration,
    };
    let hint = previous
        .filter(|p| {
            attempt
                .registration
                .as_ref()
                .is_some_and(|r| r.client_id == p.registration.client_id)
        })
        .map(|p| p.id_token.as_str());
    webbrowser::open(&attempt.url(&host, hint))
        .map_err(|_| "Cannot open the system browser for ChatGPT sign-in")?;
    let deadline = Instant::now() + Duration::from_secs(300);
    let (issued, code) = loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err("Sign-in cancelled".into());
        }
        if Instant::now() > deadline {
            return Err("Sign-in timed out; use Continue with ChatGPT again".into());
        }
        match listener.accept() {
            Ok((mut stream, peer)) => {
                if !peer.ip().is_loopback() {
                    continue;
                }
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let target =
                    callback_request(&mut stream, &format!("127.0.0.1:{port}")).unwrap_or_default();
                if !target.starts_with("/auth/callback?") {
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    continue;
                }
                let result = attempt.callback(&target);
                let body = if result.is_ok() {
                    "Authorization received. Return to rustmesh for verification."
                } else {
                    "Authorization rejected. Return to rustmesh."
                };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes());
                break result?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50))
            }
            Err(_) => return Err("OAuth callback listener failed".into()),
        }
    };
    let http = client()?;
    let tokens = exchange(
        &http,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", &issued),
            ("code", &code),
            ("code_verifier", &attempt.verifier),
            ("redirect_uri", &attempt.redirect),
            ("resource", RESOURCE),
        ],
    )?;
    let claims = verify_identity(&http, &tokens.id_token, &issued, Some(&attempt.nonce))?;
    if attempt
        .registration
        .as_ref()
        .is_some_and(|r| r.subject != claims.sub)
    {
        return Err("Validated identity differs from selected account".into());
    }
    let registration = Registration {
        client_id: issued,
        subject: claims.sub,
        email: claims.email,
    };
    let credentials = from_tokens(tokens, registration, None)?;
    save_registration(dir, &credentials.registration)?;
    Ok(credentials)
}

fn verify_identity(
    http: &Client,
    token: &str,
    audience: &str,
    nonce: Option<&str>,
) -> Result<Claims, String> {
    let keys: JwkSet = http
        .get(format!("{ISSUER}/.well-known/jwks.json"))
        .send()
        .map_err(|_| "Cannot fetch OpenAI signing keys")?
        .error_for_status()
        .map_err(|_| "Signing key endpoint failed")?
        .json()
        .map_err(|_| "Invalid OpenAI signing keys")?;
    verify_with_keys(token, audience, nonce, &keys)
}
fn verify_with_keys(
    token: &str,
    audience: &str,
    nonce: Option<&str>,
    keys: &JwkSet,
) -> Result<Claims, String> {
    let header = decode_header(token).map_err(|_| "Invalid ID token header")?;
    if header.alg != Algorithm::RS256 {
        return Err("ID token algorithm is not RS256".into());
    }
    let key = keys
        .find(header.kid.as_deref().ok_or("ID token has no key ID")?)
        .ok_or("ID token signing key is unknown")?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[audience]);
    validation.leeway = 30;
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    let key = DecodingKey::from_jwk(key).map_err(|_| "Invalid signing key")?;
    let claims = decode::<Claims>(token, &key, &validation)
        .map_err(|_| "ID token signature, issuer, audience or expiration rejected")?
        .claims;
    if claims.sub.is_empty() || nonce.is_some_and(|n| claims.nonce.as_deref() != Some(n)) {
        return Err("ID token identity or nonce rejected".into());
    }
    Ok(claims)
}
fn exchange(http: &Client, fields: &[(&str, &str)]) -> Result<Tokens, String> {
    let response = http
        .post(TOKEN)
        .form(fields)
        .send()
        .map_err(|_| "OAuth token endpoint unavailable")?;
    if !response.status().is_success() {
        return Err(format!(
            "OAuth exchange rejected (HTTP {}); reauthorize if expired or revoked",
            response.status().as_u16()
        ));
    }
    response
        .json()
        .map_err(|_| "Malformed OAuth token response".into())
}
fn from_tokens(
    tokens: Tokens,
    registration: Registration,
    previous: Option<&Credentials>,
) -> Result<Credentials, String> {
    if !tokens.token_type.eq_ignore_ascii_case("bearer")
        || tokens.access_token.is_empty()
        || tokens.expires_in == 0
    {
        return Err("OAuth response has invalid token metadata".into());
    }
    let direct = tokens
        .scope
        .as_ref()
        .map(|scope| scope.split_whitespace().any(|s| s == DIRECT))
        .unwrap_or_else(|| previous.is_some_and(|p| p.direct));
    Ok(Credentials {
        registration,
        access_token: tokens.access_token,
        refresh_token: if tokens.refresh_token.is_empty() {
            previous
                .map(|p| p.refresh_token.clone())
                .unwrap_or_default()
        } else {
            tokens.refresh_token
        },
        id_token: if tokens.id_token.is_empty() {
            previous.map(|p| p.id_token.clone()).unwrap_or_default()
        } else {
            tokens.id_token
        },
        expires_at: now().saturating_add(tokens.expires_in),
        direct,
    })
}
pub fn refresh(credentials: &Credentials) -> Result<Credentials, String> {
    if credentials.refresh_token.is_empty() {
        return Err("Session is not renewable; sign in again".into());
    }
    let http = client()?;
    let tokens = exchange(
        &http,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", &credentials.registration.client_id),
            ("refresh_token", &credentials.refresh_token),
            ("resource", RESOURCE),
        ],
    )?;
    if !tokens.id_token.is_empty() {
        let claims = verify_identity(
            &http,
            &tokens.id_token,
            &credentials.registration.client_id,
            None,
        )?;
        if claims.sub != credentials.registration.subject {
            return Err("Refresh changed the validated identity".into());
        }
    }
    from_tokens(tokens, credentials.registration.clone(), Some(credentials))
}

#[derive(Clone, Debug)]
pub struct ModelChoice {
    pub slug: String,
    pub display_name: String,
}
pub fn models(credentials: &Credentials) -> Result<Vec<ModelChoice>, String> {
    if !credentials.direct {
        return Err("Identity verified, but ChatGPT plan use was not granted. Enable plan use in ChatGPT settings and reauthorize.".into());
    }
    let response = client()?
        .get(format!("{RESOURCE}/models"))
        .bearer_auth(&credentials.access_token)
        .send()
        .map_err(|_| "Model catalog unavailable")?;
    if !response.status().is_success() {
        return Err(format!(
            "Account model catalog rejected (HTTP {}); check authorization and workspace policy",
            response.status().as_u16()
        ));
    }
    let value: Value = response
        .json()
        .map_err(|_| "Malformed account model catalog")?;
    let list = value["models"]
        .as_array()
        .ok_or("Account catalog omitted models")?;
    Ok(list
        .iter()
        .filter(|m| m["visibility"] == "list")
        .filter_map(|m| {
            Some(ModelChoice {
                slug: m["slug"].as_str()?.into(),
                display_name: m["display_name"].as_str()?.into(),
            })
        })
        .collect())
}
pub fn revoke(credentials: &Credentials) -> Result<(), String> {
    if credentials.refresh_token.is_empty() {
        return Ok(());
    }
    let http = client()?;
    let discovery: Value = http
        .get(format!("{ISSUER}/.well-known/openid-configuration"))
        .send()
        .map_err(|_| "Revocation discovery unavailable")?
        .error_for_status()
        .map_err(|_| "Revocation discovery rejected")?
        .json()
        .map_err(|_| "Invalid revocation discovery")?;
    let endpoint = discovery["revocation_endpoint"]
        .as_str()
        .ok_or("No revocation endpoint")?;
    let parsed = Url::parse(endpoint).map_err(|_| "Invalid revocation endpoint")?;
    if parsed.scheme() != "https" || parsed.host_str() != Some("auth.openai.com") {
        return Err("Untrusted revocation endpoint".into());
    }
    let response = http
        .post(endpoint)
        .form(&[
            ("token", credentials.refresh_token.as_str()),
            ("token_type_hint", "refresh_token"),
            ("client_id", &credentials.registration.client_id),
        ])
        .send()
        .map_err(|_| {
            "Remote revocation was not confirmed; disconnect rustmesh in ChatGPT settings"
        })?;
    if response.status().as_u16() != 200 {
        return Err(
            "Remote revocation was not confirmed; disconnect rustmesh in ChatGPT settings".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_id_is_a_stable_uuid4_and_registered_hosts_are_never_replaced() {
        let dir =
            std::env::temp_dir().join(format!("rustmesh-host-id-test-{}", uuid::Uuid::new_v4()));
        let first = host_id(&dir).unwrap();
        assert!(valid_host_id(&first));
        assert_eq!(host_id(&dir).unwrap(), first);
        assert!(!valid_host_id("rustmesh-legacy"));
        assert!(!valid_host_id(
            "urn:uuid:00000000-0000-0000-0000-000000000000"
        ));
        fs::write(dir.join("host-id"), "rustmesh-legacy").unwrap();
        let migrated = host_id(&dir).unwrap();
        assert!(valid_host_id(&migrated));
        save_registration(
            &dir,
            &Registration {
                client_id: "oaiapp_test".into(),
                subject: "subject".into(),
                email: String::new(),
            },
        )
        .unwrap();
        fs::write(dir.join("host-id"), "rustmesh-legacy").unwrap();
        assert!(host_id(&dir).unwrap_err().contains("migration"));
        assert_eq!(
            fs::read_to_string(dir.join("host-id")).unwrap(),
            "rustmesh-legacy"
        );
        fs::remove_file(dir.join("host-id")).unwrap();
        assert!(host_id(&dir).unwrap_err().contains("migration"));
        fs::write(dir.join("registrations.json"), "invalid metadata").unwrap();
        assert!(host_id(&dir).unwrap_err().contains("metadata"));
        fs::write(dir.join("host-id"), &first).unwrap();
        assert!(host_id(&dir).unwrap_err().contains("metadata"));
        fs::remove_file(dir.join("host-id")).unwrap();
        fs::remove_file(dir.join("registrations.json")).unwrap();
        fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn fragmented_callback_is_read_completely_and_host_is_checked() {
        struct Fragmented<'a>(&'a [u8]);
        impl Read for Fragmented<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let n = out.len().min(3).min(self.0.len());
                out[..n].copy_from_slice(&self.0[..n]);
                self.0 = &self.0[n..];
                Ok(n)
            }
        }
        let request =
            b"GET /auth/callback?code=opaque&state=bound HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n\r\n";
        assert_eq!(
            callback_request(&mut Fragmented(request), "127.0.0.1:1234").unwrap(),
            "/auth/callback?code=opaque&state=bound"
        );
        assert!(callback_request(&mut Fragmented(request), "localhost:1234").is_err());
    }
    #[test]
    fn authorization_url_matches_public_client_protocol() {
        let a = attempt();
        let host = format!("urn:uuid:{}", uuid::Uuid::new_v4());
        let url = Url::parse(&a.url(&host, None)).unwrap();
        assert_eq!(url.origin().ascii_serialization(), ISSUER);
        assert_eq!(url.path(), "/api/accounts/authorize");
        let fields = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(fields.get("ext_agent_host_id").unwrap(), &host);
        assert_eq!(fields.get("client_id").unwrap(), "dynamic_agent_client");
        assert_eq!(fields.get("resource").unwrap(), RESOURCE);
        assert_eq!(fields.get("redirect_uri").unwrap(), &a.redirect);
        assert_eq!(fields.get("code_challenge_method").unwrap(), "S256");
        assert_eq!(
            fields.get("code_challenge").unwrap(),
            &URL_SAFE_NO_PAD.encode(Sha256::digest(a.verifier.as_bytes()))
        );
        assert!(!fields.contains_key("client_secret"));
    }
    fn attempt() -> Attempt {
        Attempt {
            state: "bound-state".into(),
            nonce: random(),
            verifier: random(),
            redirect: "http://127.0.0.1:1234/auth/callback".into(),
            registration: None,
        }
    }
    #[test]
    fn callback_rejects_state_duplicates_and_unissued_clients() {
        let a = attempt();
        for q in [
            "state=bad&code=x&client_id=oaiapp_a",
            "state=bound-state&state=bound-state&code=x&client_id=oaiapp_a",
            "state=bound-state&code=x&client_id=dynamic_agent_client",
            "state=bound-state&error=access_denied",
        ] {
            assert!(a.callback(&format!("/auth/callback?{q}")).is_err());
        }
        assert_eq!(
            a.callback("/auth/callback?state=bound-state&code=x&client_id=oaiapp_a")
                .unwrap(),
            ("oaiapp_a".into(), "x".into())
        );
    }
    #[test]
    fn returning_registration_cannot_switch_workspace() {
        let mut a = attempt();
        a.registration = Some(Registration {
            client_id: "oaiapp_saved".into(),
            subject: "subject".into(),
            email: "".into(),
        });
        assert!(a
            .callback("/auth/callback?state=bound-state&code=x&client_id=oaiapp_other")
            .is_err());
        assert_eq!(
            a.callback("/auth/callback?state=bound-state&code=x")
                .unwrap()
                .0,
            "oaiapp_saved"
        );
        assert!(!a.url("host", None).contains("agent_name_hint"));
    }
    #[test]
    fn initial_identity_does_not_imply_inference_permission() {
        let tokens = Tokens {
            access_token: "test".into(),
            refresh_token: "".into(),
            id_token: "".into(),
            token_type: "Bearer".into(),
            expires_in: 3600,
            scope: Some("openid email".into()),
        };
        let c = from_tokens(
            tokens,
            Registration {
                client_id: "a".into(),
                subject: "s".into(),
                email: "".into(),
            },
            None,
        )
        .unwrap();
        assert!(!c.direct);
        assert!(models(&c).unwrap_err().contains("not granted"));
    }
    /// A throwaway RSA key pair and its public key as a JWK set, generated for
    /// the test run so that no private key is stored in the repository.
    fn test_keys() -> (jsonwebtoken::EncodingKey, JwkSet) {
        use rsa::{pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts, RsaPrivateKey};
        let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
        let public = private.to_public_key();
        let key =
            jsonwebtoken::EncodingKey::from_rsa_der(private.to_pkcs1_der().unwrap().as_bytes());
        let keys = serde_json::json!({"keys": [{
            "kty": "RSA", "kid": "rustmesh-test", "alg": "RS256", "use": "sig",
            "n": URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
            "e": URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
        }]});
        (key, serde_json::from_value(keys).unwrap())
    }
    #[test]
    fn rsa_identity_verification_rejects_tampering_wrong_audience_nonce_and_expiry() {
        use jsonwebtoken::{encode, Header};
        use serde_json::json;
        let (key, keys) = test_keys();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("rustmesh-test".into());
        let mut claims = json!({"iss":ISSUER,"aud":"oaiapp_test","exp":now()+300,"sub":"subject","nonce":"nonce"});
        let token = encode(&header, &claims, &key).unwrap();
        assert!(verify_with_keys(&token, "oaiapp_test", Some("nonce"), &keys).is_ok());
        assert!(verify_with_keys(&token, "oaiapp_other", Some("nonce"), &keys).is_err());
        assert!(verify_with_keys(&token, "oaiapp_test", Some("wrong"), &keys).is_err());
        let mut parts = token.split('.').map(str::to_owned).collect::<Vec<_>>();
        parts[1] = URL_SAFE_NO_PAD.encode(b"{\"sub\":\"attacker\"}");
        assert!(verify_with_keys(&parts.join("."), "oaiapp_test", Some("nonce"), &keys).is_err());
        claims["exp"] = json!(now() - 120);
        let expired = encode(&header, &claims, &key).unwrap();
        assert!(verify_with_keys(&expired, "oaiapp_test", Some("nonce"), &keys).is_err());
        claims["exp"] = json!(now() + 300);
        claims["iss"] = json!("https://attacker.invalid");
        assert!(verify_with_keys(
            &encode(&header, &claims, &key).unwrap(),
            "oaiapp_test",
            Some("nonce"),
            &keys
        )
        .is_err());
    }
}
