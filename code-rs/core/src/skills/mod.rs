pub mod loader;
pub(crate) mod frontmatter;
pub(crate) mod injection;
pub mod model;
pub mod render;
pub mod system;

pub use model::SkillMetadata;
pub use render::render_skills_section;
