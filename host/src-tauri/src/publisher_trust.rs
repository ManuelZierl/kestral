use std::convert::TryFrom;
use std::path::PathBuf;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use app_host_kernel::ids::AppId;

use crate::atomic_json::{
    load_json_document, persist_json_document, standard_writer, AtomicJsonError,
};

const STORE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct TrustStoreDocument {
    version: u32,
    entries: Vec<TrustRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TrustRecord {
    pub key_id: String,
    pub public_key: String,
    pub scope: TrustScope,
    pub status: TrustStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum TrustScope {
    AppId { app_id: AppId },
    NamespacePrefix { namespace_prefix: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TrustStatus {
    Trusted,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SignatureState {
    Unsigned,
    ValidUnknownKey { key_id: String },
    Trusted { key_id: String, scope: TrustScope },
    Invalid { reason: String },
    Revoked { key_id: String, scope: TrustScope },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageSignatureDocument {
    pub algorithm: String,
    pub key_id: String,
    pub public_key: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustKeyRequest {
    pub key_id: String,
    pub public_key: String,
    pub scope: TrustScope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeKeyRequest {
    pub key_id: String,
    pub scope: TrustScope,
}

#[derive(Debug, Clone)]
pub struct PublisherTrustStore {
    path: Option<PathBuf>,
    document: TrustStoreDocument,
}

impl PublisherTrustStore {
    pub fn new(path: PathBuf) -> Result<Self, String> {
        let document =
            match load_json_document::<TrustStoreDocument>(&path, "publisher trust store")? {
                None => TrustStoreDocument {
                    version: STORE_VERSION,
                    entries: Vec::new(),
                },
                Some(document) => {
                    if document.version != STORE_VERSION {
                        return Err(format!(
                            "unsupported publisher trust store version {}",
                            document.version
                        ));
                    }
                    document
                }
            };
        Ok(Self {
            path: Some(path),
            document,
        })
    }

    pub fn in_memory() -> Self {
        Self {
            path: None,
            document: TrustStoreDocument {
                version: STORE_VERSION,
                entries: Vec::new(),
            },
        }
    }

    pub fn list(&self) -> Vec<TrustRecord> {
        self.document.entries.clone()
    }

    pub fn trust_key(
        &mut self,
        key_id: &str,
        public_key: &str,
        scope: TrustScope,
    ) -> Result<(), String> {
        let public_key_bytes = decode_public_key(public_key)?;
        let expected_key_id = fingerprint_key_id(&public_key_bytes);
        if key_id != expected_key_id {
            return Err(format!(
                "key id '{key_id}' does not match the supplied public key fingerprint"
            ));
        }
        validate_scope(&scope)?;
        self.upsert(TrustRecord {
            key_id: key_id.to_string(),
            public_key: public_key.to_string(),
            scope,
            status: TrustStatus::Trusted,
        })
    }

    pub fn revoke_key(&mut self, key_id: &str, scope: &TrustScope) -> Result<(), String> {
        validate_scope(scope)?;
        let previous = self.document.clone();
        let Some(existing) = self
            .document
            .entries
            .iter_mut()
            .find(|entry| entry.key_id == key_id && &entry.scope == scope)
        else {
            return Err(format!(
                "trusted key '{key_id}' is not registered for that scope"
            ));
        };
        existing.status = TrustStatus::Revoked;
        match self.persist() {
            Ok(()) => Ok(()),
            Err(error) if error.is_indeterminate() => Err(error.into_message()),
            Err(error) => {
                self.document = previous;
                Err(error.into_message())
            }
        }
    }

    pub fn resolve_signature(&self, key_id: &str, app_id: &str) -> Option<SignatureState> {
        if let Some(entry) = self.matching_entry(key_id, app_id, TrustStatus::Revoked) {
            return Some(SignatureState::Revoked {
                key_id: entry.key_id,
                scope: entry.scope,
            });
        }
        self.matching_entry(key_id, app_id, TrustStatus::Trusted)
            .map(|entry| SignatureState::Trusted {
                key_id: entry.key_id,
                scope: entry.scope,
            })
    }

    pub fn verify_signature(
        &self,
        package_digest: &str,
        signature_document: &PackageSignatureDocument,
        app_id: &str,
    ) -> Result<SignatureState, String> {
        if signature_document.algorithm != "ed25519" {
            return Err(format!(
                "unsupported signature algorithm '{}'",
                signature_document.algorithm
            ));
        }
        let public_key_bytes = decode_public_key(&signature_document.public_key)?;
        let expected_key_id = fingerprint_key_id(&public_key_bytes);
        if signature_document.key_id != expected_key_id {
            return Err("signature key id does not match the supplied public key".into());
        }
        let public_key = VerifyingKey::from_bytes(
            public_key_bytes
                .as_slice()
                .try_into()
                .map_err(|_| "signature public key has an invalid length".to_string())?,
        )
        .map_err(|error| format!("invalid signature public key: {error}"))?;
        let signature_bytes = decode_signature(&signature_document.signature)?;
        let signature = Signature::try_from(signature_bytes.as_slice())
            .map_err(|_| "signature has an invalid length".to_string())?;
        let signing_bytes = signing_bytes(package_digest, app_id);
        public_key
            .verify(&signing_bytes, &signature)
            .map_err(|error| format!("invalid package signature: {error}"))?;

        Ok(self
            .resolve_signature(&signature_document.key_id, app_id)
            .unwrap_or(SignatureState::ValidUnknownKey {
                key_id: signature_document.key_id.clone(),
            }))
    }

    pub fn signing_bytes(package_digest: &str, app_id: &str) -> Vec<u8> {
        signing_bytes(package_digest, app_id)
    }

    fn matching_entry(
        &self,
        key_id: &str,
        app_id: &str,
        status: TrustStatus,
    ) -> Option<TrustRecord> {
        self.document
            .entries
            .iter()
            .find(|entry| {
                entry.key_id == key_id
                    && entry.status == status
                    && scope_matches(&entry.scope, app_id)
            })
            .cloned()
    }

    fn upsert(&mut self, record: TrustRecord) -> Result<(), String> {
        validate_scope(&record.scope)?;
        let previous = self.document.clone();
        if let Some(existing) = self
            .document
            .entries
            .iter_mut()
            .find(|entry| entry.key_id == record.key_id && entry.scope == record.scope)
        {
            *existing = record;
        } else {
            self.document.entries.push(record);
        }
        match self.persist() {
            Ok(()) => Ok(()),
            Err(error) if error.is_indeterminate() => Err(error.into_message()),
            Err(error) => {
                self.document = previous;
                Err(error.into_message())
            }
        }
    }

    fn persist(&self) -> Result<(), AtomicJsonError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        persist_json_document(
            path,
            &self.document,
            "publisher trust store",
            standard_writer().as_ref(),
        )
    }
}

fn decode_public_key(value: &str) -> Result<Vec<u8>, String> {
    let bytes = STANDARD
        .decode(value.as_bytes())
        .map_err(|error| format!("invalid base64 public key: {error}"))?;
    if bytes.len() != 32 {
        return Err(format!(
            "public key must decode to 32 bytes, got {}",
            bytes.len()
        ));
    }
    Ok(bytes)
}

fn decode_signature(value: &str) -> Result<Vec<u8>, String> {
    let bytes = STANDARD
        .decode(value.as_bytes())
        .map_err(|error| format!("invalid base64 signature: {error}"))?;
    if bytes.len() != 64 {
        return Err(format!(
            "signature must decode to 64 bytes, got {}",
            bytes.len()
        ));
    }
    Ok(bytes)
}

fn fingerprint_key_id(public_key: &[u8]) -> String {
    format!("ed25519:{:x}", Sha256::digest(public_key))
}

fn signing_bytes(package_digest: &str, app_id: &str) -> Vec<u8> {
    format!("kestral-signature-v1\n{app_id}\n{package_digest}\n").into_bytes()
}

fn validate_scope(scope: &TrustScope) -> Result<(), String> {
    match scope {
        TrustScope::AppId { app_id } => {
            let value = app_id.as_str();
            if !crate::package::id_is_valid(value) {
                return Err(format!("invalid app id scope '{value}'"));
            }
        }
        TrustScope::NamespacePrefix { namespace_prefix } => {
            if !crate::package::id_is_valid(namespace_prefix) {
                return Err(format!(
                    "invalid namespace prefix scope '{namespace_prefix}'"
                ));
            }
        }
    }
    Ok(())
}

fn scope_matches(scope: &TrustScope, app_id: &str) -> bool {
    match scope {
        TrustScope::AppId {
            app_id: trusted_app_id,
        } => trusted_app_id.as_str() == app_id,
        TrustScope::NamespacePrefix { namespace_prefix } => {
            app_id == namespace_prefix || app_id.starts_with(&format!("{namespace_prefix}."))
        }
    }
}

impl SignatureState {
    pub fn label(&self) -> &'static str {
        match self {
            SignatureState::Unsigned => "unsigned",
            SignatureState::ValidUnknownKey { .. } => "valid-unknown-key",
            SignatureState::Trusted { .. } => "trusted",
            SignatureState::Invalid { .. } => "invalid",
            SignatureState::Revoked { .. } => "revoked",
        }
    }

    pub fn key_id(&self) -> Option<&str> {
        match self {
            SignatureState::Unsigned | SignatureState::Invalid { .. } => None,
            SignatureState::ValidUnknownKey { key_id }
            | SignatureState::Trusted { key_id, .. }
            | SignatureState::Revoked { key_id, .. } => Some(key_id.as_str()),
        }
    }

    pub fn blocking_error(&self) -> Option<String> {
        match self {
            SignatureState::Invalid { reason } => {
                Some(format!("invalid package signature: {reason}"))
            }
            SignatureState::Revoked { key_id, .. } => Some(format!(
                "package signature key '{key_id}' is revoked for this package"
            )),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
