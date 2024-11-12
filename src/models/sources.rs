use super::_entities::sources::{ActiveModel, Entity};
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
pub type Sources = Entity;

impl ActiveModelBehavior for ActiveModel {
    // extend activemodel below (keep comment for generators)
}

impl super::_entities::sources::Model {
    /// Returns the parsed metadata of the source
    ///
    /// # Panics
    ///
    /// Panics if the metadata field is None or contains invalid JSON
    #[must_use]
    pub fn get_metadata(&self) -> Option<SourceMetadata> {
        serde_json::from_value(self.metadata.clone().unwrap()).ok()
    }

    /// Returns the configured `SponsorBlock` categories for this source
    #[must_use]
    pub fn get_sponsorblock_categories(&self) -> SponsorBlockCategories {
        SponsorBlockCategories::deserialize(&self.sponsorblock)
    }

    /// Returns the list of enabled `SponsorBlock` categories
    #[must_use]
    pub fn get_sponsorblock_list(&self) -> Vec<&str> {
        self.sponsorblock
            .split(',')
            .filter(|s| !s.is_empty())
            .collect()
    }
}

pub enum Relation {}

#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
pub struct SourceMetadata {
    pub uploader: String,
    pub items: u64,
    pub source_provider: String,
}

impl From<crate::ytdlp::VideoMetadata> for SourceMetadata {
    fn from(v: crate::ytdlp::VideoMetadata) -> Self {
        Self {
            uploader: v.uploader,
            items: v.n_entries.unwrap_or(0),
            source_provider: v.extractor_key,
        }
    }
}

// To fix the "too many bools" warning, we'll add allow attribute since this matches the SponsorBlock API
#[derive(Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct SponsorBlockCategories {
    pub sponsor: bool,
    pub intro: bool,
    pub outro: bool,
    pub selfpromo: bool,
    pub preview: bool,
    pub filler: bool,
    pub interaction: bool,
    pub music_offtopic: bool,
}

impl SponsorBlockCategories {
    /// Comma delimited string, containing list of categories
    /// that are enabled for this source
    /// example: `sponsor,selfpromo`
    #[must_use]
    pub fn serialize(&self) -> String {
        let mut categories = vec![];
        if self.sponsor {
            categories.push("sponsor");
        }
        if self.intro {
            categories.push("intro");
        }
        if self.outro {
            categories.push("outro");
        }
        if self.selfpromo {
            categories.push("selfpromo");
        }
        if self.preview {
            categories.push("preview");
        }
        if self.filler {
            categories.push("filler");
        }
        if self.interaction {
            categories.push("interaction");
        }
        if self.music_offtopic {
            categories.push("music_offtopic");
        }
        categories.join(",")
    }

    /// Creates new `SponsorBlockCategories` from a string representation
    #[must_use]
    pub fn deserialize(categories: &str) -> Self {
        let mut sponsor = false;
        let mut intro = false;
        let mut outro = false;
        let mut selfpromo = false;
        let mut preview = false;
        let mut filler = false;
        let mut interaction = false;
        let mut music_offtopic = false;

        for category in categories.split(',') {
            match category {
                "sponsor" => sponsor = true,
                "intro" => intro = true,
                "outro" => outro = true,
                "selfpromo" => selfpromo = true,
                "preview" => preview = true,
                "filler" => filler = true,
                "interaction" => interaction = true,
                "music_offtopic" => music_offtopic = true,
                _ => {}
            }
        }

        Self {
            sponsor,
            intro,
            outro,
            selfpromo,
            preview,
            filler,
            interaction,
            music_offtopic,
        }
    }
}
