#![cfg(windows)]

use pallet_core::windows_proof::{rename_open_file_noreplace, Handle};
use std::fs;

#[test]
fn open_handle_rename_preserves_identity_and_never_replaces() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("A");
    let b = root.path().join("B");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let hello = a.join("hello.txt");
    fs::write(&hello, b"hello\\n").unwrap();
    let before = Handle::open(&hello, 0, 0).unwrap().identity().unwrap();

    rename_open_file_noreplace(&hello, &b, "hello.txt").unwrap();
    let moved = b.join("hello.txt");
    assert!(!hello.exists());
    assert_eq!(fs::read(&moved).unwrap(), b"hello\\n");
    assert_eq!(Handle::open(&moved, 0, 0).unwrap().identity().unwrap(), before);

    fs::write(a.join("hello.txt"), b"second").unwrap();
    let collision = rename_open_file_noreplace(&a.join("hello.txt"), &b, "hello.txt");
    assert!(collision.is_err());
    assert_eq!(fs::read(&moved).unwrap(), b"hello\\n");

    rename_open_file_noreplace(&moved, &a, "hello.txt").unwrap();
    assert_eq!(fs::read(a.join("hello.txt")).unwrap(), b"hello\\n");
}
