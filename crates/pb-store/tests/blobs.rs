#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use pb_store::FsBlobStore;

#[tokio::test]
async fn contract() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("tmp")).unwrap();
    std::fs::write(dir.path().join("tmp/left-over"), b"x").unwrap();
    let store = FsBlobStore::open(&dir.path().join("blobs"), &dir.path().join("tmp"))
        .await
        .unwrap();
    assert!(!dir.path().join("tmp/left-over").exists(), "staging is emptied at open");
    pb_store_api::contract::blob_store(&store).await;
    let reopened = FsBlobStore::open(&dir.path().join("blobs"), &dir.path().join("tmp"))
        .await
        .unwrap();
    use pb_store_api::BlobStore;
    assert_eq!(reopened.size(), store.size());
}
