//! Small JSON persistence helpers with atomic writes.

use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("could not read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not valid JSON for this application: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("could not encode data for {path}: {source}")]
    Encode {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

/// Reads and parses a JSON file; `Ok(None)` when the file does not exist.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, StoreError> {
    let display = path.display().to_string();
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(StoreError::Read {
                path: display,
                source: e,
            });
        }
    };
    serde_json::from_slice(&data)
        .map(Some)
        .map_err(|e| StoreError::Parse {
            path: display,
            source: e,
        })
}

/// Writes JSON via a temporary file + rename in the same directory, creating
/// parent directories as needed.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), StoreError> {
    let display = path.display().to_string();
    let mut data = serde_json::to_vec_pretty(value).map_err(|e| StoreError::Encode {
        path: display.clone(),
        source: e,
    })?;
    data.push(b'\n');

    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| StoreError::Write {
        path: display.clone(),
        source: e,
    })?;

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let tmp = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, &data).map_err(|e| StoreError::Write {
        path: display.clone(),
        source: e,
    })?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        StoreError::Write {
            path: display,
            source: e,
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn roundtrip_and_missing_file() {
        let dir = std::env::temp_dir().join(format!("dlm-store-test-{}", std::process::id()));
        let path = dir.join("nested").join("data.json");
        let missing: Option<Vec<i32>> = read_json(&path).unwrap();
        assert!(missing.is_none());
        write_json_atomic(&path, &vec![1, 2, 3]).unwrap();
        let back: Option<Vec<i32>> = read_json(&path).unwrap();
        assert_eq!(back, Some(vec![1, 2, 3]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupt_file_is_a_parse_error() {
        let dir = std::env::temp_dir().join(format!("dlm-store-test2-{}", std::process::id()));
        let path = dir.join("bad.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, b"{not json").unwrap();
        let result: Result<Option<Vec<i32>>, _> = read_json(&path);
        assert!(matches!(result, Err(StoreError::Parse { .. })));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
