//! Authorize an immutable Hub file before selecting its native storage scope.

// Modified from upstream Shardline to read verified native Hub file ranges.

use axum::body::Bytes;
use shardline_protocol::{ByteRange, RepositoryScope};

use crate::{ServerBackend, ServerError};

impl ServerBackend {
    pub(crate) async fn hub_repository_native_range(
        &self,
        identity: &shardline_index::XetFileIdentity,
        range: ByteRange,
    ) -> Result<Option<Bytes>, ServerError> {
        let Self::Postgres(backend) = self else {
            return Ok(None);
        };
        crate::validation::validate_content_hash(&identity.file_id)?;
        let (sha, record) = backend
            .record_store()
            .repository_xet_file(&identity.repository_scope, &identity.file_id)
            .await?
            .ok_or(ServerError::NotFound)?;
        if sha != identity.sha256 || record.total_bytes != identity.total_bytes {
            return Err(ServerError::NotFound);
        }
        crate::download_stream::authenticated_range::native_file_range(
            backend.object_store(),
            &record,
            range,
            backend.stream_work_pool.clone(),
        )
        .await
    }

    /// Select storage for an already authorized repository-wide Hub download.
    /// A missing revision grants the repository, not a literal revision-less
    /// storage namespace. Ref-limited credentials must keep their own checks.
    pub(crate) async fn hub_repository_native_read_scope(
        &self,
        identity: &shardline_index::XetFileIdentity,
    ) -> Result<RepositoryScope, ServerError> {
        if identity.repository_scope.revision().is_some() {
            return Err(ServerError::NotFound);
        }
        crate::validation::validate_content_hash(&identity.file_id)?;
        let candidate = match self {
            Self::Local(backend) => {
                let records = backend.record_store().clone();
                let identity = identity.clone();
                crate::admission::blocking_work(&backend.stream_work_pool, move || {
                    records
                        .repository_xet_file(&identity.repository_scope, &identity.file_id)
                        .map_err(ServerError::from)
                })
                .await?
            }
            Self::Postgres(backend) => {
                backend
                    .record_store()
                    .repository_xet_file(&identity.repository_scope, &identity.file_id)
                    .await?
            }
        };
        let (sha, record) = candidate.ok_or(ServerError::NotFound)?;
        if sha != identity.sha256 || record.total_bytes != identity.total_bytes {
            return Err(ServerError::NotFound);
        }
        // repository_xet_file validates the record's repository identity and
        // file identity. Its scope is a location, never an authorization grant.
        record.validate_reconstruction_plan()?;
        record.repository_scope.ok_or(ServerError::NotFound)
    }

    pub(crate) async fn hub_native_read_scope(
        &self,
        scope: &RepositoryScope,
        file_id: &str,
    ) -> Result<RepositoryScope, ServerError> {
        crate::validation::validate_content_hash(file_id)?;
        let commit = scope.revision().ok_or(ServerError::NotFound)?.to_owned();
        let repo_id = format!("{}/{}", scope.owner(), scope.name());
        let record = match self {
            Self::Local(backend) => {
                let records = backend.record_store().clone();
                let root = backend.metadata_root.join("hub");
                let scope = scope.clone();
                let file_id = file_id.to_owned();
                crate::admission::blocking_work(&backend.stream_work_pool, move || {
                    let (sha, record) = records
                        .repository_xet_file(&scope, &file_id)?
                        .ok_or(ServerError::NotFound)?;
                    let hub = shardline_index::LocalIndexStore::open(root);
                    if !hub.hub_commit_contains_lfs(&repo_id, &commit, &sha, record.total_bytes)? {
                        return Err(ServerError::NotFound);
                    }
                    Ok::<_, ServerError>(record)
                })
                .await?
            }
            Self::Postgres(backend) => {
                let (sha, record) = backend
                    .record_store()
                    .repository_xet_file(scope, file_id)
                    .await?
                    .ok_or(ServerError::NotFound)?;
                if !backend
                    .index_store()
                    .hub_commit_contains_lfs(&repo_id, &commit, &sha, record.total_bytes)
                    .await?
                {
                    return Err(ServerError::NotFound);
                }
                record
            }
        };
        record.validate_reconstruction_plan()?;
        record.repository_scope.ok_or(ServerError::NotFound)
    }
}
