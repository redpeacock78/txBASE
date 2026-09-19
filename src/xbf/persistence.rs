use super::{XbfError, XbfLimits, XbfTable, decode_with_limits, encode_with_limits};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn read_path(path: impl AsRef<Path>) -> Result<XbfTable, XbfError> {
    read_path_with_limits(path, &XbfLimits::default())
}

pub fn read_path_with_limits(
    path: impl AsRef<Path>,
    limits: &XbfLimits,
) -> Result<XbfTable, XbfError> {
    let bytes = fs::read(path)?;
    decode_with_limits(&bytes, limits)
}

pub fn write_path(path: impl AsRef<Path>, table: &XbfTable) -> Result<(), XbfError> {
    write_path_with_limits(path, table, &XbfLimits::default())
}

pub fn write_path_with_limits(
    path: impl AsRef<Path>,
    table: &XbfTable,
    limits: &XbfLimits,
) -> Result<(), XbfError> {
    let bytes = encode_with_limits(table, limits)?;
    let path = path.as_ref();
    let temporary_path = temporary_path(path);
    let mut file = fs::File::create(&temporary_path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    replace_snapshot(&temporary_path, path)?;
    sync_parent_directory(path)?;
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    path.with_extension("txbase.xbf.tmp")
}

#[cfg(windows)]
fn replace_snapshot(temporary_path: &Path, path: &Path) -> Result<(), XbfError> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    fs::rename(temporary_path, path)?;
    Ok(())
}

#[cfg(not(windows))]
fn replace_snapshot(temporary_path: &Path, path: &Path) -> Result<(), XbfError> {
    fs::rename(temporary_path, path)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<(), XbfError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> Result<(), XbfError> {
    Ok(())
}
