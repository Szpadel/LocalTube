use crate::ytdlp::VideoMetadata;
use sea_orm::entity::prelude::*;
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
