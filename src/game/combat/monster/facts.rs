//! Reading the fight for the decisions of [`super`]: what each takes is a
//! small struct of facts, built here from one view of a body
//! ([`BodyReadOnlyItem`]) and from where everyone stands. Nothing here decides
//! anything and nothing here changes anything.

use bevy::ecs::query::QueryData;
use bevy::prelude::*;

use super::Deliberation;
use super::ailments::Ailing;
use super::close::Striking;
use super::coming::Standing;
use super::conduct::{Conducting, share};
use super::guard::{Fellow, Guarding};
use super::special::Making;
use super::spell::{self, Aiming, Candidate, Casting, Slot};
use crate::content::characters::Attribute;
use crate::content::species::Reach;
use crate::content::spells::{SpellCatalog, SpellDefinition, SpellId};
use crate::game::actors::{Eyes, Hidden, Laid, Places, reaches};
use crate::game::characters::Condition;
use crate::game::combat::turn::{Fleeing, SpecialCooldown};
use crate::game::hits::definitions::Definitions;
use crate::game::hits::effects::BodyReadOnly;
use crate::game::magic::reach::Aim;
use crate::game::npcs::NpcBody;
use crate::game::party::formation::Formation;
use crate::game::party::roster::PartyRoster;
use crate::game::world::hostility::{self, Loyalty, Stance};
use crate::game::world::identify::GroupMember;
use crate::game::world::obstacles::WorldObstacles;
use crate::game::world::population::{Special, Spellcasting, Temper};

/// What is looked at to learn the facts: everyone's bodies and places, the
/// spells laid, the party, the level's far clip.
pub struct Sight<'a, 'w, 's> {
    pub bodies: &'a Query<'w, 's, BodyReadOnly>,
    pub places: &'a Places<'w, 's>,
    pub laid: &'a Laid<'w, 's>,
    pub groups: &'a Query<'w, 's, &'static GroupMember>,
    /// The party, which every member stands in.
    pub party: Entity,
    pub roster: &'a PartyRoster,
    /// The far clip, past which nobody sees anybody.
    pub far: f32,
    pub obstacles: &'a WorldObstacles<'w, 's>,
}

/// The records the spells a monster casts are read from.
#[derive(Clone, Copy)]
pub struct Records<'a> {
    pub spells: &'a SpellCatalog,
    pub definitions: &'a Definitions,
}

/// The fight a monster is deciding in.
#[derive(Clone, Copy)]
pub struct Fray<'a> {
    /// The bodies that may take part in it: the monsters and the members.
    pub bodies: &'a [Entity],
    /// A fight is on.
    pub fighting: bool,
    /// The monster's eyes make out the party.
    pub sees_party: bool,
}

/// What a monster's kind and state give it, from its own components.
#[derive(Clone, Copy)]
pub struct Kind<'a> {
    pub temper: &'a Temper,
    pub special: &'a Special,
    pub casting: &'a Spellcasting,
    /// Its special attack is cooling down.
    pub cooling: bool,
    pub npc: bool,
    /// It ran in an earlier round.
    pub fleeing: bool,
}

/// A monster's own components that its decisions rest on.
#[derive(QueryData)]
pub struct Nature {
    temper: &'static Temper,
    special: &'static Special,
    casting: &'static Spellcasting,
    cooldown: Option<&'static SpecialCooldown>,
    npc: Has<NpcBody>,
    pub fleeing: Has<Fleeing>,
}

impl NatureItem<'_, '_> {
    /// What its kind and state give it.
    pub fn kind(&self) -> Kind<'_> {
        Kind {
            temper: self.temper,
            special: self.special,
            casting: self.casting,
            cooling: self.cooldown.is_some_and(|cooldown| cooldown.0 > 0),
            npc: self.npc,
            fleeing: self.fleeing,
        }
    }
}

/// What an actor is about to do, as its ailments weigh it.
#[derive(Clone, Copy, Debug)]
pub struct Plans {
    /// Fear may send it running: it decides for itself.
    pub runs: bool,
    /// It is running for its life already.
    pub running: bool,
    /// Insanity has already turned it on its friends this round.
    pub turned_on_friends: bool,
    /// What it was about to do is aimed at its foes.
    pub against_foes: bool,
}

/// A living body in the fight as a caster weighs it, apart from any spell.
#[derive(Clone, Copy, Debug)]
pub struct Surveyed {
    body: Entity,
    member: bool,
    group: Option<Entity>,
    seen: bool,
    stance: Stance,
}

impl Sight<'_, '_, '_> {
    pub fn alive(&self, body: Entity) -> bool {
        self.bodies.get(body).is_ok_and(|body| !body.vitals.is_dead())
    }

    /// The side a body's group or kind has it on, its own turning aside.
    pub fn side(&self, body: Entity) -> Option<Stance> {
        self.bodies.get(body).ok().map(|body| body.side.own())
    }

    /// How far a body is from the party.
    pub fn party_gap(&self, body: Entity) -> f32 {
        self.places.gap(body, self.party)
    }

    /// Whether the party has a clear line to a body within the far clip.
    pub fn in_party_view(&self, body: Entity) -> bool {
        self.places.of(self.party).in_view(&self.places.of(body), self.far, self.obstacles)
    }

    /// Whether a body's eyes make out the party, which hides by its lowest
    /// able level and by Chameleon.
    pub fn sees_party(&self, watcher: Entity) -> bool {
        let Ok(own) = self.bodies.get(watcher) else { return false };
        let members = self
            .bodies
            .iter_many(self.roster.iter())
            .matched()
            .map(|member| (member.level, member.vitals));
        let hidden = Hidden::party(members, self.laid.bonus(self.party).concealment);
        Eyes::of(own.vitals, own.attributes, own.attacks).see(
            &self.places.of(watcher),
            (&self.places.of(self.party), hidden),
            self.far,
            self.obstacles,
        )
    }

    /// Whether a body's eyes make out a monster, which hides by its level.
    pub fn sees(&self, watcher: Entity, watched: Entity) -> bool {
        let (Ok(own), Ok(there)) = (self.bodies.get(watcher), self.bodies.get(watched)) else {
            return false;
        };
        Eyes::of(own.vitals, own.attributes, own.attacks).see(
            &self.places.of(watcher),
            (&self.places.of(watched), Hidden::alone(*there.level)),
            self.far,
            self.obstacles,
        )
    }

    /// Whether a body sees a living foe of its own among `others`: another
    /// monster it sees, or the party, if it makes it out and a member
    /// stands against it.
    pub fn sees_foe(&self, watcher: Entity, others: &[Entity]) -> bool {
        let Ok(own) = self.bodies.get(watcher) else { return false };
        let foe = |other: Entity| {
            self.bodies.get(other).is_ok_and(|their| {
                !their.vitals.is_dead()
                    && hostility::between(own.standing(), their.standing()) == Stance::Hostile
            })
        };
        let monsters = others
            .iter()
            .any(|&other| !self.roster.contains(&other) && foe(other) && self.sees(watcher, other));
        monsters
            || (others.iter().any(|&other| self.roster.contains(&other) && foe(other))
                && self.sees_party(watcher))
    }

    /// A body as the reinforcement rule weighs it.
    pub fn standing(&self, body: Entity, camouflaged: bool) -> Standing {
        let place = self.places.of(body);
        Standing {
            body,
            feet: place.feet,
            party_gap: self.party_gap(body),
            in_view: self.in_party_view(body),
            camouflaged,
        }
    }

    /// What ails an actor, as it weighs its action: its conditions, Strength,
    /// whether it fights blind, and whether one of its arms reaches a friend
    /// it sees (a living body on its side).
    pub fn ailing(
        &self,
        actor: Entity,
        (combatants, formation): (&[Entity], &Formation),
        plans: Plans,
    ) -> Option<Ailing> {
        let own = self.bodies.get(actor).ok()?;
        let has = |condition| own.vitals.conditions.has(condition);
        let mine = own.standing();
        let at = self.places.of(actor);
        let friend_in_reach = own.attacks.arms.iter().any(|arm| {
            combatants.iter().copied().any(|other| {
                other != actor
                    && self.bodies.get(other).is_ok_and(|their| {
                        !their.vitals.is_dead()
                            && hostility::between(mine, their.standing()) == Stance::Friendly
                            && reaches(
                                formation,
                                (actor, &at),
                                (other, &self.places.of(other)),
                                arm.reach,
                            )
                    })
            })
        });
        Some(Ailing {
            nauseated: has(Condition::Nauseated),
            webbed: has(Condition::Webbed),
            blind: has(Condition::Blind),
            insane: has(Condition::Insane),
            afraid: has(Condition::Afraid),
            turncoat: has(Condition::Turncoat),
            strength: own.attributes[Attribute::Strength],
            fights_blind: own.attacks.blind_fighting.is_some(),
            runs: plans.runs,
            running: plans.running,
            friend_in_reach,
            turned_on_friends: plans.turned_on_friends,
            against_foes: plans.against_foes,
        })
    }

    /// The living bodies among `bodies` as a caster weighs them: whether each
    /// is seen (a member only if the party is made out), its group and how the
    /// caster stands toward it, itself among them.
    pub fn survey(&self, caster: Entity, sees_party: bool, bodies: &[Entity]) -> Vec<Surveyed> {
        let Ok(own) = self.bodies.get(caster) else { return Vec::new() };
        let eye = self.places.of(caster).eye;
        bodies
            .iter()
            .filter_map(|&body| {
                let their = self.bodies.get(body).ok().filter(|b| !b.vitals.is_dead())?;
                let member = self.roster.contains(&body);
                Some(Surveyed {
                    body,
                    member,
                    group: self.groups.get(body).ok().map(|group| group.0),
                    seen: if member {
                        sees_party
                    } else {
                        body == caster || self.obstacles.clear(eye, self.places.of(body).eye)
                    },
                    stance: if body == caster {
                        Stance::Friendly
                    } else {
                        hostility::between(own.standing(), their.standing())
                    },
                })
            })
            .collect()
    }

    /// The bodies surveyed as a caster weighs them for one spell.
    fn candidates(
        &self,
        caster: Entity,
        spell: &SpellDefinition,
        definitions: &Definitions,
        surveyed: &[Surveyed],
    ) -> Vec<Candidate> {
        let reach = spell.reach.metres();
        surveyed
            .iter()
            .filter_map(|s| {
                let their = self.bodies.get(s.body).ok()?;
                Some(Candidate {
                    body: s.body,
                    member: s.member,
                    group: s.group,
                    seen: s.seen,
                    stance: s.stance,
                    reached: self.places.gap(caster, s.body) <= reach,
                    worth: definitions.worthwhile(
                        spell.id,
                        their.vitals,
                        self.laid.has(s.body, spell.id),
                    ),
                })
            })
            .collect()
    }

    /// At whom a spell may be aimed by a caster.
    fn aims(
        &self,
        caster: Entity,
        spell: &SpellDefinition,
        records: Records,
        surveyed: &[Surveyed],
    ) -> Vec<Aim> {
        let own_group = self.groups.get(caster).ok().map(|group| group.0);
        let bodies = self.candidates(caster, spell, records.definitions, surveyed);
        spell::aims(&Aiming { targeting: spell.targeting, caster, own_group, bodies: &bodies })
    }

    /// What a monster knows, to decide from: of itself and of the `bodies` it
    /// may take part in a fight with. `fighting` is whether a fight is on.
    pub fn know<'c>(
        &self,
        monster: Entity,
        kind: Kind,
        Fray { bodies, fighting, sees_party }: Fray,
        records: Records<'c>,
    ) -> Option<Known<'c>> {
        let own = self.bodies.get(monster).ok()?;
        let (temper, special, casting) = (kind.temper.0, kind.special.0, kind.casting);
        let party_gap = self.party_gap(monster);
        let arms: Vec<Reach> = own.attacks.arms.iter().map(|arm| arm.reach).collect();
        let surveyed = self.survey(monster, sees_party, bodies);
        let silenced = own.vitals.conditions.has(Condition::Silenced);
        let blind =
            own.vitals.conditions.has(Condition::Blind) && own.attacks.blind_fighting.is_none();
        let striking =
            |arms| Striking { arms, party_gap, prefers_range: temper.prefers_range, blind };

        // A breath is aimed as a cone spell is.
        let cone = records.spells.get(SpellId::SPECIAL_ATTACK_CONE);
        let aims = match (special.kind, cone) {
            (Some(kind), Some(cone))
                if crate::game::hits::specials::special(kind).summons.is_none() =>
            {
                self.aims(monster, cone, records, &surveyed)
            }
            _ => Vec::new(),
        };
        let summons = special
            .kind
            .is_some_and(|kind| crate::game::hits::specials::special(kind).summons.is_some());
        let stamina = own.vitals.stamina;
        let making = Making {
            has: special.kind.is_some(),
            summons,
            chance: special.chance,
            cooling: kind.cooling,
            stamina: stamina.current,
            stamina_most: stamina.max,
            silenced,
            honours_silence: special
                .kind
                .is_some_and(|kind| crate::game::hits::specials::special(kind).silenced),
            striking: striking(&[]),
            fighting,
            stance: own.stance(),
            aims: &[],
        };

        // Each slot's spell, with the aims a castable one has.
        let slots: Vec<Option<Slot<'c>>> = casting
            .spells
            .iter()
            .map(|id| {
                let spell = records.spells.get((*id)?)?;
                let modelled = records.definitions.spell(spell.id).is_some();
                let castable = spell::castable(spell, modelled, silenced, casting.silent_alchemy);
                let aims = if casting.chance != 0 && castable {
                    self.aims(monster, spell, records, &surveyed)
                } else {
                    Vec::new()
                };
                Some(Slot { spell, modelled, aims })
            })
            .collect();

        let mine = own.standing();
        let reach = own.attacks.arms.first().map(|arm| arm.reach.metres());
        let fellows: Vec<Fellow> = bodies
            .iter()
            .copied()
            .filter(|&other| other != monster)
            .filter_map(|other| {
                let fellow = self.bodies.get(other).ok().filter(|f| !f.vitals.is_dead())?;
                Some(Fellow {
                    body: other,
                    friendly: hostility::between(mine, fellow.standing()) == Stance::Friendly,
                    in_reach: reach.is_some_and(|reach| self.places.gap(monster, other) <= reach),
                    level: fellow.level.0,
                    health: share(fellow.vitals.health.current, fellow.vitals.health.max),
                })
            })
            .collect();

        Some(Known {
            conducting: Conducting {
                health: share(own.vitals.health.current, own.vitals.health.max),
                flee_chance: temper.flee_chance,
                craven: temper.craven,
                camouflaged: temper.camouflaged,
                npc: kind.npc,
                bystander: *own.loyalty == Loyalty::Bystander,
                stance: own.stance(),
                fleeing: kind.fleeing,
                party_gap,
            },
            arms,
            striking: striking(&[]),
            making,
            aims,
            casting: Casting {
                chance: casting.chance,
                striking: striking(&[]),
                silenced,
                silent_alchemy: casting.silent_alchemy,
                budget: own.budget.map(|budget| budget.0),
                slots: &[],
            },
            slots,
            guarding: Guarding {
                striking: striking(&[]),
                bodyguard: temper.bodyguard,
                able: own.vitals.can_guard(),
                wary: temper.wary,
                intelligence: own.attributes[Attribute::Intelligence],
                level: own.level.0,
                fellows: &[],
            },
            fellows,
        })
    }
}

/// What a monster knows, owned: [`Known::deliberation`] lends it to the
/// decisions.
pub struct Known<'c> {
    conducting: Conducting,
    arms: Vec<Reach>,
    striking: Striking<'static>,
    making: Making<'static>,
    aims: Vec<Aim>,
    casting: Casting<'static>,
    slots: Vec<Option<Slot<'c>>>,
    guarding: Guarding<'static>,
    fellows: Vec<Fellow>,
}

impl Known<'_> {
    /// The facts of the whole decision, in the order of section 4.
    pub fn deliberation(&self) -> Deliberation<'_> {
        let striking = self.striking();
        Deliberation {
            conducting: self.conducting,
            striking,
            special: Making { striking, aims: &self.aims, ..self.making },
            casting: Casting { striking, slots: &self.slots, ..self.casting },
            guarding: Guarding { striking, fellows: &self.fellows, ..self.guarding },
        }
    }

    /// What its arms reach of the party.
    pub fn striking(&self) -> Striking<'_> {
        Striking { arms: &self.arms, ..self.striking }
    }

    /// Its special attack: whether it is ready, with an aim, and so may open a
    /// fight.
    pub fn special(&self) -> Making<'_> {
        Making { striking: self.striking(), aims: &self.aims, ..self.making }
    }

    /// Its casting.
    pub fn casting(&self) -> Casting<'_> {
        Casting { striking: self.striking(), slots: &self.slots, ..self.casting }
    }
}

#[cfg(test)]
mod tests {
    use avian3d::prelude::Position;
    use bevy::ecs::system::RunSystemOnce;

    use super::*;
    use crate::content::species::{MonsterMagic, Species};
    use crate::content::spells::Targeting;
    use crate::game::actors::{Armour, Attacks, Attributes, Level, Resistances};
    use crate::game::characters::{Lasting, Vitals};
    use crate::game::data::Catalogs;
    use crate::game::party::body::PartyBody;
    use crate::game::rng::GameRng;
    use crate::game::world::hostility::Side;
    use crate::game::world::population::{Monster, ScaledBody};

    fn spell(id: SpellId, targeting: Targeting) -> SpellDefinition {
        SpellDefinition {
            id,
            targeting,
            level: 1,
            cost: 2,
            monsters_cast: true,
            realm: Some(crate::content::characters::Realm::Divine),
            reach: Reach::Long,
            ..Default::default()
        }
    }

    /// A monster at `distance` from the origin on a side, hurt or whole.
    fn body(world: &mut World, stance: Stance, distance: f32, hurt: bool) -> Entity {
        let mut vitals = Vitals::full(10, 10);
        vitals.health.current -= u32::from(hurt);
        world
            .spawn((
                vitals,
                Side::new(stance),
                Loyalty::None,
                Attributes::default(),
                Level::default(),
                Resistances::default(),
                Armour::default(),
                Attacks::default(),
                crate::game::world::population::SpellBudget(50),
                Monster {
                    location: 0,
                    species: 0,
                    body: ScaledBody { scale: 1.0, radius: 0.5, eye_height: 1.0, speed: 1.0 },
                },
                Transform::from_translation(Vec3::Z * distance),
            ))
            .id()
    }

    /// A monster heals itself only when hurt, aims its bolt at a member in
    /// reach, and a turned one blasts its fellow rather than the party.
    #[test]
    fn a_monster_casts_what_would_work_at_its_chance() {
        let mut catalog = SpellCatalog(vec![SpellDefinition::default(); 80]);
        catalog.0[SpellId::HEAL_WOUNDS.0 as usize] = spell(SpellId::HEAL_WOUNDS, Targeting::Caster);
        catalog.0[SpellId::ENERGY_BLAST.0 as usize] =
            spell(SpellId::ENERGY_BLAST, Targeting::Enemy);
        let mut world = World::new();
        world.init_resource::<avian3d::collider_tree::ColliderTrees>();
        world.insert_resource(Catalogs { spells: catalog, ..default() });
        world.run_system_once(crate::game::hits::definitions::define).unwrap();
        world.flush();
        let caster = body(&mut world, Stance::Hostile, 0.0, false);
        let hurt = body(&mut world, Stance::Hostile, 0.0, true);
        // The party's members stand where it does: it is moved to put the
        // member in reach or out of it.
        let member = world
            .spawn((
                Vitals::full(10, 10),
                Side::FRIENDLY,
                Loyalty::None,
                Attributes::default(),
                Level::default(),
                Resistances::default(),
                Armour::default(),
                Attacks::default(),
            ))
            .id();
        let party = crate::game::party::roster::test_party(&mut world, &[member]);
        world.entity_mut(party).insert((PartyBody::default(), Position(Vec3::Z * 10.0)));
        let fellow = body(&mut world, Stance::Hostile, 10.0, false);
        let mut spells = [None; 10];
        spells[0] = Some(SpellId::HEAL_WOUNDS);
        let healer =
            Species { magic: MonsterMagic { chance: 50, spells, budget: 50 }, ..default() };
        let mut spells = [None; 10];
        spells[9] = Some(SpellId::ENERGY_BLAST);
        let blaster =
            Species { magic: MonsterMagic { chance: 50, spells, budget: 50 }, ..default() };
        // What the monster casts, at a chance of 100 or 0, among those it sees
        // (the party in it only if it makes it out, which a close monster does).
        let choose =
            |world: &mut World, caster: Entity, kind: &Species, chance: u8, fray: &[Entity]| {
                let casting = Spellcasting { chance, ..Spellcasting::of(kind) };
                let fray = fray.to_vec();
                world
                    .run_system_once(
                        move |bodies: Query<BodyReadOnly>,
                              roster: Single<(Entity, &PartyRoster)>,
                              definitions: Res<Definitions>,
                              laid: Laid,
                              places: Places,
                              obstacles: WorldObstacles,
                              groups: Query<&'static GroupMember>,
                              content: Res<Catalogs>| {
                            let (party, roster) = *roster;
                            let sight = Sight {
                                bodies: &bodies,
                                places: &places,
                                laid: &laid,
                                groups: &groups,
                                party,
                                roster,
                                far: 100.0,
                                obstacles: &obstacles,
                            };
                            let kind = Kind {
                                temper: &Temper(default()),
                                special: &Special(default()),
                                casting: &casting,
                                cooling: false,
                                npc: false,
                                fleeing: false,
                            };
                            let records =
                                Records { spells: &content.spells, definitions: &definitions };
                            let sees_party = sight.sees_party(caster);
                            let fray = Fray { bodies: &fray, fighting: true, sees_party };
                            let known = sight.know(caster, kind, fray, records).unwrap();
                            let mut rng = GameRng::seeded(3);
                            spell::casts(&mut rng, &known.casting())
                                .map(|cast| (cast.spell, cast.aim))
                        },
                    )
                    .unwrap()
            };
        assert_eq!(choose(&mut world, caster, &healer, 100, &[caster, member]), None, "whole");
        assert_eq!(
            choose(&mut world, hurt, &healer, 100, &[hurt, member]),
            Some((SpellId::HEAL_WOUNDS, Aim::Body(hurt)))
        );
        assert_eq!(choose(&mut world, hurt, &healer, 0, &[hurt, member]), None);
        assert_eq!(
            choose(&mut world, caster, &blaster, 100, &[caster, member]),
            Some((SpellId::ENERGY_BLAST, Aim::Body(member)))
        );
        world.entity_mut(party).insert(Position(Vec3::Z * 30.0));
        assert_eq!(
            choose(&mut world, caster, &blaster, 100, &[caster, member]),
            None,
            "out of reach"
        );
        // Turned, it stands with the party and against its fellow.
        world.entity_mut(party).insert(Position(Vec3::Z * 10.0));
        world.get_mut::<Vitals>(caster).unwrap().afflict(Condition::Turncoat, Lasting::Rounds(5));
        assert_eq!(
            choose(&mut world, caster, &blaster, 100, &[caster, member, fellow]),
            Some((SpellId::ENERGY_BLAST, Aim::Body(fellow))),
            "a turned caster blasts its fellow, not the party"
        );
    }

    /// A party member under Turncoat is a monster's friend, so a candidate
    /// ward by the same rule as a monster: the page's Open question about
    /// members is not a difference of the code.
    #[test]
    fn a_turned_member_is_a_fellow_on_the_monsters_side() {
        let mut world = World::new();
        world.init_resource::<avian3d::collider_tree::ColliderTrees>();
        world.insert_resource(Catalogs::default());
        world.run_system_once(crate::game::hits::definitions::define).unwrap();
        let monster = body(&mut world, Stance::Hostile, 0.0, false);
        let mut hurt = Vitals::full(10, 10);
        hurt.health.current = 3;
        hurt.afflict(Condition::Turncoat, Lasting::Rounds(5));
        let member = world
            .spawn((
                hurt,
                Side::FRIENDLY,
                Loyalty::None,
                Level(9),
                Attributes::default(),
                Resistances::default(),
                Armour::default(),
                Attacks::default(),
            ))
            .id();
        let party = crate::game::party::roster::test_party(&mut world, &[member]);
        world.entity_mut(party).insert((PartyBody::default(), Position(Vec3::Z * 10.0)));
        let fellows = world
            .run_system_once(
                move |bodies: Query<BodyReadOnly>,
                      roster: Single<(Entity, &PartyRoster)>,
                      definitions: Res<Definitions>,
                      laid: Laid,
                      places: Places,
                      obstacles: WorldObstacles,
                      groups: Query<&'static GroupMember>,
                      content: Res<Catalogs>| {
                    let (party, roster) = *roster;
                    let sight = Sight {
                        bodies: &bodies,
                        places: &places,
                        laid: &laid,
                        groups: &groups,
                        party,
                        roster,
                        far: 100.0,
                        obstacles: &obstacles,
                    };
                    let kind = Kind {
                        temper: &Temper(default()),
                        special: &Special(default()),
                        casting: &Spellcasting::of(&Species::default()),
                        cooling: false,
                        npc: false,
                        fleeing: false,
                    };
                    let records = Records { spells: &content.spells, definitions: &definitions };
                    let fray =
                        Fray { bodies: &[monster, member], fighting: true, sees_party: true };
                    let known = sight.know(monster, kind, fray, records).unwrap();
                    known.deliberation().guarding.fellows.to_vec()
                },
            )
            .unwrap();
        assert_eq!(fellows.len(), 1);
        let ward = fellows[0];
        // Friendly toward the hostile monster; level 9; 3 of 10 health is 30 percent.
        assert!(ward.friendly && ward.body == member);
        assert_eq!((ward.level, ward.health), (9, 30));
    }
}
