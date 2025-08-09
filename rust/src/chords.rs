use godot::prelude::{GodotConvert, Var};
use serde::{Deserialize, Serialize};
use strum::EnumIter;

#[derive(Debug, Default, Clone, Copy, EnumIter, GodotConvert, Var, Serialize, Deserialize)]
#[godot(via = i64)]
pub enum Chord {
    #[default]
    Cmaj,
    Cmin,
    Caug, // Kinda dissonant
    C7,
    Cmaj7,
    Cmin7,
    Chalfdim7,
    CminMaj7, // Kinda dissonant
    C9,
    Cmaj9,
    Cmin9,
    C11,
    C13,
}

impl Chord {
    pub fn as_intervals(&self) -> Vec<u8> {
        match self {
            Chord::Cmaj => vec![0, 4, 7],
            Chord::Cmin => vec![0, 3, 7],
            Chord::Caug => vec![0, 4, 8],
            Chord::C7 => vec![0, 4, 7, 10],
            Chord::Cmaj7 => vec![0, 4, 7, 11],
            Chord::Cmin7 => vec![0, 3, 7, 10],
            Chord::Chalfdim7 => vec![0, 3, 6, 10],
            Chord::CminMaj7 => vec![0, 3, 7, 11],
            Chord::C9 => vec![0, 4, 7, 10, 14],
            Chord::Cmaj9 => vec![0, 4, 7, 11, 14],
            Chord::Cmin9 => vec![0, 3, 7, 10, 14],
            Chord::C11 => vec![0, 4, 7, 10, 14, 17], // Warning - 4 clashes with 17 (17 - 12 = 5)
            Chord::C13 => vec![0, 4, 7, 10, 14, 17, 21], // Warning - 4 clashes with 17 (17 - 12 = 5)
        }
    }
}
