//! Name rules. A name is one directory entry, never a path.

pub const META_DIR: &str = ".pallet";
pub const RECOVERED_COPIES: &str = "Recovered Copies";

#[derive(Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    Dot,
    Separator,
    Nul,
    Reserved,
    TooLong,
}

impl NameError {
    pub fn message(&self) -> &'static str {
        match self {
            NameError::Empty => "Name is empty.",
            NameError::Dot => "Names `.` and `..` are not allowed.",
            NameError::Separator => "Names can't contain `/`.",
            NameError::Nul => "Names can't contain NUL.",
            NameError::Reserved => "`.pallet` is reserved for PALLET's own records.",
            NameError::TooLong => "Name is longer than 255 bytes.",
        }
    }
}

/// Validates a single entry name. `at_root` additionally reserves `.pallet`.
pub fn validate(name: &str, at_root: bool) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name == "." || name == ".." {
        return Err(NameError::Dot);
    }
    if name.contains('/') {
        return Err(NameError::Separator);
    }
    if name.contains('\0') {
        return Err(NameError::Nul);
    }
    if name.len() > 255 {
        return Err(NameError::TooLong);
    }
    if at_root && name == META_DIR {
        return Err(NameError::Reserved);
    }
    Ok(())
}

/// Splits `report.final.txt` into (`report.final`, `.txt`). Dotfiles keep their dot.
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// `name (copy).ext`, then `name (copy 2).ext`, ...
pub fn copy_name(name: &str, n: u32) -> String {
    let (stem, ext) = split_ext(name);
    if n <= 1 {
        format!("{stem} (copy){ext}")
    } else {
        format!("{stem} (copy {n}){ext}")
    }
}

/// `name (2).ext`, `name (3).ext`, ... for collision alternates.
pub fn numbered_name(name: &str, n: u32) -> String {
    let (stem, ext) = split_ext(name);
    format!("{stem} ({n}){ext}")
}

pub fn is_zip(name: &str) -> bool {
    name.len() > 4 && name[name.len() - 4..].eq_ignore_ascii_case(".zip")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_and_reserved() {
        assert_eq!(validate("", false), Err(NameError::Empty));
        assert_eq!(validate("..", false), Err(NameError::Dot));
        assert_eq!(validate("a/b", false), Err(NameError::Separator));
        assert_eq!(validate("/etc/passwd", false), Err(NameError::Separator));
        assert_eq!(validate("a\0b", false), Err(NameError::Nul));
        assert_eq!(validate(".pallet", true), Err(NameError::Reserved));
        assert_eq!(validate(".pallet", false), Ok(()));
        assert_eq!(validate(&"x".repeat(256), false), Err(NameError::TooLong));
        assert_eq!(validate("Ünïcødé 名前.txt", false), Ok(()));
    }

    #[test]
    fn alternate_names() {
        assert_eq!(copy_name("hello.txt", 1), "hello (copy).txt");
        assert_eq!(copy_name("hello.txt", 3), "hello (copy 3).txt");
        assert_eq!(copy_name(".bashrc", 1), ".bashrc (copy)");
        assert_eq!(numbered_name("a.tar.gz", 2), "a.tar (2).gz");
        assert!(is_zip("Backup.ZIP"));
        assert!(!is_zip(".zip"));
    }
}
