mod assets;
pub mod items;
pub mod mobs;
pub mod players;
pub mod settings;
pub mod skybox;
pub mod world;

pub use fmc;

pub mod prelude {
    pub use fmc::prelude::*;
}

use fmc::bevy::app::{PluginGroup, PluginGroupBuilder};
pub struct DefaultPlugins;
impl PluginGroup for DefaultPlugins {
    fn build(self) -> fmc::bevy::app::PluginGroupBuilder {
        let group = PluginGroupBuilder::start::<Self>();
        group
            .add(settings::SettingsPlugin)
            .add(assets::ExtractBundledAssetsPlugin)
            .add_group(fmc::DefaultPlugins)
            .add(items::ItemPlugin)
            .add(players::PlayerPlugin)
            .add(world::WorldPlugin)
            .add(skybox::SkyPlugin)
            .add(mobs::MobsPlugin)
    }
}