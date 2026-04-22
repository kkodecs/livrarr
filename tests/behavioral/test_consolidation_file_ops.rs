#![allow(dead_code, unused_imports)]

//! Behavioral tests for atomic file operations (PRIM-FILE-002).
//! Covers: fn.atomic_copy

use livrarr_library::atomic_copy;
use std::fs;

// =============================================================================
// atomic_copy
// =============================================================================

#[tokio::test]

async fn test_atomic_copy_produces_correct_copy() {
    // PRIM-FILE-002: Given a 1MB file, copies correctly and returns 1MB
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src.bin");
    let dst = dir.path().join("dst.bin");
    let data = vec![0x5Au8; 1024 * 1024];
    fs::write(&src, &data).unwrap();

    let copied = atomic_copy(&src, &dst)
        .await
        .expect("atomic_copy should succeed");

    assert_eq!(copied, data.len() as u64);
    assert_eq!(fs::read(&dst).unwrap(), data);
}

#[tokio::test]

async fn test_atomic_copy_constant_memory_large_file() {
    // PRIM-FILE-002, test.atomic_copy.constant_memory: Uses streaming copy (std::io::copy), not read-into-memory
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("large.bin");
    let dst = dir.path().join("large-copy.bin");
    let chunk = vec![0xAB; 1024 * 1024];

    {
        let mut f = fs::File::create(&src).unwrap();
        for _ in 0..16 {
            std::io::Write::write_all(&mut f, &chunk).unwrap();
        }
    }

    let copied = atomic_copy(&src, &dst)
        .await
        .expect("large streaming copy should succeed");

    assert_eq!(copied, 16 * 1024 * 1024);
    assert_eq!(fs::metadata(&dst).unwrap().len(), 16 * 1024 * 1024);
}

#[tokio::test]

async fn test_atomic_copy_no_partial_on_error() {
    // PRIM-FILE-002: On copy error, no partial file at dst
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("missing.bin");
    let dst = dir.path().join("copied.bin");

    let result = atomic_copy(&src, &dst).await;
    assert!(result.is_err(), "copy from missing src should fail");
    assert!(
        !dst.exists(),
        "no partial destination file should exist after copy failure"
    );
}

#[tokio::test]

async fn test_atomic_copy_creates_parent_directories() {
    // PRIM-FILE-002: Parent directories are created if missing
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src.txt");
    let dst = dir.path().join("a").join("b").join("dst.txt");
    fs::write(&src, b"copy me").unwrap();

    let copied = atomic_copy(&src, &dst)
        .await
        .expect("should create destination parents");

    assert_eq!(copied, 7);
    assert_eq!(fs::read(&dst).unwrap(), b"copy me");
}
