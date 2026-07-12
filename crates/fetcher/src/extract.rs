//! Safely extract a GitHub `tar.gz` into a working directory.
//!
//! GitHub wraps the repo in a top-level `<repo>-<sha>/` directory, which we strip. The archive
//! is untrusted, so we: reject any entry whose path escapes the root, skip symlinks/special
//! files (never recreate them), and enforce file-count and byte budgets (tar-bomb guard).

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use tar::{Archive, EntryType};

use crate::error::FetchError;

/// Resource limits for extraction.
#[derive(Debug, Clone, Copy)]
pub struct Caps {
    pub max_files: usize,
    pub max_total_bytes: u64,
    pub max_file_bytes: u64,
}

impl Default for Caps {
    fn default() -> Self {
        Caps {
            max_files: 20_000,
            max_total_bytes: 100 * 1024 * 1024, // 100 MiB
            max_file_bytes: 25 * 1024 * 1024,   // 25 MiB
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExtractStats {
    pub files: usize,
    pub total_bytes: u64,
}

/// Strip the leading `<repo>-<sha>/` component and validate the remainder is a clean relative
/// path. Returns `None` for the top-level directory entry itself or any unsafe path.
pub fn sanitize_entry_path(raw: &Path) -> Option<PathBuf> {
    let mut components = raw.components();
    components.next()?; // drop the GitHub prefix component

    let mut out = PathBuf::new();
    for comp in components {
        match comp {
            Component::Normal(name) => out.push(name),
            // `..`, absolute, prefix, or a stray `.` at root all disqualify the entry.
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Extract a gzipped tar stream into `dest`, applying `caps`.
pub fn extract_tar_gz<R: Read>(
    reader: R,
    dest: &Path,
    caps: &Caps,
) -> Result<ExtractStats, FetchError> {
    std::fs::create_dir_all(dest).map_err(|e| FetchError::io(dest, e))?;

    let mut archive = Archive::new(GzDecoder::new(reader));
    let entries = archive
        .entries()
        .map_err(|e| FetchError::Archive(e.to_string()))?;

    let mut stats = ExtractStats::default();
    for entry in entries {
        let mut entry = entry.map_err(|e| FetchError::Archive(e.to_string()))?;
        let entry_type = entry.header().entry_type();
        let raw = entry
            .path()
            .map_err(|e| FetchError::Archive(e.to_string()))?
            .into_owned();

        let Some(rel) = sanitize_entry_path(&raw) else {
            continue; // top-level dir or unsafe path
        };
        let out = dest.join(&rel);

        match entry_type {
            EntryType::Directory => {
                std::fs::create_dir_all(&out).map_err(|e| FetchError::io(&out, e))?;
            }
            EntryType::Regular | EntryType::GNULongName | EntryType::Continuous => {
                let size = entry.size();
                if size > caps.max_file_bytes {
                    return Err(FetchError::ArchiveTooLarge {
                        limit: caps.max_file_bytes,
                    });
                }
                stats.total_bytes += size;
                if stats.total_bytes > caps.max_total_bytes {
                    return Err(FetchError::ArchiveTooLarge {
                        limit: caps.max_total_bytes,
                    });
                }
                stats.files += 1;
                if stats.files > caps.max_files {
                    return Err(FetchError::TooManyFiles {
                        limit: caps.max_files,
                    });
                }

                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| FetchError::io(parent, e))?;
                }
                let mut file = std::fs::File::create(&out).map_err(|e| FetchError::io(&out, e))?;
                std::io::copy(&mut entry, &mut file).map_err(|e| FetchError::io(&out, e))?;
            }
            // Symlinks, hardlinks, device/fifo nodes: never recreated from an untrusted archive.
            _ => continue,
        }
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::path::Path;
    use tempfile::TempDir;

    /// Build a gzipped tar from regular-file entries and an optional symlink entry.
    fn make_targz(files: &[(&str, &[u8])], symlink: Option<(&str, &str)>) -> Vec<u8> {
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        for (path, data) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(EntryType::Regular);
            header.set_cksum();
            builder.append_data(&mut header, path, *data).unwrap();
        }
        if let Some((link, target)) = symlink {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(EntryType::Symlink);
            header.set_size(0);
            header.set_mode(0o777);
            builder.append_link(&mut header, link, target).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn sanitize_strips_prefix_and_rejects_escapes() {
        assert_eq!(
            sanitize_entry_path(Path::new("repo-abc123/src/solution.py")),
            Some(PathBuf::from("src/solution.py"))
        );
        // top-level dir entry itself
        assert_eq!(sanitize_entry_path(Path::new("repo-abc123/")), None);
        assert_eq!(sanitize_entry_path(Path::new("repo-abc123")), None);
        // traversal / absolute escapes
        assert_eq!(sanitize_entry_path(Path::new("repo-abc/../../etc/passwd")), None);
        assert_eq!(sanitize_entry_path(Path::new("repo-abc//etc")), Some(PathBuf::from("etc")));
    }

    #[test]
    fn extracts_files_with_prefix_stripped() {
        let dir = TempDir::new().unwrap();
        let tgz = make_targz(
            &[
                ("myrepo-deadbee/src/solution.py", b"print('hi')"),
                ("myrepo-deadbee/tests/test_x.py", b"assert True"),
            ],
            None,
        );
        let stats = extract_tar_gz(&tgz[..], dir.path(), &Caps::default()).unwrap();

        assert_eq!(stats.files, 2);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/solution.py")).unwrap(),
            "print('hi')"
        );
        assert!(dir.path().join("tests/test_x.py").exists());
        // the prefix directory must not survive
        assert!(!dir.path().join("myrepo-deadbee").exists());
    }

    #[test]
    fn skips_symlink_entries() {
        let dir = TempDir::new().unwrap();
        let tgz = make_targz(
            &[("r-sha/ok.txt", b"data")],
            Some(("r-sha/evil", "/etc/passwd")),
        );
        extract_tar_gz(&tgz[..], dir.path(), &Caps::default()).unwrap();

        assert!(dir.path().join("ok.txt").exists());
        assert!(!dir.path().join("evil").exists());
    }

    #[test]
    fn enforces_total_byte_cap() {
        let dir = TempDir::new().unwrap();
        let tgz = make_targz(&[("r-sha/a", &[0u8; 100]), ("r-sha/b", &[0u8; 100])], None);
        let caps = Caps {
            max_total_bytes: 150,
            ..Caps::default()
        };
        let err = extract_tar_gz(&tgz[..], dir.path(), &caps).unwrap_err();
        assert!(matches!(err, FetchError::ArchiveTooLarge { .. }));
    }

    #[test]
    fn enforces_file_count_cap() {
        let dir = TempDir::new().unwrap();
        let tgz = make_targz(&[("r-sha/a", b"x"), ("r-sha/b", b"y")], None);
        let caps = Caps {
            max_files: 1,
            ..Caps::default()
        };
        let err = extract_tar_gz(&tgz[..], dir.path(), &caps).unwrap_err();
        assert!(matches!(err, FetchError::TooManyFiles { .. }));
    }
}
