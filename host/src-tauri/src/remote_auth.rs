use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use webauthn_rs::prelude::{
    CreationChallengeResponse, Passkey, PasskeyAuthentication, PasskeyRegistration,
    PublicKeyCredential, RegisterPublicKeyCredential, RequestChallengeResponse, Url, Webauthn,
    WebauthnBuilder,
};

use crate::atomic_json::{
    load_json_document, persist_json_document, standard_writer, AtomicFileWriter, AtomicJsonError,
};

const AUTH_FORMAT_VERSION: u32 = 1;
const PAIRING_FORMAT_VERSION: u32 = 1;
const PAIRING_LIFETIME_MINUTES: i64 = 10;
const CEREMONY_LIFETIME_MINUTES: i64 = 5;
const SESSION_IDLE_MINUTES: i64 = 30;
const SESSION_ABSOLUTE_HOURS: i64 = 12;
const MAX_CREDENTIALS: usize = 16;
const MAX_PENDING_CEREMONIES: usize = 32;
const MAX_SESSIONS: usize = 64;
const SESSION_COOKIE: &str = "kestral_owner_session";

pub(crate) fn validate_persisted_owner_auth(path: &Path) -> Result<(), String> {
    let Some(document) =
        load_json_document::<OwnerAuthDocument>(path, "remote owner authentication")?
    else {
        return Ok(());
    };
    if document.format_version != AUTH_FORMAT_VERSION {
        return Err(format!(
            "unsupported remote owner authentication format version {}",
            document.format_version
        ));
    }
    if document.credentials.is_empty() || document.credentials.len() > MAX_CREDENTIALS {
        return Err("remote owner authentication store has an invalid credential count".into());
    }
    for (index, credential) in document.credentials.iter().enumerate() {
        if document.credentials[..index]
            .iter()
            .any(|existing| existing.cred_id() == credential.cred_id())
        {
            return Err("remote owner authentication store contains duplicate credentials".into());
        }
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct AuthFailure {
    pub(crate) status: u16,
    pub(crate) message: String,
}

impl AuthFailure {
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: 400,
            message: message.into(),
        }
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: 401,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: 409,
            message: message.into(),
        }
    }

    fn too_many(message: impl Into<String>) -> Self {
        Self {
            status: 429,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: 500,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerAuthDocument {
    format_version: u32,
    rp_id: String,
    origin: String,
    owner_user_id: Uuid,
    credentials: Vec<Passkey>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairingDocument {
    format_version: u32,
    code_sha256: [u8; 32],
    expires_at: DateTime<Utc>,
}

struct RegistrationCeremony {
    expires_at: DateTime<Utc>,
    owner_user_id: Uuid,
    state: PasskeyRegistration,
}

struct AuthenticationCeremony {
    expires_at: DateTime<Utc>,
    state: PasskeyAuthentication,
}

struct OwnerSession {
    created_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistrationStart {
    ceremony_id: Uuid,
    options: CreationChallengeResponse,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistrationFinish {
    ceremony_id: Uuid,
    credential: RegisterPublicKeyCredential,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthenticationStart {
    ceremony_id: Uuid,
    options: RequestChallengeResponse,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthenticationFinish {
    ceremony_id: Uuid,
    credential: PublicKeyCredential,
}

pub(crate) struct RemoteOwnerAuth {
    webauthn: Webauthn,
    rp_id: String,
    origin: String,
    auth_path: PathBuf,
    pairing_path: PathBuf,
    document: Option<OwnerAuthDocument>,
    registrations: HashMap<Uuid, RegistrationCeremony>,
    authentications: HashMap<Uuid, AuthenticationCeremony>,
    sessions: HashMap<[u8; 32], OwnerSession>,
    secure_cookie: bool,
    writer: std::sync::Arc<dyn AtomicFileWriter>,
}

impl RemoteOwnerAuth {
    pub(crate) fn open(
        auth_path: PathBuf,
        pairing_path: PathBuf,
        origin: &str,
        configured_rp_id: Option<&str>,
    ) -> Result<Self, String> {
        Self::open_with_writer(
            auth_path,
            pairing_path,
            origin,
            configured_rp_id,
            standard_writer(),
        )
    }

    fn open_with_writer(
        auth_path: PathBuf,
        pairing_path: PathBuf,
        origin: &str,
        configured_rp_id: Option<&str>,
        writer: std::sync::Arc<dyn AtomicFileWriter>,
    ) -> Result<Self, String> {
        let parsed_origin = parse_origin(origin)?;
        let rp_id = configured_rp_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| parsed_origin.host_str().map(str::to_string))
            .ok_or_else(|| "HOST_REMOTE_ORIGIN must contain a host".to_string())?;
        let canonical_origin = parsed_origin.origin().ascii_serialization();
        let document = load_json_document::<OwnerAuthDocument>(
            &auth_path,
            "remote owner authentication store",
        )?;
        if let Some(document) = &document {
            validate_document(document, &rp_id, &canonical_origin)?;
        }
        let webauthn = WebauthnBuilder::new(&rp_id, &parsed_origin)
            .map_err(|error| format!("invalid remote WebAuthn configuration: {error}"))?
            .rp_name("Kestral")
            .build()
            .map_err(|error| format!("build remote WebAuthn configuration failed: {error}"))?;

        Ok(Self {
            webauthn,
            rp_id,
            origin: canonical_origin,
            auth_path,
            pairing_path,
            document,
            registrations: HashMap::new(),
            authentications: HashMap::new(),
            sessions: HashMap::new(),
            secure_cookie: parsed_origin.scheme() == "https",
            writer,
        })
    }

    pub(crate) fn is_paired(&self) -> bool {
        self.document
            .as_ref()
            .is_some_and(|document| !document.credentials.is_empty())
    }

    pub(crate) fn origin(&self) -> &str {
        &self.origin
    }

    pub(crate) fn start_registration(
        &mut self,
        pairing_code: &str,
    ) -> Result<RegistrationStart, AuthFailure> {
        let now = Utc::now();
        self.prune(now);
        if self.registrations.len() >= MAX_PENDING_CEREMONIES {
            return Err(AuthFailure::too_many(
                "too many passkey registrations are pending",
            ));
        }
        if self
            .document
            .as_ref()
            .is_some_and(|document| document.credentials.len() >= MAX_CREDENTIALS)
        {
            return Err(AuthFailure::conflict(
                "the maximum number of owner passkeys is already registered",
            ));
        }
        self.consume_pairing_code(pairing_code, now)?;

        let owner_user_id = self
            .document
            .as_ref()
            .map(|document| document.owner_user_id)
            .unwrap_or_else(Uuid::new_v4);
        let excluded = self.document.as_ref().map(|document| {
            document
                .credentials
                .iter()
                .map(|credential| credential.cred_id().clone())
                .collect()
        });
        let (options, state) = self
            .webauthn
            .start_passkey_registration(owner_user_id, "kestral-owner", "Kestral owner", excluded)
            .map_err(|error| {
                eprintln!("start passkey registration failed: {error}");
                AuthFailure::internal("could not start passkey registration")
            })?;
        let ceremony_id = Uuid::new_v4();
        self.registrations.insert(
            ceremony_id,
            RegistrationCeremony {
                expires_at: now + Duration::minutes(CEREMONY_LIFETIME_MINUTES),
                owner_user_id,
                state,
            },
        );
        Ok(RegistrationStart {
            ceremony_id,
            options,
        })
    }

    pub(crate) fn finish_registration(
        &mut self,
        request: RegistrationFinish,
    ) -> Result<String, AuthFailure> {
        let now = Utc::now();
        self.prune(now);
        let ceremony = self
            .registrations
            .remove(&request.ceremony_id)
            .ok_or_else(|| {
                AuthFailure::unauthorized("registration ceremony is invalid or expired")
            })?;
        let passkey = self
            .webauthn
            .finish_passkey_registration(&request.credential, &ceremony.state)
            .map_err(|error| {
                eprintln!("finish passkey registration failed: {error}");
                AuthFailure::unauthorized("passkey registration failed")
            })?;

        let mut candidate = self.document.clone().unwrap_or_else(|| OwnerAuthDocument {
            format_version: AUTH_FORMAT_VERSION,
            rp_id: self.rp_id.clone(),
            origin: self.origin.clone(),
            owner_user_id: ceremony.owner_user_id,
            credentials: Vec::new(),
        });
        if candidate.owner_user_id != ceremony.owner_user_id {
            return Err(AuthFailure::unauthorized(
                "registration ceremony does not match the owner",
            ));
        }
        if candidate
            .credentials
            .iter()
            .any(|credential| credential.cred_id() == passkey.cred_id())
        {
            return Err(AuthFailure::conflict("passkey is already registered"));
        }
        if candidate.credentials.len() >= MAX_CREDENTIALS {
            return Err(AuthFailure::conflict(
                "the maximum number of owner passkeys is already registered",
            ));
        }
        candidate.credentials.push(passkey);
        match self.persist_document(&candidate) {
            Ok(()) => self.document = Some(candidate),
            Err(error) if error.is_indeterminate() => {
                self.document = Some(candidate);
                return Err(AuthFailure::internal(error.into_message()));
            }
            Err(error) => return Err(AuthFailure::internal(error.into_message())),
        }
        Ok(self.issue_session(now))
    }

    pub(crate) fn start_authentication(&mut self) -> Result<AuthenticationStart, AuthFailure> {
        let now = Utc::now();
        self.prune(now);
        if self.authentications.len() >= MAX_PENDING_CEREMONIES {
            return Err(AuthFailure::too_many(
                "too many passkey sign-ins are pending",
            ));
        }
        let document = self
            .document
            .as_ref()
            .filter(|document| !document.credentials.is_empty())
            .ok_or_else(|| {
                AuthFailure::conflict(
                    "no owner passkey is registered; create a pairing code on the host",
                )
            })?;
        let (options, state) = self
            .webauthn
            .start_passkey_authentication(&document.credentials)
            .map_err(|error| {
                eprintln!("start passkey authentication failed: {error}");
                AuthFailure::internal("could not start passkey sign-in")
            })?;
        let ceremony_id = Uuid::new_v4();
        self.authentications.insert(
            ceremony_id,
            AuthenticationCeremony {
                expires_at: now + Duration::minutes(CEREMONY_LIFETIME_MINUTES),
                state,
            },
        );
        Ok(AuthenticationStart {
            ceremony_id,
            options,
        })
    }

    pub(crate) fn finish_authentication(
        &mut self,
        request: AuthenticationFinish,
    ) -> Result<String, AuthFailure> {
        let now = Utc::now();
        self.prune(now);
        let ceremony = self
            .authentications
            .remove(&request.ceremony_id)
            .ok_or_else(|| AuthFailure::unauthorized("sign-in ceremony is invalid or expired"))?;
        let result = self
            .webauthn
            .finish_passkey_authentication(&request.credential, &ceremony.state)
            .map_err(|error| {
                eprintln!("finish passkey authentication failed: {error}");
                AuthFailure::unauthorized("passkey sign-in failed")
            })?;
        if !result.user_verified() {
            return Err(AuthFailure::unauthorized(
                "passkey sign-in did not verify the owner",
            ));
        }

        let mut candidate = self
            .document
            .clone()
            .ok_or_else(|| AuthFailure::unauthorized("owner passkey is not registered"))?;
        let matched = candidate
            .credentials
            .iter_mut()
            .find_map(|credential| credential.update_credential(&result));
        let Some(updated) = matched else {
            return Err(AuthFailure::unauthorized("owner passkey is not registered"));
        };
        if updated {
            match self.persist_document(&candidate) {
                Ok(()) => self.document = Some(candidate),
                Err(error) if error.is_indeterminate() => {
                    self.document = Some(candidate);
                    return Err(AuthFailure::internal(error.into_message()));
                }
                Err(error) => return Err(AuthFailure::internal(error.into_message())),
            }
        }
        Ok(self.issue_session(now))
    }

    pub(crate) fn authenticate_cookie(&mut self, cookie_header: Option<&str>) -> bool {
        self.authenticate_cookie_until(cookie_header).is_some()
    }

    pub(crate) fn authenticate_cookie_until(
        &mut self,
        cookie_header: Option<&str>,
    ) -> Option<DateTime<Utc>> {
        let token = session_token(cookie_header)?;
        self.authenticate_session_until(token, Utc::now())
    }

    pub(crate) fn logout_cookie(&mut self, cookie_header: Option<&str>) {
        if let Some(token) = session_token(cookie_header) {
            self.sessions.remove(&token_hash(token));
        }
    }

    pub(crate) fn session_cookie(&self, token: &str) -> String {
        let secure = if self.secure_cookie { "; Secure" } else { "" };
        format!(
            "{SESSION_COOKIE}={token}; Path=/api; HttpOnly; SameSite=Strict; Max-Age={}{}",
            Duration::hours(SESSION_ABSOLUTE_HOURS).num_seconds(),
            secure
        )
    }

    pub(crate) fn clear_session_cookie(&self) -> String {
        let secure = if self.secure_cookie { "; Secure" } else { "" };
        format!("{SESSION_COOKIE}=; Path=/api; HttpOnly; SameSite=Strict; Max-Age=0{secure}")
    }

    fn persist_document(&self, document: &OwnerAuthDocument) -> Result<(), AtomicJsonError> {
        persist_json_document(
            &self.auth_path,
            document,
            "remote owner authentication store",
            self.writer.as_ref(),
        )
    }

    fn consume_pairing_code(
        &self,
        supplied_code: &str,
        now: DateTime<Utc>,
    ) -> Result<(), AuthFailure> {
        if supplied_code.len() > 256 {
            return Err(AuthFailure::unauthorized(
                "pairing code is invalid or expired",
            ));
        }
        let document =
            load_json_document::<PairingDocument>(&self.pairing_path, "remote owner pairing code")
                .map_err(AuthFailure::internal)?
                .ok_or_else(|| AuthFailure::unauthorized("pairing code is invalid or expired"))?;
        if document.format_version != PAIRING_FORMAT_VERSION
            || document.expires_at <= now
            || !constant_time_equal(&document.code_sha256, &token_hash(supplied_code))
        {
            if document.expires_at <= now {
                let _ = fs::remove_file(&self.pairing_path);
            }
            return Err(AuthFailure::unauthorized(
                "pairing code is invalid or expired",
            ));
        }
        fs::remove_file(&self.pairing_path).map_err(|error| {
            AuthFailure::internal(format!("consume remote owner pairing code failed: {error}"))
        })
    }

    fn issue_session(&mut self, now: DateTime<Utc>) -> String {
        self.prune(now);
        if self.sessions.len() >= MAX_SESSIONS {
            if let Some(oldest) = self
                .sessions
                .iter()
                .min_by_key(|(_, session)| session.last_seen_at)
                .map(|(hash, _)| *hash)
            {
                self.sessions.remove(&oldest);
            }
        }
        let token = random_token();
        self.sessions.insert(
            token_hash(&token),
            OwnerSession {
                created_at: now,
                last_seen_at: now,
            },
        );
        token
    }

    #[cfg(test)]
    fn authenticate_session_at(&mut self, token: &str, now: DateTime<Utc>) -> bool {
        self.authenticate_session_until(token, now).is_some()
    }

    fn authenticate_session_until(
        &mut self,
        token: &str,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        self.prune(now);
        let session = self.sessions.get_mut(&token_hash(token))?;
        session.last_seen_at = now;
        Some(session.created_at + Duration::hours(SESSION_ABSOLUTE_HOURS))
    }

    fn prune(&mut self, now: DateTime<Utc>) {
        self.registrations
            .retain(|_, ceremony| ceremony.expires_at > now);
        self.authentications
            .retain(|_, ceremony| ceremony.expires_at > now);
        self.sessions.retain(|_, session| {
            session.last_seen_at + Duration::minutes(SESSION_IDLE_MINUTES) > now
                && session.created_at + Duration::hours(SESSION_ABSOLUTE_HOURS) > now
        });
    }
}

pub(crate) fn create_pairing_code(path: &Path) -> Result<String, String> {
    create_pairing_code_at(path, Utc::now(), standard_writer().as_ref())
}

fn create_pairing_code_at(
    path: &Path,
    now: DateTime<Utc>,
    writer: &dyn AtomicFileWriter,
) -> Result<String, String> {
    let code = random_token();
    let document = PairingDocument {
        format_version: PAIRING_FORMAT_VERSION,
        code_sha256: token_hash(&code),
        expires_at: now + Duration::minutes(PAIRING_LIFETIME_MINUTES),
    };
    persist_json_document(path, &document, "remote owner pairing code", writer)
        .map_err(AtomicJsonError::into_message)?;
    Ok(code)
}

fn parse_origin(origin: &str) -> Result<Url, String> {
    let parsed =
        Url::parse(origin).map_err(|error| format!("invalid HOST_REMOTE_ORIGIN: {error}"))?;
    if parsed.query().is_some() || parsed.fragment().is_some() || parsed.path() != "/" {
        return Err("HOST_REMOTE_ORIGIN must contain only scheme, host, and optional port".into());
    }
    match parsed.scheme() {
        "https" => Ok(parsed),
        "http" if parsed.host_str().is_some_and(is_loopback_host) => Ok(parsed),
        _ => Err("WebAuthn requires HTTPS; plain HTTP is allowed only for localhost or a loopback address".into()),
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn validate_document(
    document: &OwnerAuthDocument,
    rp_id: &str,
    origin: &str,
) -> Result<(), String> {
    if document.format_version != AUTH_FORMAT_VERSION {
        return Err(format!(
            "unsupported remote owner authentication format version {}",
            document.format_version
        ));
    }
    if document.rp_id != rp_id || document.origin != origin {
        return Err(format!(
            "remote owner passkeys are bound to origin '{}' and RP ID '{}'; configured '{}' and '{}'",
            document.origin, document.rp_id, origin, rp_id
        ));
    }
    if document.credentials.is_empty() || document.credentials.len() > MAX_CREDENTIALS {
        return Err("remote owner authentication store has an invalid credential count".into());
    }
    for (index, credential) in document.credentials.iter().enumerate() {
        if document.credentials[..index]
            .iter()
            .any(|existing| existing.cred_id() == credential.cred_id())
        {
            return Err("remote owner authentication store contains duplicate credentials".into());
        }
    }
    Ok(())
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn constant_time_equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right.iter())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn session_token(cookie_header: Option<&str>) -> Option<&str> {
    cookie_header?.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == SESSION_COOKIE && !value.is_empty()).then_some(value)
    })
}

#[cfg(test)]
mod tests;
