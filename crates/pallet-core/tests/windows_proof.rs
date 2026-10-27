#![cfg(windows)]

use pallet_core::windows_proof::{rename_open_file_noreplace, Handle};
use std::fs;
use std::path::PathBuf;

struct Fixture {
    _root: tempfile::TempDir,
    a: PathBuf,
    b: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("A");
        let b = root.path().join("B");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        Self { _root: root, a, b }
    }
    fn write_a(&self, name: &str, contents: &[u8]) -> PathBuf {
        let path = self.a.join(name);
        fs::write(&path, contents).unwrap();
        path
    }
}
fn identity(path: &std::path::Path) -> (u64, u64) {
    Handle::open(path, 0, 0).unwrap().identity().unwrap()
}

#[test]
fn native_rename_and_reverse_preserve_identity_and_bytes() {
    let fx = Fixture::new();
    let source = fx.write_a("hello.txt", b"hello\n");
    let before = identity(&source);
    rename_open_file_noreplace(&source, &fx.b, "hello.txt").unwrap();
    let moved = fx.b.join("hello.txt");
    assert!(!source.exists());
    assert_eq!(fs::read(&moved).unwrap(), b"hello\n");
    assert_eq!(identity(&moved), before);
    rename_open_file_noreplace(&moved, &fx.a, "hello.txt").unwrap();
    assert_eq!(fs::read(&source).unwrap(), b"hello\n");
    assert_eq!(identity(&source), before);
}

#[test]
fn native_rename_refuses_collision_without_changing_either_file() {
    let fx = Fixture::new();
    let source = fx.write_a("hello.txt", b"source\n");
    let destination = fx.b.join("hello.txt");
    fs::write(&destination, b"destination\n").unwrap();
    let source_id = identity(&source);
    let destination_id = identity(&destination);
    assert!(rename_open_file_noreplace(&source, &fx.b, "hello.txt").is_err());
    assert_eq!(fs::read(&source).unwrap(), b"source\n");
    assert_eq!(fs::read(&destination).unwrap(), b"destination\n");
    assert_eq!(identity(&source), source_id);
    assert_eq!(identity(&destination), destination_id);
}

#[test]
fn native_reverse_rename_refuses_when_original_location_is_occupied() {
    let fx = Fixture::new();
    let source = fx.write_a("hello.txt", b"original\n");
    rename_open_file_noreplace(&source, &fx.b, "hello.txt").unwrap();
    let moved = fx.b.join("hello.txt");
    let moved_id = identity(&moved);
    fs::write(&source, b"occupant\n").unwrap();
    let occupant_id = identity(&source);
    assert!(rename_open_file_noreplace(&moved, &fx.a, "hello.txt").is_err());
    assert_eq!(fs::read(&moved).unwrap(), b"original\n");
    assert_eq!(fs::read(&source).unwrap(), b"occupant\n");
    assert_eq!(identity(&moved), moved_id);
    assert_eq!(identity(&source), occupant_id);
}
