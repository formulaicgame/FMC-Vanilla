use std::collections::HashMap;

use fmc::{
    bevy::math::DVec3,
    blocks::{BlockConfig, BlockFace, BlockId, BlockPosition, Blocks},
    items::{ItemStack, Items},
    models::{Model, ModelMap, ModelVisibility},
    networking::{NetworkMessage, Server},
    physics::{shapes::Aabb, Collider},
    players::{Camera, Player, Target, Targets},
    prelude::*,
    protocol::messages,
    utils::Rng,
    world::{chunk::ChunkPosition, BlockUpdate, ChunkSubscriptions, WorldMap},
};

use crate::{
    items::{DroppedItem, ItemRegistry, ItemUseSystems, ItemUses},
    players::Hotbar,
};

pub struct HandPlugin;
impl Plugin for HandPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MiningEvents::default()).add_systems(
            Update,
            (
                handle_left_clicks,
                handle_right_clicks.in_set(ItemUseSystems),
                break_blocks.after(handle_left_clicks),
            ),
        );
    }
}

/// Tracks which players have right clicked the entity
#[derive(Component, Default)]
pub struct HandInteractions {
    player_entities: Vec<Entity>,
}

impl HandInteractions {
    pub fn read(&mut self) -> impl Iterator<Item = Entity> + '_ {
        self.player_entities.drain(..)
    }

    pub fn push(&mut self, player_entity: Entity) {
        self.player_entities.push(player_entity);
    }
}

fn handle_left_clicks(
    mut clicks: EventReader<NetworkMessage<messages::LeftClick>>,
    player_query: Query<(&Targets, &Camera, &GlobalTransform), With<Player>>,
    mut block_breaking_events: ResMut<MiningEvents>,
) {
    for click in clicks.read() {
        let (targets, camera, transform) = player_query.get(click.player_entity).unwrap();

        let camera_position = transform.translation() + camera.translation;

        for target in targets.iter() {
            match target {
                Target::Block {
                    block_position,
                    block_id,
                    block_face,
                    distance,
                    ..
                } => {
                    let block_config = Blocks::get().get_config(block_id);

                    if block_config.hardness.is_some() {
                        let hit_position = camera_position + camera.forward() * *distance;
                        block_breaking_events.insert(
                            *block_position,
                            (click.player_entity, *block_id, *block_face, hit_position),
                        );

                        break;
                    }
                }
                _ => continue,
            }
        }
    }
}

#[derive(Resource, Deref, DerefMut, Default, Debug)]
struct MiningEvents(HashMap<BlockPosition, (Entity, BlockId, BlockFace, DVec3)>);

// Keeps the state of how far along a block is to breaking
#[derive(Debug)]
struct BreakingBlock {
    model_entity: Entity,
    progress: f32,
    prev_hit: std::time::Instant,
    particle_timer: Timer,
}

#[derive(Component)]
struct BreakingBlockMarker;

fn break_blocks(
    mut commands: Commands,
    time: Res<Time>,
    net: Res<Server>,
    items: Res<Items>,
    chunk_subscriptions: Res<ChunkSubscriptions>,
    hotbar_query: Query<&Hotbar, With<Player>>,
    mut model_query: Query<(&mut Model, &mut ModelVisibility), With<BreakingBlockMarker>>,
    mut block_update_writer: EventWriter<BlockUpdate>,
    mut mining_events: ResMut<MiningEvents>,
    mut being_broken: Local<HashMap<BlockPosition, BreakingBlock>>,
    mut rng: Local<Rng>,
) {
    let now = std::time::Instant::now();

    let blocks = Blocks::get();

    for (block_position, (player_entity, block_id, block_face, hit_position)) in
        mining_events.drain()
    {
        let block_config = blocks.get_config(&block_id);

        let Some(hardness) = block_config.hardness else {
            // Unbreakable block
            continue;
        };

        let hotbar = hotbar_query.get(player_entity).unwrap();

        let tool_config = if let Some(item) = hotbar.held_item_stack().item() {
            Some(items.get_config(&item.id))
        } else {
            None
        };

        let broken = if let Some(breaking_block) = being_broken.get_mut(&block_position) {
            if (now - breaking_block.prev_hit).as_secs_f32() > 0.05 {
                // The interval between two clicks needs to be short in order to be counted as
                // holding the button down.
                breaking_block.prev_hit = now;
                continue;
            }

            if breaking_block.particle_timer.finished() {
                let chunk_position = ChunkPosition::from(block_position);
                if let Some(subscribers) = chunk_subscriptions.get_subscribers(&chunk_position) {
                    if let Some(particle_effect) =
                        hit_particles(block_config, block_face, hit_position)
                    {
                        net.send_many(subscribers, particle_effect);
                    }

                    if let Some(hit_sound) = block_config.sound.hit(&mut rng) {
                        net.send_many(
                            subscribers,
                            messages::Sound {
                                position: Some(hit_position),
                                volume: 0.2,
                                speed: 0.5,
                                sound: hit_sound.to_owned(),
                            },
                        )
                    }
                }
            }

            // The timer is set to finished on the first hit to show particles immediately.
            // If we tick before checking if it is finished it will set itself to unfinished again.
            breaking_block.particle_timer.tick(time.delta());

            let (mut model, mut visibility) =
                model_query.get_mut(breaking_block.model_entity).unwrap();

            let prev_progress = breaking_block.progress;

            let efficiency = if let Some(config) = tool_config {
                config.tool_efficiency(block_config)
            } else {
                1.0
            };
            breaking_block.progress +=
                (now - breaking_block.prev_hit).as_secs_f32() / hardness * efficiency;
            breaking_block.prev_hit = now;

            let Model::Custom {
                ref mut material_parallax_texture,
                ..
            } = *model
            else {
                unreachable!()
            };

            let progress = breaking_block.progress;

            // Ordering from high to low lets it skip stages.
            if prev_progress < 0.9 && progress > 0.9 {
                *material_parallax_texture = Some("blocks/breaking_9.png".to_owned());
            } else if prev_progress < 0.8 && progress > 0.8 {
                *material_parallax_texture = Some("blocks/breaking_8.png".to_owned());
            } else if prev_progress < 0.7 && progress > 0.7 {
                *material_parallax_texture = Some("blocks/breaking_7.png".to_owned());
            } else if prev_progress < 0.6 && progress > 0.6 {
                *material_parallax_texture = Some("blocks/breaking_6.png".to_owned());
            } else if prev_progress < 0.5 && progress > 0.5 {
                *material_parallax_texture = Some("blocks/breaking_5.png".to_owned());
            } else if prev_progress < 0.4 && progress > 0.4 {
                *material_parallax_texture = Some("blocks/breaking_4.png".to_owned());
            } else if prev_progress < 0.3 && progress > 0.3 {
                *material_parallax_texture = Some("blocks/breaking_3.png".to_owned());
            } else if prev_progress < 0.2 && progress > 0.2 {
                *material_parallax_texture = Some("blocks/breaking_2.png".to_owned());
            } else if prev_progress < 0.1 && progress > 0.1 {
                *visibility = ModelVisibility::Visible;
            }

            if progress >= 1.0 {
                true
            } else {
                continue;
            }
        } else {
            false
        };

        // When hardness is zero it will break instantly
        if broken || hardness == 0.0 {
            let chunk_position = ChunkPosition::from(block_position);
            if let Some(subscribers) = chunk_subscriptions.get_subscribers(&chunk_position) {
                let position = block_position.as_dvec3() + DVec3::splat(0.5);
                if let Some(particle_effect) = break_particles(block_config, position) {
                    net.send_many(subscribers, particle_effect);
                }

                if let Some(destroy_sound) = block_config.sound.destroy(&mut rng) {
                    net.send_many(
                        subscribers,
                        messages::Sound {
                            position: Some(position),
                            volume: 1.0,
                            speed: 1.0,
                            sound: destroy_sound.to_owned(),
                        },
                    )
                }
            }

            // TODO: Dropping a block like this is too error prone. If two systems break a block at
            // once, it will dupe. Also too much boilerplate just to drop an item, it should just
            // be:
            // block_break_events.send(BreakEvent {
            //     position: IVec3,
            //     something to signify if it should drop
            // })
            block_update_writer.send(BlockUpdate::Replace {
                position: block_position,
                block_id: blocks.get_id("air"),
                block_state: None,
                block_data: None,
            });

            let (dropped_item_id, count) = match block_config.drop(tool_config) {
                Some(drop) => drop,
                None => continue,
            };

            let item_config = items.get_config(&dropped_item_id);
            let item_stack = ItemStack::new(item_config, count);

            commands.spawn((
                DroppedItem::new(item_stack),
                Transform::from_translation(block_position.as_dvec3() + DVec3::splat(0.5)),
            ));
        } else {
            let model_entity = commands
                .spawn((
                    build_breaking_model(),
                    // The model shouldn't show until some progress has been made
                    ModelVisibility::Hidden,
                    Transform::from_translation(block_position.as_dvec3()),
                    BreakingBlockMarker,
                ))
                .id();

            let particle_timer = Timer::new(
                std::time::Duration::from_secs_f32(0.2),
                TimerMode::Repeating,
            );
            // Tick the timer so the first particles show up immediately
            //particle_timer.tick(std::time::Duration::from_secs(1));

            being_broken.insert(
                block_position,
                BreakingBlock {
                    model_entity,
                    progress: 0.0,
                    prev_hit: std::time::Instant::now(),
                    particle_timer,
                },
            );
        }
    }

    // Remove break progress after not being hit for 0.5 seconds.
    being_broken.retain(|_, breaking_block| {
        let remove_timout = (now - breaking_block.prev_hit).as_secs_f32() > 0.5;
        let remove_broken = breaking_block.progress >= 1.0;

        if remove_timout || remove_broken {
            commands.entity(breaking_block.model_entity).despawn();
            return false;
        } else {
            return true;
        }
    });
}

fn hit_particles(
    block_config: &BlockConfig,
    block_face: BlockFace,
    position: DVec3,
) -> Option<messages::ParticleEffect> {
    let Some(particle_texture) = block_config.particle_texture(block_face) else {
        return None;
    };

    let direction = block_face
        .shift_position(BlockPosition::default())
        .as_vec3();
    let spawn_offset = Vec3::select(direction.cmpeq(Vec3::ZERO), Vec3::splat(0.4), Vec3::ZERO);

    const VELOCITY: Vec3 = Vec3::new(2.5, 1.5, 2.5);
    let mut min_velocity = Vec3::select(direction.cmpeq(Vec3::ZERO), -VELOCITY, Vec3::ZERO);
    min_velocity.y = 0.0;

    let mut max_velocity = -min_velocity;
    max_velocity += direction * 2.0;
    max_velocity.y = max_velocity.y.max(VELOCITY.y);

    // Need to offset so the particle's aabb won't be inside the block
    let block_face_offset = block_face
        .shift_position(BlockPosition::default())
        .as_dvec3()
        * 0.15;

    Some(messages::ParticleEffect::Explosion {
        position: position + block_face_offset,
        spawn_offset,
        size_range: (0.1, 0.2),
        min_velocity,
        max_velocity,
        texture: Some(particle_texture.to_owned()),
        color: block_config.particle_color(),
        lifetime: (0.3, 1.0),
        count: 4,
    })
}

fn break_particles(
    block_config: &BlockConfig,
    position: DVec3,
) -> Option<messages::ParticleEffect> {
    let Some(particle_texture) = block_config.particle_texture(BlockFace::Bottom) else {
        return None;
    };

    const VELOCITY: Vec3 = Vec3::new(7.0, 5.0, 7.0);

    Some(messages::ParticleEffect::Explosion {
        position,
        spawn_offset: Vec3::splat(0.2),
        size_range: (0.2, 0.3),
        min_velocity: -VELOCITY,
        max_velocity: VELOCITY,
        texture: Some(particle_texture.to_owned()),
        color: block_config.particle_color(),
        lifetime: (0.3, 1.0),
        count: 20,
    })
}

// TODO: This needs to be built from the model of what it is breaking. Means we have to load and
// store the quad info for each block including through gltfs
fn build_breaking_model() -> Model {
    let mesh_vertices = vec![
        // Top
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
        [1.0, 1.0, 1.0],
        // Back
        [1.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0],
        // Left
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 1.0],
        [0.0, 0.0, 1.0],
        // Right
        [1.0, 1.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
        // Front
        [0.0, 1.0, 1.0],
        [0.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [1.0, 0.0, 1.0],
        // Bottom
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
    ];

    let mesh_normals = vec![
        // Top
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        // Back
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
        // Left
        [-1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        // Right
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        // Front
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        // Bottom
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
    ];

    const UVS: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]];
    let mut mesh_uvs = Vec::new();
    for _ in 0..6 {
        mesh_uvs.extend(UVS);
    }

    const INDICES: [u32; 6] = [0, 1, 2, 2, 1, 3];
    let mut mesh_indices = Vec::new();
    for i in 0..6 {
        mesh_indices.extend(INDICES.iter().map(|x| x + 4 * i));
    }

    Model::Custom {
        mesh_indices,
        mesh_vertices,
        mesh_normals,
        mesh_uvs: Some(mesh_uvs),
        material_color_texture: None,
        material_parallax_texture: Some("blocks/breaking_1.png".to_owned()),
        material_alpha_mode: 2,
        material_alpha_cutoff: 0.0,
        material_double_sided: false,
        collider: None,
    }
}

fn handle_right_clicks(
    net: Res<Server>,
    world_map: Res<WorldMap>,
    items: Res<Items>,
    item_registry: Res<ItemRegistry>,
    model_map: Res<ModelMap>,
    chunk_subscriptions: Res<ChunkSubscriptions>,
    model_query: Query<(&Collider, &GlobalTransform), (With<Model>, Without<BlockPosition>)>,
    mut player_query: Query<(&mut Hotbar, &Targets, &Camera), With<Player>>,
    mut item_use_query: Query<&mut ItemUses>,
    mut hand_interaction_query: Query<&mut HandInteractions>,
    mut block_update_writer: EventWriter<BlockUpdate>,
    mut clicks: EventReader<NetworkMessage<messages::RightClick>>,
    mut rng: Local<Rng>,
) {
    // TODO: ActionOrder currently does nothing, but there needs to be some system for deviating
    // from the set order. Like if you hold shift, placing blocks should take precedence over
    // interacting. And there's a bunch of stuff like this where you want to do something else
    // depending on some condition.
    enum ActionOrder {
        Interact,
        PlaceBlock,
        UseItem,
    }

    for right_click in clicks.read() {
        let (mut hotbar, targets, camera) =
            player_query.get_mut(right_click.player_entity).unwrap();

        let mut action = ActionOrder::Interact;

        'outer: loop {
            match action {
                ActionOrder::Interact => {
                    for target in targets.iter() {
                        let Some(entity) = target.entity() else {
                            continue;
                        };

                        if let Ok(mut interactions) = hand_interaction_query.get_mut(entity) {
                            interactions.push(right_click.player_entity);
                            break 'outer;
                        }
                    }

                    action = ActionOrder::PlaceBlock;
                }
                ActionOrder::PlaceBlock => {
                    let blocks = Blocks::get();

                    let Some(Target::Block {
                        block_position,
                        block_id,
                        block_face,
                        ..
                    }) = targets
                        .get_first_block(|block_id| blocks.get_config(block_id).hardness.is_some())
                    else {
                        action = ActionOrder::UseItem;
                        continue;
                    };

                    let blocks = Blocks::get();
                    let equipped_item_stack = hotbar.held_item_stack_mut();

                    if let Some((block_id, replaced_block_position)) = block_placement(
                        &equipped_item_stack,
                        *block_id,
                        *block_face,
                        *block_position,
                        &items,
                        &blocks,
                        &world_map,
                    ) {
                        let block_config = blocks.get_config(&block_id);
                        let block_state = block_config.placement_rotation(*block_face, &camera);

                        let replaced_collider = Collider::Aabb(Aabb {
                            center: replaced_block_position.as_dvec3(),
                            half_extents: DVec3::splat(0.5),
                        });

                        // Check that there aren't any entities in the way of the new block
                        let chunk_position = ChunkPosition::from(replaced_block_position);
                        if let Some(entities) = model_map.get_entities(&chunk_position) {
                            for (collider, global_transform) in model_query.iter_many(entities) {
                                if collider
                                    .intersection(
                                        &global_transform.compute_transform(),
                                        &replaced_collider,
                                        &Transform::IDENTITY,
                                    )
                                    .is_some()
                                {
                                    continue;
                                }
                            }
                        }

                        equipped_item_stack.take(1);

                        if let Some(subscribers) =
                            chunk_subscriptions.get_subscribers(&chunk_position)
                        {
                            let position = block_position.as_dvec3() + DVec3::splat(0.5);

                            if let Some(place_sound) = block_config.sound.place(&mut rng) {
                                net.send_many(
                                    subscribers,
                                    messages::Sound {
                                        position: Some(position),
                                        volume: 1.0,
                                        speed: 1.0,
                                        sound: place_sound.to_owned(),
                                    },
                                )
                            }
                        }

                        block_update_writer.send(BlockUpdate::Replace {
                            position: replaced_block_position,
                            block_id,
                            block_state,
                            block_data: None,
                        });

                        break;
                    } else {
                        action = ActionOrder::UseItem;
                    }
                }
                ActionOrder::UseItem => {
                    // If nothing else was done, we try to use the item
                    let equipped_item_stack = hotbar.held_item_stack_mut();

                    let Some(item) = equipped_item_stack.item() else {
                        break;
                    };

                    if let Some(item_use_entity) = item_registry.get(&item.id) {
                        let mut uses = item_use_query.get_mut(*item_use_entity).unwrap();
                        uses.push(right_click.player_entity);
                    }

                    break;
                }
            }
        }
    }
}

fn block_placement(
    equipped_item_stack: &ItemStack,
    block_id: BlockId,
    block_face: BlockFace,
    block_position: BlockPosition,
    items: &Items,
    blocks: &Blocks,
    world_map: &WorldMap,
) -> Option<(BlockId, BlockPosition)> {
    let against_block = blocks.get_config(&block_id);

    if !against_block.is_solid() {
        return None;
    }

    let Some(item) = equipped_item_stack.item() else {
        // No item equipped, can't place block
        return None;
    };

    let item_config = items.get_config(&item.id);

    let Some(new_block_id) = item_config.block else {
        // The item isn't bound to a placeable block
        return None;
    };

    if !blocks.get_config(&new_block_id).is_placeable(block_face) {
        return None;
    }

    let replaced_block_position = if against_block.replaceable {
        // Some blocks, like grass, can be replaced instead of placing the new
        // block adjacently to it.
        block_position
    } else if let Some(block_id) = world_map.get_block(block_face.shift_position(block_position)) {
        if !blocks.get_config(&block_id).replaceable {
            return None;
        }
        block_face.shift_position(block_position)
    } else {
        return None;
    };

    return Some((new_block_id, replaced_block_position));
}
