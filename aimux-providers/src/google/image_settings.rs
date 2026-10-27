//! `google-image-settings.ts`.

#[derive(Debug, Clone, Default)]
pub struct GoogleImageSettings {
    pub max_images_per_call: Option<u32>,
}
