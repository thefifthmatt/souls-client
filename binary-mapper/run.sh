set -e
# TODO: Switch to rva bundles I guess
cargo.exe run --manifest-path=fromsoftware-rs/Cargo.toml --bin binary-mapper -- map --profile binary-mapper/ds3.toml --exe 'C:\Program Files (x86)\Steam\steamapps\common\DARK SOULS III\Game\DarkSoulsIII.exe' --output rust | grep 0x | sed -e 's/^\(.*\): \(.*\),/pub const \1: u32 = \2;/' | tee src/rva/ds3.rs
# exe is quite encrypted, do it manually for now
# cargo.exe run --manifest-path=fromsoftware-rs/Cargo.toml --bin binary-mapper -- map --profile binary-mapper/sekiro.toml --exe 'C:\Program Files (x86)\Steam\steamapps\common\Sekiro\sekiro.exe' --output rust | grep 0x | sed -e 's/^\(.*\): \(.*\),/pub const \1: u32 = \2;/' | tee src/rva/sekiro.rs
cargo.exe run --manifest-path=fromsoftware-rs/Cargo.toml --bin binary-mapper -- map --profile binary-mapper/ds1r.toml --exe 'C:\Program Files (x86)\Steam\steamapps\common\DARK SOULS REMASTERED\DarkSoulsRemastered.exe' --output rust | grep 0x | sed -e 's/^\(.*\): \(.*\),/pub const \1: u32 = \2;/' | tee src/rva/ds1r.rs
cargo.exe run --manifest-path=fromsoftware-rs/Cargo.toml --bin binary-mapper -- map --profile binary-mapper/eldenring.toml --exe 'C:/Program Files (x86)/Steam/steamapps/common/ELDEN RING/Game/eldenring.exe' --output rust | grep 0x | sed -e 's/^\(.*\): \(.*\),/pub const \1: u32 = \2;/' | tee src/rva/eldenring.rs
