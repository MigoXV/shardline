//! Server-to-server duplication of verified Xet references.

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
};
use serde::Deserialize;
use shardline_protocol::{RepositoryProvider, RepositoryScope};
use shardline_server_core::AuthorizedRepository;

use super::{
    AppState,
    reconstruction_routes::{XetRepository, XetWriteRepository},
};
use crate::{ServerError, admission::weights, xet_adapter::validate_hash_path};

#[derive(Deserialize)]
pub(crate) struct CopyFile {
    pub(crate) sha256: String,
    pub(crate) xet_hash: String,
    pub(crate) size: u64,
}

fn service_scope(auth: &AuthorizedRepository) -> Result<&RepositoryScope, ServerError> {
    let claims = auth.claims().ok_or(ServerError::MissingAuthorization)?;
    let scope = auth.namespace().ok_or(ServerError::InsufficientScope)?;
    if claims.issuer() != "kohakuhub"
        || claims.subject() != "kohakuhub"
        || scope.provider() != RepositoryProvider::Generic
        || scope.owner() != "kohakuhub"
        || scope.revision().is_some()
        || !scope.name().starts_with("repo-")
    {
        return Err(ServerError::InsufficientScope);
    }
    Ok(scope)
}

pub(super) async fn copy_references(
    State(state): State<Arc<AppState>>,
    target: XetWriteRepository,
    headers: HeaderMap,
    Json(files): Json<Vec<CopyFile>>,
) -> Result<StatusCode, ServerError> {
    let target = service_scope(target.capability())?;
    let mut source_headers = HeaderMap::new();
    let mut values = headers.get_all("x-source-authorization").iter();
    let source_header = values.next().ok_or(ServerError::MissingAuthorization)?;
    if values.next().is_some() {
        return Err(ServerError::InvalidAuthorizationHeader);
    }
    source_headers.insert(AUTHORIZATION, source_header.clone());
    let source = XetRepository::read(&state, &source_headers)?;
    let source = service_scope(source.capability())?;
    if source == target || !(1..=128).contains(&files.len()) {
        return Err(ServerError::InvalidDigest);
    }
    if !target.name().starts_with("repo-copy-") {
        return Err(ServerError::InsufficientScope);
    }
    let _permit = state
        .admission
        .try_acquire(weights::XET_DEDUP)
        .ok_or(ServerError::WorkQueueSaturated)?;
    for file in &files {
        validate_hash_path(&file.sha256)?;
        validate_hash_path(&file.xet_hash)?;
    }
    state.backend.copy_hub_xet_references(source, target, &files).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn drop_references(
    State(state): State<Arc<AppState>>,
    target: XetWriteRepository,
) -> Result<StatusCode, ServerError> {
    let target = service_scope(target.capability())?;
    if !target.name().starts_with("repo-copy-") {
        return Err(ServerError::InsufficientScope);
    }
    let _permit = state
        .admission
        .try_acquire(weights::XET_DEDUP)
        .ok_or(ServerError::WorkQueueSaturated)?;
    state.backend.drop_hub_xet_references(target).await?;
    Ok(StatusCode::NO_CONTENT)
}
