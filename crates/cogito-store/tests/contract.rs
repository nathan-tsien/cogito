//! Integration test asserting `JsonlStore` satisfies the
//! `ConversationStore` contract.

use std::sync::Arc;

use cogito_protocol::ConversationStore;
use cogito_store::JsonlStore;
use cogito_test_fixtures::store_contract::run_store_contract;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn jsonl_passes_store_contract() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let root = tmp.path().to_path_buf();
    run_store_contract(move || {
        let store: Arc<dyn ConversationStore> = Arc::new(JsonlStore::new(root.clone()));
        async move { store }
    })
    .await;
    Ok(())
}
