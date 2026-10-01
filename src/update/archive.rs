//! The executable inside a release archive. Only that one file is read, into
//! memory: nothing from the archive is ever unpacked to disk.

use std::io::Read;

use super::{Context, Result, fail};

/// Largest executable taken from an archive.
const MAX_EXECUTABLE: u64 = 256 * 1024 * 1024;

/// `<folder>/<bin>` from a `.tar.gz`.
#[cfg(unix)]
pub fn executable(bytes: &[u8], folder: &str, bin: &str) -> Result<Vec<u8>> {
    use std::path::Path;

    let expected = Path::new(folder).join(bin);
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    let mut found = None;
    let entries = archive
        .entries()
        .context(|| "read the release archive".to_owned())?;
    for entry in entries {
        let entry = entry.context(|| "read the release archive".to_owned())?;
        let path = entry
            .path()
            .context(|| "read the release archive".to_owned())?;
        if path.strip_prefix(".").unwrap_or(&path) != expected {
            continue;
        }
        if found.is_some() || !entry.header().entry_type().is_file() {
            return fail(format!(
                "the release archive's {} is not one regular file",
                expected.display()
            ));
        }
        found = Some(read(entry)?);
    }
    found.ok_or_else(|| super::Error(format!("the release archive has no {}", expected.display())))
}

/// `<folder>/<bin>.exe` from a `.zip`.
#[cfg(windows)]
pub fn executable(bytes: &[u8], folder: &str, bin: &str) -> Result<Vec<u8>> {
    let expected = format!("{folder}/{bin}.exe");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .context(|| "read the release archive".to_owned())?;
    let mut file = archive
        .by_name(&expected)
        .context(|| format!("find {expected} in the release archive"))?;
    if !file.is_file() || file.is_symlink() {
        return fail(format!(
            "the release archive's {expected} is not a regular file"
        ));
    }
    read(&mut file)
}

fn read(reader: impl Read) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_EXECUTABLE + 1)
        .read_to_end(&mut bytes)
        .context(|| "unpack the executable".to_owned())?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_EXECUTABLE {
        return fail("the release's executable is empty or too big");
    }
    Ok(bytes)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn tar_gz(entries: &[(&str, &[u8], tar::EntryType)]) -> Vec<u8> {
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        {
            let mut archive = tar::Builder::new(&mut gzip);
            for &(name, contents, kind) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(kind);
                header.set_size(contents.len() as u64);
                header.set_mode(0o755);
                if kind == tar::EntryType::Symlink {
                    header.set_size(0);
                    archive
                        .append_link(&mut header, name, "/usr/bin/true")
                        .unwrap();
                } else {
                    header.set_cksum();
                    archive.append_data(&mut header, name, contents).unwrap();
                }
            }
            archive.finish().unwrap();
        }
        gzip.finish().unwrap()
    }

    const FILE: tar::EntryType = tar::EntryType::Regular;

    #[test]
    fn reads_only_the_expected_executable() {
        let bytes = tar_gz(&[
            ("vitals-x86_64-unknown-linux-gnu/README.md", b"readme", FILE),
            ("vitals-x86_64-unknown-linux-gnu/vitals", b"binary", FILE),
            ("unrelated/vitals", b"ignored", FILE),
        ]);
        let folder = "vitals-x86_64-unknown-linux-gnu";
        assert_eq!(executable(&bytes, folder, "vitals").unwrap(), b"binary");
        assert!(executable(&bytes, "vitals-aarch64-apple-darwin", "vitals").is_err());
        assert!(executable(&bytes, folder, "grove").is_err());
    }

    #[test]
    fn takes_paths_written_with_a_leading_dot() {
        let bytes = tar_gz(&[("./grove-aarch64-apple-darwin/grove", b"binary", FILE)]);
        assert_eq!(
            executable(&bytes, "grove-aarch64-apple-darwin", "grove").unwrap(),
            b"binary"
        );
    }

    #[test]
    fn refuses_links_repeats_and_empty_files() {
        let folder = "vitals-x86_64-unknown-linux-gnu";
        let link = tar_gz(&[(
            "vitals-x86_64-unknown-linux-gnu/vitals",
            b"",
            tar::EntryType::Symlink,
        )]);
        assert!(executable(&link, folder, "vitals").is_err());
        let twice = tar_gz(&[
            ("vitals-x86_64-unknown-linux-gnu/vitals", b"one", FILE),
            ("vitals-x86_64-unknown-linux-gnu/vitals", b"two", FILE),
        ]);
        assert!(executable(&twice, folder, "vitals").is_err());
        let empty = tar_gz(&[("vitals-x86_64-unknown-linux-gnu/vitals", b"", FILE)]);
        assert!(executable(&empty, folder, "vitals").is_err());
        assert!(executable(b"not an archive", folder, "vitals").is_err());
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn reads_the_executable_from_the_zip() {
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        archive
            .start_file("vitals-x86_64-pc-windows-msvc/README.md", options)
            .unwrap();
        archive.write_all(b"readme").unwrap();
        archive
            .start_file("vitals-x86_64-pc-windows-msvc/vitals.exe", options)
            .unwrap();
        archive.write_all(b"windows binary").unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        let folder = "vitals-x86_64-pc-windows-msvc";
        assert_eq!(
            executable(&bytes, folder, "vitals").unwrap(),
            b"windows binary"
        );
        assert!(executable(&bytes, folder, "grove").is_err());
    }
}
