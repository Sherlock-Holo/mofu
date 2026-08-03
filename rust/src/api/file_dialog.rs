use std::ffi::OsString;
use std::path::PathBuf;

use thiserror::Error;

use crate::file_dialog::{
    DialogSelection, DirectoryDialogRequest, FileDialog, MusicFilesDialogRequest, RfdFileDialog,
};

const PICK_DIRECTORY_TITLE: &str = "选择音乐目录";
const PICK_FILES_TITLE: &str = "添加音乐文件";

#[derive(Debug, Error)]
enum FileDialogApiError {
    #[error("所选路径不是有效的 UTF-8：{0:?}")]
    NonUtf8Path(OsString),
}

struct FfiPath(String);

impl FfiPath {
    fn into_string(self) -> String {
        self.0
    }

    fn collect(paths: Vec<PathBuf>) -> Result<Vec<String>, FileDialogApiError> {
        paths
            .into_iter()
            .map(|path| Self::try_from(path).map(Self::into_string))
            .collect()
    }
}

impl TryFrom<PathBuf> for FfiPath {
    type Error = FileDialogApiError;

    fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
        path.into_os_string()
            .into_string()
            .map(Self)
            .map_err(FileDialogApiError::NonUtf8Path)
    }
}

/// 打开系统目录单选对话框；取消时返回 `None`。
pub async fn pick_music_directory() -> Result<Option<String>, String> {
    match RfdFileDialog::default()
        .pick_directory(DirectoryDialogRequest::new(PICK_DIRECTORY_TITLE))
        .await
    {
        DialogSelection::Cancelled => Ok(None),
        DialogSelection::Selected(path) => FfiPath::try_from(path)
            .map(FfiPath::into_string)
            .map(Some)
            .map_err(|err| err.to_string()),
    }
}

/// 打开带音频扩展名过滤的系统文件多选对话框；取消时返回 `None`。
pub async fn pick_music_files() -> Result<Option<Vec<String>>, String> {
    match RfdFileDialog::default()
        .pick_music_files(MusicFilesDialogRequest::new(PICK_FILES_TITLE))
        .await
    {
        DialogSelection::Cancelled => Ok(None),
        DialogSelection::Selected(paths) => FfiPath::collect(paths)
            .map(Some)
            .map_err(|err| err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_path_accepts_utf8_path() {
        let path = FfiPath::try_from(PathBuf::from("/music/song.flac"))
            .expect("UTF-8 path should convert");

        assert_eq!(path.into_string(), "/music/song.flac");
    }

    #[cfg(unix)]
    #[test]
    fn ffi_path_rejects_non_utf8_path() {
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(OsString::from_vec(vec![0xff]));
        let err = FfiPath::try_from(path)
            .err()
            .expect("non-UTF-8 path should fail");

        assert!(matches!(err, FileDialogApiError::NonUtf8Path(_)));
    }
}
