//! Import the original UTF-16 credits as display lines.
use crate::content::ui::{CreditLine, Credits};
use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::reflect::TypePath;
use std::io::{Error, ErrorKind};

#[derive(Default, TypePath)]
pub struct CreditsLoader;

impl AssetLoader for CreditsLoader {
    type Asset = Credits;
    type Settings = ();
    type Error = Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<Credits, Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        decode(&bytes)
    }
    fn extensions(&self) -> &[&str] {
        &["txt"]
    }
}

fn decode(bytes: &[u8]) -> Result<Credits, Error> {
    if !bytes.starts_with(&[0xff, 0xfe]) || !bytes.len().is_multiple_of(2) {
        return Err(Error::new(ErrorKind::InvalidData, "credits require UTF-16LE with a BOM"));
    }
    let units: Vec<_> =
        bytes[2..].as_chunks::<2>().0.iter().copied().map(u16::from_le_bytes).collect();
    let text =
        String::from_utf16(&units).map_err(|error| Error::new(ErrorKind::InvalidData, error))?;
    let lines = text
        .lines()
        .filter(|line| !line.starts_with('*'))
        .map(|line| {
            if let Some(heading) = line.strip_prefix('!') {
                CreditLine::Heading(heading.to_owned())
            } else if let Some((left, right)) = line.split_once('&') {
                CreditLine::Pair(left.trim_end().to_owned(), right.trim_start().to_owned())
            } else if line.is_empty() {
                CreditLine::Blank
            } else {
                CreditLine::Centred(line.to_owned())
            }
        })
        .collect();
    Ok(Credits(lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credits_keep_headings_pairs_and_spacing() {
        let text = "!Team\r\nA & B\r\n\r\n*ignored\r\nC\r\n";
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        let lines = decode(&bytes).unwrap().0;
        assert!(
            matches!(&lines[..], [CreditLine::Heading(a), CreditLine::Pair(b, c), CreditLine::Blank, CreditLine::Centred(d)] if (a.as_str(), b.as_str(), c.as_str(), d.as_str()) == ("Team", "A", "B", "C"))
        );
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
    }
}
