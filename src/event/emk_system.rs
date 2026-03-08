use eldenring::{
    Tree, cs::{BlockId, CSEzRabbitNoUpdateTask}, dlkr::DLAllocatorRef, fd4::{FD4ResCap, FD4ResCapHolder, FD4ResRep}
};
use fromsoftware_shared::{OwnedPtr, Program, Subclass, singleton};
use pelite::pe64::Pe;
use std::{
    mem::{MaybeUninit, transmute},
    ptr::{NonNull, null},
};

use crate::rva::{EMEVD_GROUP_SWITCH, EVENT_INS_CONSTRUCTOR, EVENT_INS_DESTRUCTOR};

type CSEmkEventInsCtor =
    extern "C" fn(*mut CSEmkEventIns, &EmkEventId, &[usize; 2], *const u8, u32, i32, i32);
type CSEmkEventInsDtor = extern "C" fn(*mut CSEmkEventIns, u32);
type EmkInstructionBanksExecute = extern "C" fn(*mut EmkInstructionBanks, f32, &CSEmkEventIns);

#[repr(C)]
pub struct EmkEventId {
    pub id: i32,
    pub slot: i32,
    pub compound_key: i32,
}

impl EmkEventId {
    pub fn new(id: i32, slot: i32) -> Self {
        Self {
            id,
            slot,
            compound_key: if slot < 0 { id } else { id + slot },
        }
    }
}

#[repr(C)]
pub struct CSEmkCondition {
    vftable: usize,
    unk8: usize,
    pub next: Option<OwnedPtr<CSEmkCondition>>,
    pub result: bool,
    _pad19: [u8; 0x7],
    unk20: usize,
}

#[repr(C)]
pub struct EventConditionSet {
    unk0: usize,
    unk8: usize,
    unk10: usize,
    pub head: Option<OwnedPtr<CSEmkCondition>>,
    pub tail: Option<OwnedPtr<CSEmkCondition>>,
}

#[repr(C)]
pub struct CSEventIns {
    vftable: usize,
    unk8: [u8; 0x38],
    pub conditions: EventConditionSet,
    unk68: [u8; 0x38],
}

#[repr(C)]
pub struct EmkInstruction {
    pub bank: u32,
    pub id: u32,
    pub arg_length: usize,
    pub arg_offset: isize,
    pub event_layer_offset: isize,
}

impl EmkInstruction {
    pub const fn new(bank: u32, id: u32) -> Self {
        Self {
            bank,
            id,
            arg_length: 0,
            arg_offset: -1,
            event_layer_offset: -1,
        }
    }

    pub const fn new_with_length(bank: u32, id: u32, arg_length: usize) -> Self {
        Self {
            bank,
            id,
            arg_length,
            arg_offset: -1,
            event_layer_offset: -1
        }
    }
}

#[repr(C)]
pub struct FakeVector {
    pub allocator: usize,
    pub start: usize,
    pub end: usize,
    pub capacity: usize,
}

impl FakeVector {
    pub fn new() -> Self { FakeVector { allocator: 0, start: 0, end: 0, capacity: 0 } }
}

#[repr(C)]
pub struct EmevdContainer {
    pub file_size: i32,
    pub unk4: i32,
    pub data: usize,
    pub offsets: Box<FakeVector>,
    pub unk18: [u8; 0x48],
}

// TODO: Don't need this
impl EmevdContainer {
    pub fn new() -> Self {
        EmevdContainer {
            file_size: 0,
            unk4: 0,
            data: 0,
            offsets: Box::new(FakeVector::new()),
            unk18: [0; 0x48],
        }
    }
}

#[repr(C)]
pub struct CSEmkEventIns {
    pub base: CSEventIns,
    pub emevd_container: *const EmevdContainer,
    unka8: [u8; 0x20],
    pub emevd_container2: *const EmevdContainer,
    pub next_instruction: *const EmkInstruction,
    pub next_instruction_args: *const u8,
    unke0: [u8; 0x150],
}

impl CSEmkEventIns {
    /**
     * Allocate a new event with the given ID, arguments, and map
     */
    pub fn new(id: EmkEventId, args_data: Option<&[u8]>, map_id: Option<BlockId>) -> Self {
        let ctor: CSEmkEventInsCtor =
            unsafe { transmute(Program::current().rva_to_va(EVENT_INS_CONSTRUCTOR).unwrap()) };

        let mut new: MaybeUninit<Self> = MaybeUninit::uninit();

        let container = [0usize; 2];

        ctor(
            new.as_mut_ptr(),
            &id,
            &container,
            args_data.map_or(null(), |data| data.as_ptr()),
            args_data.map_or(0, |data| data.len() as u32),
            map_id.unwrap_or(BlockId::none()).0,
            map_id.unwrap_or(BlockId::none()).0,
        );

        unsafe { new.assume_init() }
    }
}

impl Drop for CSEmkEventIns {
    /**
     * Call the destructor when an event is dropped. This frees up memory allocated by the game,
     * and unregisters a task that would cause the event to continue running otherwise
     */
    fn drop(&mut self) {
        let dtor: CSEmkEventInsDtor =
            unsafe { transmute(Program::current().rva_to_va(EVENT_INS_DESTRUCTOR).unwrap()) };

        dtor(self, 0);
    }
}

#[repr(C)]
pub struct EmkInstructionBanks {
    pub control_flow_system: usize,
    pub control_flow_timer: usize,
    unk10: usize,
    unk18: usize,
    pub control_flow_character: usize,
    pub control_flow_asset: usize,
    pub sfx: usize,
    pub message: usize,
    pub camera: usize,
    pub script: usize,
    pub sound: usize,
    pub hit: usize,
    pub map: usize,
    unk68: usize,
    unk70: usize,
    unk78: usize,
    unk80: usize,
}

impl EmkInstructionBanks {
    pub fn execute(&mut self, time: f32, event: &CSEmkEventIns) {
        let execute: EmkInstructionBanksExecute =
            unsafe { transmute(Program::current().rva_to_va(EMEVD_GROUP_SWITCH).unwrap()) };

        execute(self, time, event);
    }
}

#[repr(C)]
#[derive(Subclass)]
pub struct CSEmevdResCap {
    pub inner: FD4ResCap,
    pub container: OwnedPtr<EmevdContainer>,
}

#[repr(C)]
#[singleton("CSEmevdRepository")]
#[derive(Subclass)]
#[subclass(base = FD4ResRep, base = FD4ResCap)]
pub struct CSEmevdRepository {
    pub res_rep: FD4ResRep,
    pub res_cap_holder: FD4ResCapHolder<CSEmevdResCap>,
    allocator: usize,
}

impl CSEmevdRepository {
    pub fn get_rescap(&self, name: &str) -> Option<&CSEmevdResCap> {
        self.res_cap_holder
            .entries()
            .find(|e| e.inner.name.to_string() == name)
    }

    pub fn print_rescaps(&self) {
        self.res_cap_holder
            .entries()
            .for_each(|e| log::info!("{:?}", e.inner.name.to_string()));
    }
}


#[repr(C)]
#[singleton("CSEmkSystem")]
pub struct CSEmkSystem {
    pub event: Option<NonNull<CSEmkEventIns>>,
    unk8: usize,
    unk10: usize,
    unk18: usize,
    unk20: usize,
    pub instruction_banks: OwnedPtr<EmkInstructionBanks>,
    unk30: usize,
    unk38: usize,
    unk40: usize,
    unk48: i32,
    _pad4c: [u8; 4],
    pub unk50: CSEzRabbitNoUpdateTask,
    unk70: usize,
    unk78: usize,
    pub unk80: CSEzRabbitNoUpdateTask,
    unka0: usize,
    unka8: usize,
    pub unkb0: Tree<usize>,
    pub allocator2: DLAllocatorRef,
    unkd0: usize,
    unkd8: usize,
    unke0: usize,
    _pade8: [u8; 8],
    unkf0: usize,
    unkf8: i32,
    unkfc: i32,
    pub unk108: Tree<usize>,
    _pad118: [u8; 8],
    pub event2: Option<NonNull<CSEmkEventIns>>,
}