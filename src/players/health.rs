use fmc::{
    bevy::math::DVec3,
    interfaces::{InterfaceEventRegistration, InterfaceEvents, RegisterInterfaceNode},
    items::ItemStack,
    networking::{NetworkMessage, Server},
    physics::Physics,
    players::Player,
    prelude::*,
    protocol::messages,
    utils::Rng,
};

use serde::{Deserialize, Serialize};

use crate::items::DroppedItem;

use super::{Equipment, Inventory, RespawnEvent};

pub struct HealthPlugin;
impl Plugin for HealthPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<DamageEvent>()
            .add_event::<HealEvent>()
            .add_systems(
                Update,
                (
                    register_death_interface,
                    change_health,
                    fall_damage.before(change_health),
                    death_interface.after(InterfaceEventRegistration),
                ),
            );
    }
}

#[derive(Default, Bundle)]
pub struct HealthBundle {
    pub health: Health,
    fall_damage: FallDamage,
}

impl HealthBundle {
    pub fn from_health(health: Health) -> Self {
        Self {
            health,
            ..default()
        }
    }
}

#[derive(Component, Serialize, Deserialize, Clone)]
pub struct Health {
    hearts: u32,
    max: u32,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            hearts: 20,
            max: 20,
        }
    }
}

impl Health {
    fn build_interface(&self) -> messages::InterfaceNodeVisibilityUpdate {
        let mut image_update = messages::InterfaceNodeVisibilityUpdate::default();

        for i in 0..self.hearts {
            image_update.set_visible(format!("health/{}", i + 1));
        }

        for i in self.hearts..self.max {
            image_update.set_hidden(format!("health/{}", i + 1));
        }

        image_update
    }

    fn take_damage(&mut self, damage: u32) -> messages::InterfaceNodeVisibilityUpdate {
        let old_hearts = self.hearts;
        self.hearts = self.hearts.saturating_sub(damage);

        let mut image_update = messages::InterfaceNodeVisibilityUpdate::default();
        for i in self.hearts..old_hearts {
            image_update.set_hidden(format!("health/{}", i + 1));
        }

        image_update
    }

    fn heal(&mut self, healing: u32) -> messages::InterfaceNodeVisibilityUpdate {
        let old_hearts = self.hearts;
        self.hearts = self.hearts.saturating_add(healing).min(self.max);

        let mut image_update = messages::InterfaceNodeVisibilityUpdate::default();
        for i in old_hearts..self.hearts {
            image_update.set_visible(format!("health/{}", i + 1));
        }

        image_update
    }

    pub fn is_dead(&self) -> bool {
        self.hearts == 0
    }
}

#[derive(Component, Default)]
struct FallDamage(u32);

#[derive(Event)]
pub struct DamageEvent {
    pub player_entity: Entity,
    pub damage: u32,
}

#[derive(Event)]
pub struct HealEvent {
    pub player_entity: Entity,
    pub healing: u32,
}

fn fall_damage(
    mut fall_damage_query: Query<(Entity, &mut FallDamage), With<Player>>,
    mut position_events: EventReader<NetworkMessage<messages::PlayerPosition>>,
    mut damage_events: EventWriter<DamageEvent>,
) {
    for position_update in position_events.read() {
        let (entity, mut fall_damage) = fall_damage_query
            .get_mut(position_update.player_entity)
            .unwrap();

        if fall_damage.0 != 0 && position_update.velocity.y > -0.1 {
            damage_events.send(DamageEvent {
                player_entity: entity,
                damage: fall_damage.0,
            });
            fall_damage.0 = 0;
        } else if position_update.velocity.y < 0.0 {
            fall_damage.0 = (position_update.velocity.y.abs() as u32).saturating_sub(20);
        }
    }
}

fn change_health(
    mut commands: Commands,
    net: Res<Server>,
    mut health_query: Query<(
        Entity,
        &Transform,
        &mut Inventory,
        Mut<Equipment>,
        Mut<Health>,
    )>,
    mut damage_events: EventReader<DamageEvent>,
    mut heal_events: EventReader<HealEvent>,
    mut rng: Local<Rng>,
) {
    for (player_entity, _, _, _, health) in health_query.iter() {
        if health.is_added() {
            net.send_one(player_entity, health.build_interface());
        }
    }
    for damage_event in damage_events.read() {
        let (_, transform, mut inventory, mut equipment, mut health) =
            health_query.get_mut(damage_event.player_entity).unwrap();
        let interface_update = health.take_damage(damage_event.damage);

        net.send_one(damage_event.player_entity, interface_update);
        net.broadcast(messages::Sound {
            position: Some(transform.translation),
            volume: 1.0,
            speed: 1.0,
            sound: "player_damage.ogg".to_owned(),
        });

        if health.hearts == 0 {
            // Reborrow to enable split borrowing
            let equipment = equipment.into_inner();

            for item_stack in inventory.iter_mut().chain([
                &mut equipment.helmet,
                &mut equipment.chestplate,
                &mut equipment.leggings,
                &mut equipment.boots,
            ]) {
                if item_stack.is_empty() {
                    continue;
                }

                let random_direction = (rng.next_f32() * std::f32::consts::TAU) as f64;
                let velocity_x = random_direction.sin() as f64 * 15.0 * rng.next_f32() as f64;
                let velocity_z = random_direction.cos() as f64 * 15.0 * rng.next_f32() as f64;
                let velocity_y = 6.5;

                let mut new_item_stack = ItemStack::default();
                item_stack.swap(&mut new_item_stack);
                commands.spawn((
                    DroppedItem::new(new_item_stack),
                    transform.clone(),
                    Physics {
                        velocity: DVec3::new(velocity_x, velocity_y, velocity_z),
                        ..default()
                    },
                ));
            }
            net.send_one(
                damage_event.player_entity,
                messages::InterfaceVisibilityUpdate {
                    interface_path: "death".to_owned(),
                    visible: true,
                },
            );
        }
    }

    for heal_event in heal_events.read() {
        let (_, _, _, _, mut health) = health_query.get_mut(heal_event.player_entity).unwrap();
        let interface_update = health.heal(heal_event.healing);
        net.send_one(heal_event.player_entity, interface_update);
    }
}

#[derive(Component)]
struct DeathInterface;

fn register_death_interface(
    mut commands: Commands,
    new_player_query: Query<Entity, Added<Player>>,
    mut registration_events: EventWriter<RegisterInterfaceNode>,
) {
    for player_entity in new_player_query.iter() {
        commands.entity(player_entity).with_children(|parent| {
            let death_interface_entity = parent.spawn(DeathInterface).id();

            registration_events.send(RegisterInterfaceNode {
                player_entity,
                node_path: String::from("death/respawn_button"),
                node_entity: death_interface_entity,
            });
        });
    }
}

// TODO: This should test that your health is zero. The parent of the DeathInterface is the player
// it belongs to, just query for parent.
fn death_interface(
    net: Res<Server>,
    mut interface_query: Query<
        &mut InterfaceEvents,
        (Changed<InterfaceEvents>, With<DeathInterface>),
    >,
    mut respawn_events: EventWriter<RespawnEvent>,
) {
    for mut interface_events in interface_query.iter_mut() {
        for interface_interaction in interface_events.read() {
            if !matches!(
                *interface_interaction,
                messages::InterfaceInteraction::Button { .. }
            ) {
                continue;
            }

            respawn_events.send(RespawnEvent {
                player_entity: interface_interaction.player_entity,
            });

            net.send_one(
                interface_interaction.player_entity,
                messages::InterfaceVisibilityUpdate {
                    interface_path: "death".to_owned(),
                    visible: false,
                },
            );
        }
    }
}
