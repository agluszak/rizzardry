//! A monster's script (`docs/rules/scripts.md`): what it does in order
//! while it is not fighting, and the standing orders it ends with.

use std::collections::HashMap;

use bevy::prelude::*;

/// A place a script names.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptPlace {
    Party,
    /// Where the monster was placed.
    Home,
    /// A named spot of the level.
    Spot(String),
}

impl ScriptPlace {
    fn named(name: &str) -> Self {
        match name.to_ascii_uppercase().as_str() {
            "PARTY" => Self::Party,
            "HOME" => Self::Home,
            _ => Self::Spot(name.to_owned()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScriptCondition {
    /// The party is within so many metres, in clear sight.
    PartyNear(f32),
    /// So many times in a hundred.
    Random(u32),
}

/// How a script says a group stands toward the party.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptStance {
    Neutral,
    Hostile,
    Friendly,
}

/// One command of a script.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptStep {
    /// Carry on from this step.
    Goto(usize),
    WalkTo {
        place: ScriptPlace,
        waits: bool,
    },
    Face(ScriptPlace),
    Teleport(ScriptPlace),
    /// Say a quote of its own lines, heard as it stands.
    Say {
        quote: Option<u16>,
        waits: bool,
    },
    /// Begin talking with the party, with a quote if one is given.
    Talk {
        quote: Option<u16>,
    },
    If {
        condition: Option<ScriptCondition>,
        negated: bool,
    },
    Else,
    EndIf,
    Disposition(ScriptStance),
    Trigger(String),
    /// Wait so many seconds.
    Delay(f32),
    /// An animation played once.
    Cycle {
        name: String,
        waits: bool,
    },
    /// A sound heard from where it stands: its file, and how loud of 127.
    Sound {
        path: String,
        volume: f32,
    },
    /// The party turns to look at it.
    LookHere,
    /// It is gone from the level.
    Vanish,
    /// One of the story's named happenings.
    DoAction(String),
    End,
}

/// How a monster keeps itself once its script has run to its orders.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StandingOrders {
    pub duty: Option<Duty>,
    /// The way it faces, as a compass yaw, east 0, counter-clockwise.
    pub facing: Option<f32>,
    pub faces_party: bool,
    /// It does not hear.
    pub deaf: bool,
    pub stays_home: bool,
    /// How often it looks about, and for how long.
    pub looks_about: Option<(u32, u32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Duty {
    Guard(ScriptPlace),
    /// Walk about its home: how far, and by how much that varies, metres.
    Patrol {
        distance: f32,
        variation: f32,
    },
    PointPatrol {
        places: Vec<ScriptPlace>,
        random: bool,
    },
}

/// A monster's script as its file gives it.
#[derive(Asset, TypePath, Clone, Debug, Default, PartialEq)]
pub struct MonsterScript {
    pub steps: Vec<ScriptStep>,
    /// Where each label is, by its name in capitals.
    pub labels: HashMap<String, usize>,
    /// The orders it ends with, if it has any.
    pub orders: Option<StandingOrders>,
}

const DIRECTIONS: [&str; 8] =
    ["EAST", "NORTHEAST", "NORTH", "NORTHWEST", "WEST", "SOUTHWEST", "SOUTH", "SOUTHEAST"];

impl MonsterScript {
    /// Read a script's text (retail `ProcessScript`'s commands). A line that
    /// is no command is passed over, as the original passes it over.
    pub fn parse(text: &str) -> Self {
        // Labels first, so a jump may go forward: where each would stand
        // among the commands before the orders.
        let lines: Vec<Vec<&str>> = text
            .lines()
            .map(|line| line.split_whitespace().collect::<Vec<_>>())
            .filter(|words| !words.is_empty())
            .collect();
        let command = |words: &[&str]| words[0].to_ascii_uppercase();
        let is_label = |words: &[&str]| words.len() == 1 && words[0].ends_with(':');
        let mut labels = HashMap::new();
        let mut at = 0;
        for words in &lines {
            if command(words) == "BEGINORDERS" {
                break;
            }
            if is_label(words) {
                labels.insert(command(words).trim_end_matches(':').to_owned(), at);
            } else if Self::step(words, &HashMap::new(), 0).is_some() {
                at += 1;
            }
        }
        let mut script = Self { labels, ..default() };
        let mut ordering = false;
        for words in &lines {
            let word = command(words);
            if ordering {
                let orders = script.orders.get_or_insert_default();
                if word == "ENDORDERS" {
                    break;
                }
                Self::order(orders, &word, &words[1..]);
            } else if word == "BEGINORDERS" {
                ordering = true;
                script.orders.get_or_insert_default();
            } else if !is_label(words)
                && let Some(step) = Self::step(words, &script.labels, script.steps.len())
            {
                script.steps.push(step);
            }
        }
        script
    }

    /// The command a line is, as the step it would be at `at`.
    fn step(words: &[&str], labels: &HashMap<String, usize>, at: usize) -> Option<ScriptStep> {
        let word = words[0].to_ascii_uppercase();
        let argument = words.get(1).copied();
        let place = || argument.map(ScriptPlace::named);
        let waits = !words.iter().any(|word| word.eq_ignore_ascii_case("NOBLOCK"));
        let quote = argument.and_then(|quote| quote.parse::<u16>().ok());
        Some(match word.as_str() {
            // A label that is nowhere goes nowhere: the step ends, and the
            // script carries on from the next line.
            "GOTO" => ScriptStep::Goto(
                argument
                    .and_then(|label| labels.get(&label.to_ascii_uppercase()))
                    .copied()
                    .unwrap_or(at + 1),
            ),
            "WALKTO" => ScriptStep::WalkTo { place: place()?, waits },
            "FACE" => ScriptStep::Face(place()?),
            "TELEPORT" => ScriptStep::Teleport(place()?),
            "SAY" => ScriptStep::Say { quote, waits },
            "NPCINTERACTION" => ScriptStep::Talk { quote },
            "IF" => {
                let negated = argument.is_some_and(|word| word.eq_ignore_ascii_case("NOT"));
                let asked = if negated { words.get(2).copied() } else { argument };
                ScriptStep::If { condition: asked.and_then(condition), negated }
            }
            "ELSE" => ScriptStep::Else,
            "ENDIF" => ScriptStep::EndIf,
            "DISPOSITION" => {
                ScriptStep::Disposition(match argument?.to_ascii_uppercase().as_str() {
                    "DISP_NEUTRAL" => ScriptStance::Neutral,
                    "DISP_HOSTILE" => ScriptStance::Hostile,
                    "DISP_FRIENDLY" => ScriptStance::Friendly,
                    _ => return None,
                })
            }
            "TRIGGER" => ScriptStep::Trigger(argument?.to_owned()),
            // Retail scales the number by a thousandth.
            "DELAY" => ScriptStep::Delay(argument?.parse::<f32>().ok()? / 1000.0),
            "CYCLE" => ScriptStep::Cycle { name: argument?.to_ascii_uppercase(), waits },
            "DIE" => ScriptStep::Cycle { name: "DIE".into(), waits: false },
            // Retail reads a volume, then how far it carries, then the file,
            // whose name may have spaces.
            "PLAY" => {
                let volume = argument?.parse::<f32>().ok()?;
                let path = words.get(3..)?.join(" ").replace('\\', "/").to_lowercase();
                ScriptStep::Sound { path, volume }
            }
            "LOOKHERE" => ScriptStep::LookHere,
            "DISAPPEAR" | "FADEOUT" => ScriptStep::Vanish,
            "DOACTION" => ScriptStep::DoAction(argument?.to_ascii_uppercase()),
            "END" => ScriptStep::End,
            _ => return None,
        })
    }

    fn order(orders: &mut StandingOrders, word: &str, arguments: &[&str]) {
        let number = |at: usize| arguments.get(at).and_then(|n| n.parse::<f32>().ok());
        match word {
            "GUARD" => {
                if let Some(place) = arguments.first() {
                    orders.duty = Some(Duty::Guard(ScriptPlace::named(place)));
                }
            }
            "PATROL" => {
                if let (Some(distance), Some(variation)) = (number(0), number(1)) {
                    orders.duty = Some(Duty::Patrol { distance, variation });
                }
            }
            "POINTPATROL" | "RANDOMPOINTPATROL" if !arguments.is_empty() => {
                let places = arguments.iter().map(|place| ScriptPlace::named(place)).collect();
                orders.duty = Some(Duty::PointPatrol { places, random: word != "POINTPATROL" });
            }
            "FACE" => match arguments.first().map(|way| way.to_ascii_uppercase()) {
                Some(way) if way == "PARTY" => orders.faces_party = true,
                Some(way) => {
                    if let Some(eighth) = DIRECTIONS.iter().position(|d| *d == way) {
                        orders.facing = Some(eighth as f32 * std::f32::consts::FRAC_PI_4);
                    }
                }
                None => {}
            },
            "TURNTOFACEPARTY" => orders.faces_party = true,
            "DEAF" => orders.deaf = true,
            "STAYHOME" => orders.stays_home = true,
            "LOOKABOUT" => {
                if let Some(often) = number(0) {
                    orders.looks_about = Some((often as u32, number(1).unwrap_or(0.0) as u32));
                }
            }
            _ => {}
        }
    }
}

fn condition(text: &str) -> Option<ScriptCondition> {
    let (name, rest) = text.split_once('(')?;
    let amount = rest.trim_end_matches(')').parse::<f32>().ok()?;
    match name.to_ascii_uppercase().as_str() {
        "PARTYNEAR" => Some(ScriptCondition::PartyNear(amount)),
        "RANDOM" => Some(ScriptCondition::Random(amount as u32)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A person's script as shipped: it looks for the party each step, talks
    /// when it is near, and then keeps to its orders.
    #[test]
    fn a_script_is_read_into_its_commands_and_its_orders() {
        let text = "START:\r\nFACE PARTY\r\nIF PARTYNEAR(12)\r\n\tNPCINTERACTION\r\n\tGOTO SAIDLINES\r\n\
            ENDIF\r\nGOTO START\r\nSAIDLINES:\r\nBEGINORDERS\r\nPATROL 5 30\r\nTURNTOFACEPARTY\r\nENDORDERS";
        let script = MonsterScript::parse(text);
        assert_eq!(
            script.steps,
            [
                ScriptStep::Face(ScriptPlace::Party),
                ScriptStep::If {
                    condition: Some(ScriptCondition::PartyNear(12.0)),
                    negated: false
                },
                ScriptStep::Talk { quote: None },
                ScriptStep::Goto(6),
                ScriptStep::EndIf,
                ScriptStep::Goto(0),
            ]
        );
        let orders = script.orders.unwrap();
        assert_eq!(orders.duty, Some(Duty::Patrol { distance: 5.0, variation: 30.0 }));
        assert!(orders.faces_party);
    }
}
