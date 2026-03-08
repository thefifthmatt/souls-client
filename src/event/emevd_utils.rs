use eldenring::cs::CSWorldGeomMan;
use fromsoftware_shared::FromStatic;

use crate::event::emk_system::{CSEmkEventIns, CSEmkSystem, EmkEventId, EmkInstruction, CSEmevdRepository};

const DELTA_TIME: f32 = 1f32 / 30f32;

/**
 * Execute a single ad hoc EMEVD instruction in a temporary event, returning the condition result,
 * if any
 *
 * Safety: args must be the type expected by the instruction
 */
pub unsafe fn execute_emevd_instruction(
    instruction: EmkInstruction,
    args: *const u8,
) -> Option<bool> {
    // Make sure the world is loaded to avoid panic in EMEVD instructions
    if unsafe { !CSWorldGeomMan::instance().is_ok() } {
        return None;
    }

    let cs_emk_system = unsafe { CSEmkSystem::instance() }.ok()?;

    // Construct a new event for this instruction, which is immediately destroyed afterwards
    // For init common func, the area id needs to be between 50 and 89 and not 60 or 61 to use the event flag as-is
    let mut event = CSEmkEventIns::new(
        EmkEventId::new(0, 0), None, None);
    event.next_instruction = &instruction;
    event.next_instruction_args = args;

    // Needed for event init
    if instruction.arg_length > 0 {
        let repository = unsafe { CSEmevdRepository::instance() }.ok()?;
        if let Some(common_func) = repository.get_rescap("common_func") {
            event.emevd_container = common_func.container.as_ptr();
            event.emevd_container2 = common_func.container.as_ptr();
        } else {
            // Event will crash if it continues
            return Option::None;
        }
    }

    cs_emk_system.instruction_banks.execute(DELTA_TIME, &event);

    let condition = event.base.conditions.head.as_ref();
    condition.map(|condition| condition.result)
}

// Not actually Lua here, this just creates a regular function
#[macro_export]
macro_rules! lua_emevd_commands {
    (
        $(struct $struct_name:ident($bank:literal, $id:literal) {
            $($arg_name:ident: $arg_ty:ty = $arg_default:expr),* $(,)?
        })*
    ) => {
        paste::paste! {
            $(#[repr(C)]
            struct $struct_name {
                $($arg_name: $arg_ty,)*
            })*

            $(pub fn [<$struct_name:snake>] ( $($arg_name: $arg_ty,)* ) -> Option<bool> {
                let instruction = $crate::event::emk_system::EmkInstruction::new($bank, $id);
                let args = $struct_name {
                    $($arg_name: $arg_name,)*
                };
                unsafe {
                    let args: *const u8 = &args as *const _ as *const u8;
                    $crate::event::emevd_utils::execute_emevd_instruction(
                        instruction,
                        args
                    )
                }
            }
            )*
        }
    };
}
