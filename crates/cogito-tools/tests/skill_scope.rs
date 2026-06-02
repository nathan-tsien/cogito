//! Read-only skill-root scope (ADR-0032): `read_file` and `list_dir` may
//! resolve a path lexically within a registered `ExecCtx.skill_roots` entry as
//! a read-only host read, in addition to the writable workspace. Absolute
//! paths outside both the workspace and the skill roots stay rejected.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use cogito_protocol::ExecCtx;
use cogito_protocol::ids::{SessionId, TurnId};
use cogito_protocol::tool::{ToolErrorKind, ToolResult};
use cogito_tools::provider::BuiltinTool;
use cogito_tools::workspace::LocalWorkspace;
use cogito_tools::{ListDir, ReadFile};
use tempfile::TempDir;

/// `ExecCtx` with a writable workspace at `ws` and a read-only skill root at
/// `skill`.
fn ctx(ws: &std::path::Path, skill: &std::path::Path) -> ExecCtx {
    let mut c = ExecCtx::open_ended(SessionId::new(), TurnId::new());
    c.workspace = Some(Arc::new(LocalWorkspace::new(ws)));
    c.skill_roots = Arc::from(vec![skill.to_path_buf()]);
    c
}

fn text_of(result: &ToolResult) -> String {
    let ToolResult::Output(blocks) = result else {
        panic!("expected Output, got {result:?}");
    };
    blocks[0].as_str().expect("text block").to_string()
}

#[tokio::test]
async fn read_file_reads_a_bundled_file_within_a_skill_root() {
    let ws = TempDir::new().unwrap();
    let skill = TempDir::new().unwrap();
    std::fs::create_dir(skill.path().join("scripts")).unwrap();
    std::fs::write(skill.path().join("scripts").join("gen.py"), "print('hi')").unwrap();
    // The model addresses the bundled file by the absolute root (ADR-0029 header).
    let abs = skill.path().join("scripts").join("gen.py");
    let res = ReadFile
        .invoke(
            serde_json::json!({ "path": abs.to_str().unwrap() }),
            ctx(ws.path(), skill.path()),
        )
        .await;
    assert_eq!(text_of(&res), "print('hi')");
}

#[tokio::test]
async fn list_dir_lists_a_directory_within_a_skill_root() {
    let ws = TempDir::new().unwrap();
    let skill = TempDir::new().unwrap();
    std::fs::create_dir(skill.path().join("scripts")).unwrap();
    std::fs::write(skill.path().join("scripts").join("b.py"), "").unwrap();
    std::fs::write(skill.path().join("scripts").join("a.py"), "").unwrap();
    std::fs::create_dir(skill.path().join("scripts").join("nested")).unwrap();
    let abs = skill.path().join("scripts");
    let res = ListDir
        .invoke(
            serde_json::json!({ "path": abs.to_str().unwrap() }),
            ctx(ws.path(), skill.path()),
        )
        .await;
    assert_eq!(text_of(&res), "a.py\nb.py\nnested/");
}

#[tokio::test]
async fn read_file_rejects_absolute_path_outside_skill_roots() {
    let ws = TempDir::new().unwrap();
    let skill = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "nope").unwrap();
    let abs = outside.path().join("secret.txt");
    let res = ReadFile
        .invoke(
            serde_json::json!({ "path": abs.to_str().unwrap() }),
            ctx(ws.path(), skill.path()),
        )
        .await;
    match res {
        ToolResult::Error { kind, .. } => assert_eq!(kind, ToolErrorKind::InvalidArgs),
        other => panic!(
            "expected InvalidArgs for a path outside workspace and skill roots, got {other:?}"
        ),
    }
}

#[tokio::test]
async fn read_file_rejects_escape_above_a_skill_root() {
    let ws = TempDir::new().unwrap();
    let skill = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "nope").unwrap();
    // An absolute path that starts inside the skill root but climbs out via `..`.
    let escape = skill
        .path()
        .join("..")
        .join(outside.path().file_name().unwrap())
        .join("secret.txt");
    let res = ReadFile
        .invoke(
            serde_json::json!({ "path": escape.to_str().unwrap() }),
            ctx(ws.path(), skill.path()),
        )
        .await;
    assert!(
        matches!(res, ToolResult::Error { .. }),
        "a `..` escape above the skill root must not be readable, got {res:?}"
    );
}
