//! Audio for the Underground 2 reimplementation. Independent of rendering so it can be
//! tested on any host; platforms pull mixed frames from [`sound::Mixer::render`].

pub mod music;
pub mod sound;
