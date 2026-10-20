//! The skills API responses (`skills/anthropic-skills-api.ts`).

use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct SkillResponse {
    pub(super) id: String,
    pub(super) display_title: Option<String>,
    pub(super) name: Option<String>,
    pub(super) description: Option<String>,
    pub(super) latest_version: Option<String>,
    pub(super) source: String,
    pub(super) created_at: String,
    pub(super) updated_at: String,
}

#[derive(Deserialize)]
pub(super) struct VersionResponse {
    #[serde(rename = "type")]
    pub(super) _kind: String,
    #[serde(rename = "skill_id")]
    pub(super) _skill_id: String,
    pub(super) name: Option<String>,
    pub(super) description: Option<String>,
}
