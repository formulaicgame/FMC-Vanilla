use std::collections::HashMap;

use fmc::{
    bevy::math::{DQuat, DVec3},
    blocks::{BlockPosition, Blocks},
    database::Database,
    items::ItemStack,
    models::{Model, Models},
    networking::{NetworkEvent, NetworkMessage, Server},
    physics::Collider,
    players::{Camera, Player},
    prelude::*,
    protocol::messages,
    world::{
        chunk::{Chunk, ChunkPosition},
        WorldMap,
    },
};
use serde::{Deserialize, Serialize};

use crate::world::WorldProperties;

mod hand;
mod hotbar;

pub use hand::HandInteractions;

pub struct PlayerPlugin;
impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<RespawnEvent>()
            .add_plugins(hand::HandPlugin)
            .add_plugins(hotbar::HotbarPlugin)
            .add_systems(
                Update,
                (
                    on_gamemode_update,
                    (add_players, apply_deferred).chain(),
                    respawn_players,
                    rotate_player_model,
                ),
            )
            // Save player after all remaining events have been handled. Avoid dupes and other
            // unexpected behaviour.
            .add_systems(PostUpdate, save_player_data);
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    Survival,
    Creative,
}

#[derive(Component, Serialize, Deserialize, Deref, DerefMut, Clone)]
pub struct Hotbar {
    #[deref]
    items: Vec<ItemStack>,
    equipped_item: usize,
}

impl Default for Hotbar {
    fn default() -> Self {
        Self {
            items: vec![ItemStack::default(); 9],
            equipped_item: 0,
        }
    }
}

impl Hotbar {
    pub fn held_item_stack(&self) -> &ItemStack {
        &self[self.equipped_item]
    }

    pub fn held_item_stack_mut(&mut self) -> &mut ItemStack {
        let index = self.equipped_item;
        &mut self[index]
    }
}

/// Default bundle used for new players.
#[derive(Bundle)]
pub struct PlayerBundle {
    transform: Transform,
    camera: Camera,
    aabb: Collider,
    hotbar: Hotbar,
    gamemode: GameMode,
}

impl Default for PlayerBundle {
    fn default() -> Self {
        Self {
            transform: Transform::default(),
            camera: Camera::default(),
            aabb: Collider::from_min_max(DVec3::new(-0.3, 0.0, -0.3), DVec3::new(0.3, 1.8, 0.3)),
            hotbar: Hotbar::default(),
            gamemode: GameMode::Survival,
        }
    }
}

impl From<PlayerSave> for PlayerBundle {
    fn from(save: PlayerSave) -> Self {
        PlayerBundle {
            transform: Transform::from_translation(save.position),
            camera: Camera::new(Transform {
                translation: save.camera_position,
                rotation: save.camera_rotation,
                ..default()
            }),
            hotbar: save.hotbar,
            gamemode: save.game_mode,
            ..default()
        }
    }
}

// TODO: Remember equipped and send to player
//
/// The format the player is saved as in the database.
#[derive(Serialize, Deserialize)]
pub struct PlayerSave {
    position: DVec3,
    camera_position: DVec3,
    camera_rotation: DQuat,
    hotbar: Hotbar,
    game_mode: GameMode,
}

impl PlayerSave {
    fn save(&self, username: &str, database: &Database) {
        let conn = database.get_write_connection();

        let mut stmt = conn
            .prepare("INSERT OR REPLACE INTO players VALUES (?,?)")
            .unwrap();
        let json = serde_json::to_string(self).unwrap();

        stmt.execute(rusqlite::params![username, json]).unwrap();
    }

    fn load(username: &str, database: &Database) -> Option<Self> {
        let conn = database.get_read_connection();

        let mut stmt = conn
            .prepare("SELECT save FROM players WHERE name = ?")
            .unwrap();
        let mut rows = if let Ok(rows) = stmt.query([username]) {
            rows
        } else {
            return None;
        };

        // TODO: I've forgot how you're supposed to do this correctly
        if let Some(row) = rows.next().unwrap() {
            let json: String = row.get_unwrap(0);
            let save: PlayerSave = serde_json::from_str(&json).unwrap();
            return Some(save);
        } else {
            return None;
        };
    }
}

fn add_players(
    mut commands: Commands,
    net: Res<Server>,
    database: Res<Database>,
    models: Res<Models>,
    mut respawn_events: EventWriter<RespawnEvent>,
    added_players: Query<(Entity, &Player), Added<Player>>,
) {
    for (player_entity, player) in added_players.iter() {
        let bundle = if let Some(save) = PlayerSave::load(&player.username, &database) {
            PlayerBundle::from(save)
        } else {
            respawn_events.send(RespawnEvent { player_entity });
            PlayerBundle::default()
        };

        net.send_one(
            player_entity,
            messages::PlayerPosition {
                position: bundle.transform.translation,
            },
        );

        net.send_one(
            player_entity,
            messages::PlayerCameraPosition {
                position: bundle.camera.translation.as_vec3(),
            },
        );

        net.send_one(
            player_entity,
            messages::PlayerCameraRotation {
                rotation: bundle.camera.rotation.as_quat(),
            },
        );

        commands
            .entity(player_entity)
            .insert(bundle)
            .with_children(|parent| {
                parent.spawn((
                    Model::Asset(models.get_by_name("player").id),
                    Transform {
                        //translation: player_bundle.camera.translation - player_bundle.camera.translation.y,
                        translation: DVec3::Z * 0.3 + DVec3::X * 0.3,
                        ..default()
                    },
                ));
            });
    }
}

fn save_player_data(
    database: Res<Database>,
    mut network_events: EventReader<NetworkEvent>,
    mut players: Query<(&Player, &Transform, &Camera, &Hotbar, &GameMode)>,
) {
    for network_event in network_events.read() {
        let NetworkEvent::Disconnected { entity } = network_event else {
            continue;
        };

        let Ok((player, transform, camera, hotbar, game_mode)) = players.get_mut(*entity) else {
            continue;
        };

        PlayerSave {
            position: transform.translation,
            camera_position: camera.translation,
            camera_rotation: camera.rotation,
            hotbar: hotbar.clone(),
            game_mode: *game_mode,
        }
        .save(&player.username, &database);
    }
}

#[derive(Event)]
pub struct RespawnEvent {
    pub player_entity: Entity,
}

// TODO: If it can't find a valid spawn point it will just oscillate in an infinite loop between the
// air chunk above and the one it can't find anything in.
// TODO: This might take a really long time to compute because of the chunk loading, and should
// probably be done ahead of time through an async task. Idk if the spawn point should change
// between each spawn. A good idea if it's really hard to validate that the player won't suffocate
// infinitely.
fn respawn_players(
    net: Res<Server>,
    world_properties: Res<WorldProperties>,
    world_map: Res<WorldMap>,
    database: Res<Database>,
    mut player_query: Query<&mut Transform, With<Player>>,
    mut respawn_events: EventReader<RespawnEvent>,
) {
    for respawn_event in respawn_events.read() {
        let blocks = Blocks::get();
        let air = blocks.get_id("air");

        let mut chunk_position = ChunkPosition::from(world_properties.spawn_point.center);
        let spawn_position = 'outer: loop {
            let chunk = futures_lite::future::block_on(Chunk::load(
                chunk_position,
                world_map.terrain_generator.clone(),
                database.clone(),
            ))
            .1;

            if chunk.is_uniform() && chunk[0] == air {
                break BlockPosition::from(chunk_position);
            }

            // Find two consecutive air blocks to spawn in
            for (i, block_column) in chunk.blocks.chunks_exact(Chunk::SIZE).enumerate() {
                let mut count = 0;
                for (j, block) in block_column.iter().enumerate() {
                    if count == 0 && *block == air {
                        count += 1;
                    } else if count == 1 && *block == air {
                        let mut spawn_position = BlockPosition::from(chunk_position)
                            + BlockPosition::from(i * Chunk::SIZE + j);
                        spawn_position.y -= 1;
                        break 'outer spawn_position;
                    } else {
                        count = 0;
                    }
                }
            }

            chunk_position.y += Chunk::SIZE as i32;
        };

        let spawn_position = spawn_position.as_dvec3() + DVec3::new(0.5, 0.0, 0.5);

        // TODO: Because of the latency before the client reports back its new position, the player will
        // be alive for a small moment at the spot they died, picking up their items again. So we
        // have to set the position server side too.
        let mut player_transform = player_query.get_mut(respawn_event.player_entity).unwrap();
        player_transform.translation = spawn_position;

        net.send_one(
            respawn_event.player_entity,
            messages::PlayerPosition {
                position: spawn_position,
            },
        );
    }
}

// TODO: This rotates the main player transform and lets propagation take care of the model.
// Propagation takes a long time to be sent to the clients because of unfortunate system ordering.
// This needs to be fixed on its own, but it will also become necessary to handle the player's
// models directly, as there will be a small collection of them.
fn rotate_player_model(
    mut player_query: Query<&mut Transform, With<Player>>,
    mut camera_rotation_events: EventReader<NetworkMessage<messages::PlayerCameraRotation>>,
) {
    for rotation_update in camera_rotation_events.read() {
        let mut transform = player_query.get_mut(rotation_update.player_entity).unwrap();

        let rotation = rotation_update.rotation.as_dquat();

        let theta = rotation.y.atan2(rotation.w);
        transform.rotation = DQuat::from_xyzw(0.0, theta.sin(), 0.0, theta.cos());
    }
}

fn on_gamemode_update(
    net: Res<Server>,
    player_query: Query<(Entity, &GameMode), Changed<GameMode>>,
    mut current_movement_function: Local<HashMap<Entity, Option<String>>>,
) {
    for (player_entity, gamemode) in player_query.iter() {
        let current_movement_function = current_movement_function.entry(player_entity).or_default();
        if let Some(name) = current_movement_function.take() {
            net.send_one(player_entity, messages::Plugin::Disable(name));
        }

        match gamemode {
            GameMode::Creative => {
                net.send_one(
                    player_entity,
                    messages::Plugin::Enable("creative".to_owned()),
                );
                *current_movement_function = Some("creative".to_owned());
            }
            GameMode::Survival => {
                net.send_one(
                    player_entity,
                    messages::Plugin::Enable("movement".to_owned()),
                );
                *current_movement_function = Some("movement".to_owned());
            }
        }
    }
}
