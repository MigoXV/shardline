//! Authorize an immutable Hub file before selecting its native storage scope.

// Modified from upstream Shardline to read verified native Hub file ranges.

use axum::body::Bytes;
use shardline_index::{DeduplicationDomain, FileRecord, FileRecordStorageLayout, XetFileIdentity};
use shardline_protocol::{ByteRange, RepositoryScope};
use shardline_storage::AsyncObjectStore;

use crate::{ServerBackend, ServerError, app::hub_xet_copy::CopyFile};

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
        self.hub_repository_native_record(identity)
            .await?
            .repository_scope
            .ok_or(ServerError::NotFound)
    }

    async fn hub_repository_native_record(
        &self,
        identity: &XetFileIdentity,
    ) -> Result<FileRecord, ServerError> {
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
        Ok(record)
    }

    pub(crate) async fn copy_hub_xet_references(
        &self,
        source: &RepositoryScope,
        target: &RepositoryScope,
        files: &[CopyFile],
    ) -> Result<(), ServerError> {
        let mut records = Vec::with_capacity(files.len());
        let mut identities = Vec::with_capacity(files.len());
        let mut references = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut total_terms = 0usize;
        let target_domain = DeduplicationDomain::from_scope(Some(target));
        for file in files {
            if !seen.insert(&file.sha256) {
                return Err(ServerError::InvalidDigest);
            }
            let identity = XetFileIdentity {
                repository_scope: source.clone(),
                sha256: file.sha256.clone(),
                file_id: file.xet_hash.clone(),
                total_bytes: file.size,
            };
            let mut record = self.hub_repository_native_record(&identity).await?;
            if record.storage_layout() != FileRecordStorageLayout::ReferencedObjectTerms {
                return Err(ServerError::NotFound);
            }
            total_terms = total_terms.saturating_add(record.chunks.len());
            if total_terms > 65_536 {
                return Err(ServerError::TooManyShardTerms);
            }
            if let Some(current) = self.xet_file_identity(target, &file.sha256).await?
                && (current.file_id != file.xet_hash || current.total_bytes != file.size)
            {
                return Err(ServerError::InvalidDigest);
            }
            for chunk in &record.chunks {
                references.push(crate::xet_adapter::xorb_object_key(&chunk.hash)?);
            }
            record.repository_scope = Some(target.clone());
            record.deduplication_domain = Some(target_domain.clone());
            records.push(record);
            identities.push(XetFileIdentity {
                repository_scope: target.clone(),
                ..identity
            });
        }
        references.sort_unstable_by(|left, right| left.as_str().cmp(right.as_str()));
        references.dedup();
        match self {
            Self::Local(backend) => {
                let store = backend.object_store();
                for key in &references {
                    if store.metadata(key).await?.is_none() {
                        return Err(ServerError::MissingReferencedXorb);
                    }
                }
                backend
                    .record_store()
                    .commit_native_shard_metadata_with_identities(&records, &[], &identities)
                    .await?;
            }
            Self::Postgres(backend) => {
                backend
                    .commit_validated_native_shard_with_descriptors(
                        &records, &[], &references, &identities, &[],
                    )
                    .await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn drop_hub_xet_references(
        &self,
        target: &RepositoryScope,
    ) -> Result<(), ServerError> {
        match self {
            Self::Local(backend) => backend.record_store().drop_hub_xet_scope(target).await?,
            Self::Postgres(backend) => backend.record_store().drop_hub_xet_scope(target).await?,
        }
        Ok(())
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
