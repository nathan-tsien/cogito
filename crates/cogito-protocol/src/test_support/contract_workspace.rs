//! Shared contract every `Workspace` implementation must satisfy. A backend
//! crate (e.g. `cogito-tools` for `LocalWorkspace`) calls these from its own
//! test module against a concrete, freshly-rooted workspace.
//!
//! Each function assumes an EMPTY workspace — the consumer constructs a fresh
//! instance (e.g. rooted at a new temp dir) per call.
//!
//! Marked test-only via the crate's `test-support` feature.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crate::workspace::{Workspace, WorkspaceError};

/// A byte slice written to a path is read back identically.
pub async fn contract_write_then_read(ws: Arc<dyn Workspace>) {
    ws.write("a.txt", b"hello world").await.expect("write ok");
    let got = ws.read("a.txt").await.expect("read ok");
    assert_eq!(got, b"hello world");
}

/// `write` creates missing parent directories.
pub async fn contract_write_creates_parent_dirs(ws: Arc<dyn Workspace>) {
    ws.write("sub/dir/b.txt", b"x")
        .await
        .expect("nested write ok");
    let got = ws.read("sub/dir/b.txt").await.expect("nested read ok");
    assert_eq!(got, b"x");
}

/// Reading a missing path is `NotFound`, not an opaque I/O error.
pub async fn contract_read_missing_is_not_found(ws: Arc<dyn Workspace>) {
    let err = ws
        .read("nope.txt")
        .await
        .expect_err("missing read must fail");
    assert!(
        matches!(err, WorkspaceError::NotFound(_)),
        "expected NotFound, got {err:?}"
    );
}

/// Paths that escape the root — `..` climbing above root, and absolute paths
/// — are rejected with `PathEscapesRoot` on both read and write. This is the
/// confinement invariant.
pub async fn contract_path_escape_rejected(ws: Arc<dyn Workspace>) {
    let dotdot = ws
        .read("../escape.txt")
        .await
        .expect_err("`..` must be rejected");
    assert!(
        matches!(dotdot, WorkspaceError::PathEscapesRoot(_)),
        "expected PathEscapesRoot for `..`, got {dotdot:?}"
    );

    let abs = ws
        .write("/etc/passwd", b"x")
        .await
        .expect_err("absolute path must be rejected");
    assert!(
        matches!(abs, WorkspaceError::PathEscapesRoot(_)),
        "expected PathEscapesRoot for absolute path, got {abs:?}"
    );

    // A nested `..` that still resolves outside root is rejected.
    let sneaky = ws
        .read("sub/../../escape.txt")
        .await
        .expect_err("nested escape must be rejected");
    assert!(
        matches!(sneaky, WorkspaceError::PathEscapesRoot(_)),
        "expected PathEscapesRoot for nested escape, got {sneaky:?}"
    );
}

/// `list` returns immediate children with correct `is_dir` flags.
pub async fn contract_list_returns_entries(ws: Arc<dyn Workspace>) {
    ws.write("top.txt", b"1").await.unwrap();
    ws.write("nested/inner.txt", b"2").await.unwrap();

    let mut entries = ws.list("").await.expect("list root ok");
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"top.txt"), "entries: {entries:?}");
    assert!(names.contains(&"nested"), "entries: {entries:?}");

    let top = entries.iter().find(|e| e.name == "top.txt").unwrap();
    assert!(!top.is_dir);
    let nested = entries.iter().find(|e| e.name == "nested").unwrap();
    assert!(nested.is_dir);
}

/// `exists` reflects presence.
pub async fn contract_exists(ws: Arc<dyn Workspace>) {
    assert!(!ws.exists("c.txt").await.expect("exists ok"));
    ws.write("c.txt", b"x").await.unwrap();
    assert!(ws.exists("c.txt").await.expect("exists ok"));
}

/// `remove` deletes a file; removing a missing file is `NotFound`.
pub async fn contract_remove(ws: Arc<dyn Workspace>) {
    ws.write("d.txt", b"x").await.unwrap();
    ws.remove("d.txt").await.expect("remove ok");
    assert!(!ws.exists("d.txt").await.unwrap());

    let err = ws
        .remove("d.txt")
        .await
        .expect_err("remove missing must fail");
    assert!(
        matches!(err, WorkspaceError::NotFound(_)),
        "expected NotFound, got {err:?}"
    );
}
