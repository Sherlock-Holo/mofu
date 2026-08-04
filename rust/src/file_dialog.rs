use std::path::PathBuf;

use rfd::AsyncFileDialog;

/// 系统目录选择请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirectoryDialogRequest {
    title: String,
}

impl DirectoryDialogRequest {
    pub(crate) fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
        }
    }
}

/// 当前曲库支持导入的音频扩展名。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioFileExtension {
    Mp3,
    Flac,
    Wav,
    Ogg,
    M4a,
    Aac,
    Opus,
    Wma,
    Aiff,
    Aif,
}

impl AudioFileExtension {
    const ALL: [Self; 10] = [
        Self::Mp3,
        Self::Flac,
        Self::Wav,
        Self::Ogg,
        Self::M4a,
        Self::Aac,
        Self::Opus,
        Self::Wma,
        Self::Aiff,
        Self::Aif,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::Wav => "wav",
            Self::Ogg => "ogg",
            Self::M4a => "m4a",
            Self::Aac => "aac",
            Self::Opus => "opus",
            Self::Wma => "wma",
            Self::Aiff => "aiff",
            Self::Aif => "aif",
        }
    }
}

/// 系统音频文件多选请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MusicFilesDialogRequest {
    title: String,
    extensions: Vec<AudioFileExtension>,
}

impl MusicFilesDialogRequest {
    pub(crate) fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            extensions: AudioFileExtension::ALL.to_vec(),
        }
    }
}

/// 系统选择器的取消或选择结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DialogSelection<T> {
    Cancelled,
    Selected(T),
}

impl<T> From<Option<T>> for DialogSelection<T> {
    fn from(value: Option<T>) -> Self {
        match value {
            Some(value) => Self::Selected(value),
            None => Self::Cancelled,
        }
    }
}

/// 文件对话框模块边界。
pub(crate) trait FileDialog {
    fn pick_directory(
        &self,
        request: DirectoryDialogRequest,
    ) -> impl Future<Output = DialogSelection<PathBuf>> + Send;

    fn pick_music_files(
        &self,
        request: MusicFilesDialogRequest,
    ) -> impl Future<Output = DialogSelection<Vec<PathBuf>>> + Send;
}

/// 使用 `rfd` 系统对话框的文件选择器。
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RfdFileDialog {}

impl FileDialog for RfdFileDialog {
    async fn pick_directory(&self, request: DirectoryDialogRequest) -> DialogSelection<PathBuf> {
        AsyncFileDialog::new()
            .set_title(request.title)
            .pick_folder()
            .await
            .map(|handle| handle.path().to_path_buf())
            .into()
    }

    async fn pick_music_files(
        &self,
        request: MusicFilesDialogRequest,
    ) -> DialogSelection<Vec<PathBuf>> {
        let extensions: Vec<_> = request
            .extensions
            .into_iter()
            .map(AudioFileExtension::as_str)
            .collect();

        AsyncFileDialog::new()
            .set_title(request.title)
            .add_filter("音频文件", &extensions)
            .pick_files()
            .await
            .map(|handles| {
                handles
                    .into_iter()
                    .map(|handle| handle.path().to_path_buf())
                    .collect()
            })
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn music_request_keeps_supported_extension_order() {
        let request = MusicFilesDialogRequest::new("添加音乐文件");
        let extensions: Vec<_> = request
            .extensions
            .into_iter()
            .map(AudioFileExtension::as_str)
            .collect();

        assert_eq!(
            extensions,
            [
                "mp3", "flac", "wav", "ogg", "m4a", "aac", "opus", "wma", "aiff", "aif"
            ]
        );
    }

    #[test]
    fn option_selection_preserves_cancel_and_value() {
        assert_eq!(
            DialogSelection::<PathBuf>::from(None),
            DialogSelection::Cancelled
        );
        assert_eq!(
            DialogSelection::from(Some(PathBuf::from("/music/song.flac"))),
            DialogSelection::Selected(PathBuf::from("/music/song.flac"))
        );
    }
}
