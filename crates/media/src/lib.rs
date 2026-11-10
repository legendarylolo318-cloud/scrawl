//! Media: images, PDF import, lecture audio, function plotter. Heavy parts are lazy: nothing
//! here allocates or spawns threads until used.

pub mod audio;
pub mod images;
pub mod pdf;
pub mod plot;
