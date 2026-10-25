//! Retail asset names.
//!
//! The original game ran on a case-insensitive filesystem and used Windows
//! separators. Inside the retail namespace every name is lowercase, uses `/`
//! and is relative to the installation root.

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid retail asset path {0:?}")]
pub struct RetailPathError(pub String);

pub fn normalize(raw: &str) -> Result<String, RetailPathError> {
    let mut parts = Vec::new();
    for part in raw.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return Err(RetailPathError(raw.to_owned())),
            part if part.contains(':') => return Err(RetailPathError(raw.to_owned())),
            part => parts.push(part.to_lowercase()),
        }
    }
    if parts.is_empty() {
        return Err(RetailPathError(raw.to_owned()));
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_case_and_separators() {
        assert_eq!(normalize("Levels\\Monastery\\./A.PVL").unwrap(), "levels/monastery/a.pvl");
        assert_eq!(normalize("/data//x").unwrap(), "data/x");
    }

    #[test]
    fn rejects_escapes_and_drives() {
        assert!(normalize("../x").is_err());
        assert!(normalize("C:\\x").is_err());
        assert!(normalize("\\").is_err());
    }
}
