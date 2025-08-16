use crate::ytdlp::VideoMetadata;
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

use super::_entities::medias::{ActiveModel, Entity};
pub type Medias = Entity;

impl ActiveModelBehavior for ActiveModel {
    // extend activemodel below (keep comment for generators)
}

impl super::_entities::medias::Model {
    /// Returns the parsed metadata of the media
    ///
    /// # Panics
    ///
    /// Panics if the metadata field is None or contains invalid JSON
    #[must_use]
    pub fn get_metadata(&self) -> Option<MediaMetadata> {
        serde_json::from_value(self.metadata.clone().unwrap()).ok()
    }

    /// Removes media files from the filesystem
    ///
    /// This removes both the main media file and the corresponding .info.json file.
    /// Files that don't exist are silently ignored (not an error).
    ///
    /// # Errors
    ///
    /// Returns an error if file removal fails due to permission issues or other filesystem errors.
    #[allow(clippy::result_large_err)]
    pub fn remove_media_files(&self) -> Result<()> {
        if let Some(path) = &self.media_path {
            let base_path = crate::ytdlp::media_directory().join(path);
            let info_path = base_path.with_extension("info.json");

            for file_path in [&info_path, &base_path] {
                if file_path.exists() {
                    std::fs::remove_file(file_path).map_err(|e| {
                        Error::string(&format!(
                            "Failed to remove file {}: {}",
                            file_path.display(),
                            e
                        ))
                    })?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct MediaMetadata {
    pub title: String,
    pub description: Option<String>,
    pub duration: u64,
    pub extractor_key: String,
    pub original_url: String,
    pub timestamp: i64,
}

impl From<VideoMetadata> for MediaMetadata {
    fn from(v: VideoMetadata) -> Self {
        Self {
            title: v.title,
            description: v.description,
            duration: v.duration,
            extractor_key: v.extractor_key,
            original_url: v.original_url,
            timestamp: v.timestamp,
        }
    }
}
