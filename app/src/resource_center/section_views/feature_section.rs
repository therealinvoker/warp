/// Identifies a Resource Center content section. The drawer that rendered these
/// sections has been removed, but the identifier is still used by the shared
/// [`super::SectionView`] header rendering and the changelog section/modal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureSection {
    WhatsNew,
    GettingStarted,
    MaximizeWarp,
    AdvancedSetup,
}

impl FeatureSection {
    pub fn section_name_string(&self) -> &'static str {
        match self {
            FeatureSection::WhatsNew => "What's New?",
            FeatureSection::GettingStarted => "Getting Started",
            FeatureSection::MaximizeWarp => "Maximize Warp",
            FeatureSection::AdvancedSetup => "Advanced Setup",
        }
    }
}
