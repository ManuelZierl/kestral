use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::atomic_json::{
    load_json_document, persist_json_document, standard_writer, AtomicFileWriter,
};

use super::{ChromeNotice, TrustedNoticeRecord, MAX_TRUSTED_NOTICE_HISTORY};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TrustedNoticeStoreError {
    Load(String),
    Validation(String),
    Persist(String),
}

impl fmt::Display for TrustedNoticeStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(message) => write!(f, "trusted notice store load failed: {message}"),
            Self::Validation(message) => {
                write!(f, "trusted notice store validation failed: {message}")
            }
            Self::Persist(message) => write!(f, "trusted notice store persist failed: {message}"),
        }
    }
}

impl std::error::Error for TrustedNoticeStoreError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustedNoticeStoreDocument {
    version: u32,
    next_sequence: u64,
    records: Vec<TrustedNoticeRecord>,
}

pub(crate) struct TrustedNoticeStore {
    path: PathBuf,
    document: TrustedNoticeStoreDocument,
    writer: Arc<dyn AtomicFileWriter>,
}

impl TrustedNoticeStore {
    pub(crate) fn new(path: PathBuf) -> Result<Self, TrustedNoticeStoreError> {
        Self::with_writer(path, standard_writer())
    }

    pub(crate) fn with_writer(
        path: PathBuf,
        writer: Arc<dyn AtomicFileWriter>,
    ) -> Result<Self, TrustedNoticeStoreError> {
        let document = if let Some(document) = load_json_document(&path, "trusted notice store")
            .map_err(TrustedNoticeStoreError::Load)?
        {
            Self::validate_document(document)?
        } else {
            Self::default_document()
        };
        Ok(Self {
            path,
            document,
            writer,
        })
    }

    pub(crate) fn record(
        &mut self,
        notice: ChromeNotice,
    ) -> Result<TrustedNoticeRecord, TrustedNoticeStoreError> {
        let mut candidate = self.document.clone();
        let record = TrustedNoticeRecord {
            sequence: candidate.next_sequence,
            recorded_at: Utc::now(),
            acknowledged_at: None,
            notice,
        };
        candidate.next_sequence = candidate.next_sequence.checked_add(1).ok_or_else(|| {
            TrustedNoticeStoreError::Validation("notice sequence overflow".into())
        })?;
        candidate.records.push(record.clone());
        if candidate.records.len() > MAX_TRUSTED_NOTICE_HISTORY {
            let excess = candidate.records.len() - MAX_TRUSTED_NOTICE_HISTORY;
            candidate.records.drain(0..excess);
        }
        self.commit(candidate)?;
        Ok(record)
    }

    pub(crate) fn recent(&self) -> Vec<TrustedNoticeRecord> {
        self.document.records.iter().rev().cloned().collect()
    }

    fn commit(
        &mut self,
        document: TrustedNoticeStoreDocument,
    ) -> Result<(), TrustedNoticeStoreError> {
        match persist_json_document(
            &self.path,
            &document,
            "trusted notice store",
            self.writer.as_ref(),
        ) {
            Ok(()) => {
                self.document = document;
                Ok(())
            }
            Err(error) if error.is_indeterminate() => {
                self.document = document;
                Err(TrustedNoticeStoreError::Persist(error.into_message()))
            }
            Err(error) => Err(TrustedNoticeStoreError::Persist(error.into_message())),
        }
    }

    fn validate_document(
        document: TrustedNoticeStoreDocument,
    ) -> Result<TrustedNoticeStoreDocument, TrustedNoticeStoreError> {
        if document.version != 1 {
            return Err(TrustedNoticeStoreError::Validation(format!(
                "unsupported trusted notice store version: {}",
                document.version
            )));
        }
        if document.records.len() > MAX_TRUSTED_NOTICE_HISTORY {
            return Err(TrustedNoticeStoreError::Validation(format!(
                "trusted notice store exceeds retention limit: {} > {}",
                document.records.len(),
                MAX_TRUSTED_NOTICE_HISTORY
            )));
        }
        let mut previous_sequence = None;
        for record in &document.records {
            if let Some(previous) = previous_sequence {
                if record.sequence <= previous {
                    return Err(TrustedNoticeStoreError::Validation(
                        "trusted notice store records must be strictly ordered by sequence".into(),
                    ));
                }
            }
            previous_sequence = Some(record.sequence);
        }
        if let Some(last_sequence) = previous_sequence {
            if document.next_sequence <= last_sequence {
                return Err(TrustedNoticeStoreError::Validation(format!(
                    "trusted notice store next sequence {} is behind last record {}",
                    document.next_sequence, last_sequence
                )));
            }
        } else if document.next_sequence == 0 {
            // fine
        }
        Ok(document)
    }

    fn default_document() -> TrustedNoticeStoreDocument {
        TrustedNoticeStoreDocument {
            version: 1,
            next_sequence: 0,
            records: vec![],
        }
    }
}

#[cfg(test)]
mod tests;
