use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{AppRevision, ManagedAppOperation};
use crate::app_data::AppDataRevision;

const JOURNAL_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UpdatePhase {
    Prepared,
    Deactivated,
    DataCandidateValidated,
    DataCommitted,
    Activated,
    RollingBack,
    DataRollbackCommitted,
    RolledBack,
    Committed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UpdateJournal {
    pub version: u32,
    pub transition_id: String,
    pub app_id: String,
    pub operation: ManagedAppOperation,
    pub phase: UpdatePhase,
    pub current_revision_id: Option<String>,
    pub target_revision: AppRevision,
    pub prior_revisions: Vec<AppRevision>,
    pub enabled: bool,
    pub data_transition: Option<AppDataTransitionJournal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppDataTransitionJournal {
    pub source_revision_id: Option<String>,
    pub source_format_version: Option<u32>,
    pub source_digest: Option<String>,
    pub candidate: AppDataRevision,
    pub candidate_digest: Option<String>,
    pub migration_revision_id: Option<String>,
    pub destructive: bool,
}

impl UpdateJournal {
    pub(crate) fn validate_version(&self) -> Result<(), String> {
        if self.version == JOURNAL_VERSION {
            return Ok(());
        }
        Err(format!(
            "unsupported app update journal version {}; expected {}",
            self.version, JOURNAL_VERSION
        ))
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        self.validate_version()?;
        if self.transition_id.trim().is_empty() || self.app_id.trim().is_empty() {
            return Err("app update journal has an empty transition or app id".into());
        }
        let data_phase = matches!(
            self.phase,
            UpdatePhase::DataCandidateValidated
                | UpdatePhase::DataCommitted
                | UpdatePhase::DataRollbackCommitted
        );
        let Some(data) = self.data_transition.as_ref() else {
            return if data_phase {
                Err("app update journal entered a data phase without a data transition".into())
            } else {
                Ok(())
            };
        };
        Uuid::parse_str(&data.candidate.revision_id)
            .map_err(|_| "app update journal has an invalid data candidate id".to_string())?;
        if data.candidate.format_version == 0 {
            return Err("app update journal has an invalid target data format".into());
        }
        match (
            data.source_revision_id.as_deref(),
            data.source_format_version,
            data.migration_revision_id.as_deref(),
        ) {
            (None, None, None) => {}
            (Some(source), Some(format), Some(_)) if format > 0 => {
                Uuid::parse_str(source).map_err(|_| {
                    "app update journal has an invalid source data revision id".to_string()
                })?;
            }
            _ => {
                return Err(
                    "app update journal has inconsistent source data migration fields".into(),
                )
            }
        }
        let source_digest_required = data.source_revision_id.is_some()
            && matches!(
                self.phase,
                UpdatePhase::DataCandidateValidated
                    | UpdatePhase::DataCommitted
                    | UpdatePhase::Activated
                    | UpdatePhase::RollingBack
                    | UpdatePhase::DataRollbackCommitted
                    | UpdatePhase::RolledBack
                    | UpdatePhase::Committed
            );
        if source_digest_required != data.source_digest.is_some() {
            return Err("app update journal has inconsistent source data digest state".into());
        }
        let candidate_digest_required = matches!(
            self.phase,
            UpdatePhase::DataCandidateValidated
                | UpdatePhase::DataCommitted
                | UpdatePhase::Activated
                | UpdatePhase::RollingBack
                | UpdatePhase::DataRollbackCommitted
                | UpdatePhase::RolledBack
                | UpdatePhase::Committed
        );
        if candidate_digest_required != data.candidate_digest.is_some() {
            return Err("app update journal has inconsistent candidate data digest state".into());
        }
        Ok(())
    }

    pub fn new(
        transition_id: String,
        app_id: String,
        operation: ManagedAppOperation,
        current_revision_id: Option<String>,
        target_revision: AppRevision,
        prior_revisions: Vec<AppRevision>,
        enabled: bool,
    ) -> Self {
        Self {
            version: JOURNAL_VERSION,
            transition_id,
            app_id,
            operation,
            phase: UpdatePhase::Prepared,
            current_revision_id,
            target_revision,
            prior_revisions,
            enabled,
            data_transition: None,
        }
    }

    pub(crate) fn with_data_transition(
        mut self,
        data_transition: Option<AppDataTransitionJournal>,
    ) -> Self {
        self.data_transition = data_transition;
        self
    }
}

#[cfg(test)]
mod tests;
