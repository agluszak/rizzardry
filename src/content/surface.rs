//! Complete surface behavior retained by every renderable content part.
use bevy::prelude::*;

#[derive(Clone, Debug)]
pub struct Surface {
    pub material: Handle<StandardMaterial>,
    pub ambient: Vec3,
    /// Retail glow is multiplied by the texture, not StandardMaterial emissive.
    pub emission: f32,
    pub animation: TextureFrames,
}

/// How an animated texture advances (retail `W8TextureAnimationMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Playback {
    #[default]
    Loop,
    PingPong,
    Once,
    /// Never advances by itself; driven by game state (e.g. the sky by time
    /// of day).
    Manual,
}

/// A material showing one frame of a frame list at a time.
#[derive(Component, Clone, Debug, Default)]
pub struct TextureFrames {
    pub frames: Vec<Handle<Image>>,
    pub playback: Playback,
    pub frames_per_second: f32,
    pub start_frame: usize,
}

impl TextureFrames {
    /// The frame shown `seconds` after the animation started (retail
    /// `Texture::UpdateFrame`).
    pub fn frame_at(&self, seconds: f32) -> usize {
        let count = self.frames.len();
        if count == 0 {
            return 0;
        }
        let steps = (seconds.max(0.0) * self.frames_per_second) as usize;
        let start = self.start_frame.min(count - 1);
        match self.playback {
            Playback::Manual => start,
            Playback::Loop => (start + steps) % count,
            Playback::Once => (start + steps).min(count - 1),
            Playback::PingPong => {
                let step = (start + steps) % count;
                if ((start + steps) / count).is_multiple_of(2) { step } else { count - 1 - step }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animation(playback: Playback) -> TextureFrames {
        TextureFrames {
            frames: vec![default(); 4],
            playback,
            frames_per_second: 2.0,
            start_frame: 0,
        }
    }

    #[test]
    fn frames_follow_the_retail_playback_modes() {
        let at = |p, s| animation(p).frame_at(s);
        assert_eq!([0.0, 0.5, 1.9, 2.0].map(|s| at(Playback::Loop, s)), [0, 1, 3, 0]);
        assert_eq!([2.0, 2.5, 3.5].map(|s| at(Playback::PingPong, s)), [3, 2, 0]);
        assert_eq!(at(Playback::Once, 10.0), 3);
        assert_eq!(at(Playback::Manual, 10.0), 0);
    }
}
