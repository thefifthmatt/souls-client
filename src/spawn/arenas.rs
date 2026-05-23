use std::sync::LazyLock;
use eldenring::{cs::BlockId, position::BlockPosition};
use crate::spawn::Arena;

pub static MAIN_ARENAS: LazyLock<Vec<Arena>> = LazyLock::new(||
     vec![
        Arena::new("Placidusax", BlockId::from_parts(13, 0, 0, 0), BlockPosition::from_xyz(14.938, 1009.611, 330.489), 14.5),
        Arena::new("Astel", BlockId::from_parts(12, 4, 0, 0), BlockPosition::from_xyz(-96.953, -106.137, -130.124), 20.0),
        Arena::new("Fortissax (Torrent enabled)", BlockId::from_parts(12, 3, 0, 0), BlockPosition::from_xyz(-403.278, 149.344, -253.918), 20.0),
        Arena::new("Morgott", BlockId::from_parts(11, 0, 0, 0), BlockPosition::from_xyz(37.268, 64.963, -416.036), 17.0),
        Arena::new("Mimic Tear", BlockId::from_parts(12, 2, 0, 0), BlockPosition::from_xyz(1007.655, -617.556, 1139.351), 9.0),
    ]
);

