//! `LocalWorkspace` must satisfy the shared `Workspace` contract (ADR-0030).
//! Each contract function gets a freshly-rooted workspace over a new temp dir.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_protocol::test_support::contract_workspace as contract;
use cogito_protocol::workspace::Workspace;
use cogito_tools::LocalWorkspace;
use tempfile::TempDir;

fn fresh() -> (Arc<dyn Workspace>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let ws = LocalWorkspace::new(tmp.path());
    (Arc::new(ws), tmp)
}

#[tokio::test]
async fn write_then_read() {
    let (ws, _tmp) = fresh();
    contract::contract_write_then_read(ws).await;
}

#[tokio::test]
async fn write_creates_parent_dirs() {
    let (ws, _tmp) = fresh();
    contract::contract_write_creates_parent_dirs(ws).await;
}

#[tokio::test]
async fn read_missing_is_not_found() {
    let (ws, _tmp) = fresh();
    contract::contract_read_missing_is_not_found(ws).await;
}

#[tokio::test]
async fn path_escape_rejected() {
    let (ws, _tmp) = fresh();
    contract::contract_path_escape_rejected(ws).await;
}

#[tokio::test]
async fn list_returns_entries() {
    let (ws, _tmp) = fresh();
    contract::contract_list_returns_entries(ws).await;
}

#[tokio::test]
async fn exists() {
    let (ws, _tmp) = fresh();
    contract::contract_exists(ws).await;
}

#[tokio::test]
async fn remove() {
    let (ws, _tmp) = fresh();
    contract::contract_remove(ws).await;
}
