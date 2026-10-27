//! Processed-asset output (`imported_assets/`) that is safe to share between
//! processes. Bevy's file writer rewrites a processed file in place, so while
//! one game or test bakes a navmesh another can load it half-written. Every
//! process here bakes identical bytes, so publishing each file whole, by
//! renaming a finished temporary file over it, is enough: a reader sees the
//! old file or the new one, never a torn one.

use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use bevy::asset::io::file::{FileAssetReader, FileAssetWriter};
use bevy::asset::io::{AssetWriter, AssetWriterError, ErasedAssetWriter, Writer};
use bevy::tasks::futures_lite::AsyncWrite;

/// A processed writer for `root` (relative to the asset base path, like
/// Bevy's own file writer).
pub fn processed_writer(
    root: &str,
) -> impl FnMut(bool) -> Option<Box<dyn ErasedAssetWriter>> + Send + Sync + use<> {
    let root = root.to_owned();
    move |create_root| {
        Some(Box::new(CacheWriter {
            root: FileAssetReader::get_base_path().join(&root),
            files: FileAssetWriter::new(&root, create_root),
        }))
    }
}

struct CacheWriter {
    root: PathBuf,
    /// Removal, renaming and directories need no care: only a write can be
    /// observed half done.
    files: FileAssetWriter,
}

impl AssetWriter for CacheWriter {
    async fn write<'a>(&'a self, path: &'a Path) -> Result<Box<Writer>, AssetWriterError> {
        Ok(Box::new(WholeFile { target: self.root.join(path), bytes: Vec::new() }))
    }

    async fn write_meta<'a>(&'a self, path: &'a Path) -> Result<Box<Writer>, AssetWriterError> {
        let mut meta = self.root.join(path).into_os_string();
        meta.push(".meta");
        Ok(Box::new(WholeFile { target: meta.into(), bytes: Vec::new() }))
    }

    async fn remove<'a>(&'a self, path: &'a Path) -> Result<(), AssetWriterError> {
        AssetWriter::remove(&self.files, path).await
    }

    async fn remove_meta<'a>(&'a self, path: &'a Path) -> Result<(), AssetWriterError> {
        AssetWriter::remove_meta(&self.files, path).await
    }

    async fn rename<'a>(&'a self, old: &'a Path, new: &'a Path) -> Result<(), AssetWriterError> {
        AssetWriter::rename(&self.files, old, new).await
    }

    async fn rename_meta<'a>(
        &'a self,
        old: &'a Path,
        new: &'a Path,
    ) -> Result<(), AssetWriterError> {
        AssetWriter::rename_meta(&self.files, old, new).await
    }

    async fn create_directory<'a>(&'a self, path: &'a Path) -> Result<(), AssetWriterError> {
        AssetWriter::create_directory(&self.files, path).await
    }

    async fn remove_directory<'a>(&'a self, path: &'a Path) -> Result<(), AssetWriterError> {
        AssetWriter::remove_directory(&self.files, path).await
    }

    async fn remove_empty_directory<'a>(&'a self, path: &'a Path) -> Result<(), AssetWriterError> {
        AssetWriter::remove_empty_directory(&self.files, path).await
    }

    async fn remove_assets_in_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<(), AssetWriterError> {
        AssetWriter::remove_assets_in_directory(&self.files, path).await
    }
}

/// Collects a file and publishes it whole when flushed. Bevy flushes once,
/// after the last byte of a processed asset or its meta.
struct WholeFile {
    target: PathBuf,
    bytes: Vec<u8>,
}

impl WholeFile {
    fn publish(&self) -> io::Result<()> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = self.target.parent().expect("a processed file lies in a directory");
        std::fs::create_dir_all(dir)?;
        let mut temp = self.target.clone().into_os_string();
        temp.push(format!(".{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&temp, &self.bytes)?;
        std::fs::rename(&temp, &self.target)
    }
}

impl AsyncWrite for WholeFile {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.bytes.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(self.publish())
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }
}
