//! Retail text: the game's string table (`data/strings/stringdata.dat`) and
//! per-level message files (`data/messages/<level>.msg`).
//!
//! After Whizzardry `ReadStringTable` and `GetStringFromStringDatabase`.
//! Text is UTF-16LE, each unit stored as `!unit + 0x9697`.

use super::pvl::{PvlError, invalid};
use crate::bytes::Bytes;

fn decode(units: &[u8], encoded: bool) -> String {
    let units: Vec<u16> = units
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .map(|unit| if encoded { (!unit).wrapping_add(0x9697) } else { unit })
        .take_while(|&unit| unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// The game's strings, by index.
pub fn read_string_table(data: &[u8]) -> Result<Vec<String>, PvlError> {
    let mut bytes = Bytes::new(data);
    let count = bytes.i32()?;
    if !(0..=100_000).contains(&count) {
        return Err(invalid("string table count out of range", &bytes));
    }
    (0..count)
        .map(|_| {
            let length = bytes.i32()?;
            if length < 0 || length % 2 != 0 {
                return Err(invalid("string length out of range", &bytes));
            }
            Ok(decode(bytes.take(length as usize)?, true))
        })
        .collect()
}

/// A message file's messages, by id; `None` where an id has no message.
/// The whole file is `None` when it is malformed.
pub fn read_messages(data: &[u8]) -> Option<Vec<Option<String>>> {
    let encoded = *data.get(4)? != 0;
    let word = |at: usize| -> Option<i32> {
        Some(i32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
    };
    let count = usize::try_from(word(data.len().checked_sub(8)?)?).ok()?;
    let table = data.len().checked_sub(8 + count * 4)?;
    Some(
        (0..count)
            .map(|index| {
                let offset = usize::try_from(word(table + index * 4)?).ok()?;
                if offset < 5 {
                    return None;
                }
                // Two metadata words, then the length in units.
                let length = usize::try_from(word(offset + 8)?).ok()?;
                let text = data.get(offset + 12..offset + 12 + length * 2)?;
                Some(decode(text, encoded))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_stored_complemented() {
        let unit = |c: char| (!((c as u16).wrapping_sub(0x9697))).to_le_bytes();
        let bytes: Vec<u8> = "Hi\0".chars().flat_map(unit).collect();
        assert_eq!(decode(&bytes, true), "Hi");
    }

    #[test]
    #[ignore = "requires a retail installation"]
    fn retail_strings_and_messages_decode() {
        let root =
            std::env::var("RIZZARDRY_RETAIL").expect("set RIZZARDRY_RETAIL to run this test");
        let files = crate::retail::files::RetailFiles::open(root).unwrap();
        let strings =
            read_string_table(&files.read("data/strings/stringdata.dat").unwrap().unwrap())
                .unwrap();
        println!("0x964: {:?}", strings[0x964]);
        println!("0x96b: {:?}", strings[0x96b]);
        assert!(strings[0x964].to_lowercase().contains("nothing"), "{:?}", strings[0x964]);
        let messages =
            read_messages(&files.read("data/messages/monastery.msg").unwrap().unwrap()).unwrap();
        for id in [15, 17, 36] {
            println!("{id}: {:?}", messages[id]);
            assert!(messages[id].as_ref().is_some_and(|m| !m.is_empty()));
        }
    }
}
