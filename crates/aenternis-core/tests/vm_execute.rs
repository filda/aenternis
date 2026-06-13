//! Integration tests for [`vm::execute_instruction`].
//!
//! Per-opcode coverage plus the cross-cutting behaviors:
//! modular addressing, PC wrap, unknown opcode, empty cell, and the
//! introspection invariant (sensors only see `neighbor_energies`,
//! never their own cell).

use aenternis_core::vm::execute_instruction;
use aenternis_core::{Arena, Cell, Direction, Opcode};

/// Build a cell whose memory starts with the given program slots.
/// `pc` defaults to 0; `origin_tag` defaults to 0.
fn cell_with(arena: &mut Arena, program: &[u32]) -> Cell {
    Cell::with_memory(arena, program)
}

/// Empty neighbor-energy snapshot — every direction is void.
const VOID: [u32; Direction::COUNT] = [0; Direction::COUNT];

/// Helper: encode an instruction's opcode byte at offset 0 of an
/// arbitrarily-large slot (upper bits zero).
const fn op(o: Opcode) -> u32 {
    o as u32
}

// ----- structural -----

#[test]
fn empty_cell_is_a_noop() {
    let mut arena = Arena::with_capacity(64);
    let mut c = Cell::new();
    execute_instruction(&mut c, &mut arena, &VOID);
    assert!(c.memory(&arena).is_empty());
    assert_eq!(c.pc, 0);
}

#[test]
fn high_byte_executes_folded_opcode() {
    let mut arena = Arena::with_capacity(64);
    // Low byte 0x20 = 32; 32 % 31 = 1 = Set. The byte is above the defined
    // range but `decode` is total, so it runs `set mem[4] = 0xAB`.
    let mut c = cell_with(&mut arena, &[0x20, 4, 0xAB, 0, 0]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[4], 0xAB);
    assert_eq!(c.pc, 3, "Set has length 3");
}

#[test]
fn pc_wraps_at_memory_boundary() {
    let mut arena = Arena::with_capacity(64);
    // 4-slot memory, PC at 3, opcode = Inc with arg at PC+1 (wraps to 0).
    // (Earlier draft tried `[0, Inc, 99, 99]` with PC=3 — the opcode slot
    // is then 99, which is unknown and falls back to nop-advance, defeating
    // the wrap exercise. The layout below puts Inc at PC=2 so the opcode
    // decodes and arg1 at PC+1=3 reads mem[3]=0, targeting memory[0].)
    let mut c = cell_with(&mut arena, &[op(Opcode::Inc), 1, op(Opcode::Inc), 0]);
    c.pc = 2;
    // PC=2, opcode = Inc, arg1 = mem[3] = 0 (target memory[0]).
    // mem[0] becomes wrapping_add(op(Opcode::Inc), 1) = 0x06.
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pc, (2 + 2) % 4); // = 0
    assert_eq!(c.memory(&arena)[0], op(Opcode::Inc).wrapping_add(1));
}

// ----- nop -----

#[test]
fn nop_advances_pc_by_one() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Nop), 0, 0]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pc, 1);
    assert_eq!(c.memory(&arena), &[op(Opcode::Nop), 0, 0][..]);
}

// ----- set / copy -----

#[test]
fn set_writes_value_to_address() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Set), 4, 0xDEAD_BEEF, 0, 0]); // size 5
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[4], 0xDEAD_BEEF);
    assert_eq!(c.pc, 3);
}

#[test]
fn set_address_is_modular() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Set), 5, 42]); // memory size 3
    execute_instruction(&mut c, &mut arena, &VOID);
    // 5 % 3 = 2.
    assert_eq!(c.memory(&arena)[2], 42);
}

#[test]
fn copy_moves_value_from_b_to_a() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Copy), 2, 3, 100, 200]);
    execute_instruction(&mut c, &mut arena, &VOID);
    // mem[2] = mem[3] = 100
    assert_eq!(c.memory(&arena)[2], 100);
}

// ----- arithmetic -----

#[test]
fn add_wraps_modulo_2_to_32() {
    let mut arena = Arena::with_capacity(64);
    // Operands are addresses (3 and 4); the values being added live at
    // those addresses. mem[3] = u32::MAX, mem[4] = 5.
    let mut c = cell_with(&mut arena, &[op(Opcode::Add), 3, 4, u32::MAX, 5]);
    execute_instruction(&mut c, &mut arena, &VOID);
    // mem[3] = mem[3].wrapping_add(mem[4]) = u32::MAX + 5 = 4.
    assert_eq!(c.memory(&arena)[3], 4);
}

#[test]
fn sub_wraps_modulo_2_to_32() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Sub), 3, 4, 3, 10]);
    execute_instruction(&mut c, &mut arena, &VOID);
    // mem[3] = mem[3].wrapping_sub(mem[4]) = 3 - 10 wraps.
    assert_eq!(c.memory(&arena)[3], 3u32.wrapping_sub(10));
}

#[test]
fn inc_advances_value_by_one() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Inc), 2, 41]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[2], 42);
    assert_eq!(c.pc, 2);
}

#[test]
fn inc_wraps_at_u32_max() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Inc), 2, u32::MAX]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[2], 0);
}

#[test]
fn dec_decreases_value_by_one() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Dec), 2, 1]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[2], 0);
}

#[test]
fn dec_wraps_at_zero() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Dec), 2, 0]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[2], u32::MAX);
}

// ----- control flow -----

#[test]
fn jmp_sets_pc_modularly() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Jmp), 7, 0, 0, 0]); // size 5
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pc, 7 % 5);
}

#[test]
fn jz_takes_branch_when_value_is_zero() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Jz), 3, 4, 0, 0]); // mem[3]=0
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pc, 4);
}

#[test]
fn jz_falls_through_when_value_is_nonzero() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Jz), 3, 4, 99, 0]); // mem[3]=99
    execute_instruction(&mut c, &mut arena, &VOID);
    // Length 3, fall through to PC = 3.
    assert_eq!(c.pc, 3);
}

#[test]
fn jne_takes_branch_when_value_is_nonzero() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Jne), 3, 4, 99, 0]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pc, 4);
}

#[test]
fn jne_falls_through_when_value_is_zero() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Jne), 3, 4, 0, 0]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pc, 3);
}

#[test]
fn je_takes_branch_when_values_equal() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Je), 4, 5, 6, 7, 7, 0]);
    execute_instruction(&mut c, &mut arena, &VOID);
    // mem[4]=7, mem[5]=7 → equal → PC = mem[6] mod size = 6 mod 7 = 6.
    assert_eq!(c.pc, 6);
}

#[test]
fn je_falls_through_when_values_differ() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Je), 4, 5, 6, 7, 8, 0]);
    execute_instruction(&mut c, &mut arena, &VOID);
    // Length 4, fall through to PC = 4.
    assert_eq!(c.pc, 4);
}

// ----- pointer ops -----

#[test]
fn setp_writes_pointer_and_marks_override() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Setp), 2, 4, 0, 0, 0, 0, 0]); // size 8
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pointers[Direction::Yp.index()], 4);
    assert!(c.pointer_override[Direction::Yp.index()]);
    assert_eq!(c.pc, 3);
}

#[test]
fn setp_value_is_clamped_modulo_memory_size() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Setp), 0, 99, 0, 0]); // size 5
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pointers[Direction::Xp.index()], 99 % 5);
}

#[test]
fn getp_reads_pointer_into_memory() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Getp), 1, 3, 0]);
    c.pointers[Direction::Xn.index()] = 42;
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[3], 42);
}

#[test]
fn setpv_uses_runtime_value_from_memory() {
    let mut arena = Arena::with_capacity(64);
    // memory layout: [Setpv, 0, 4, 0, 17, 0]
    //   PC=0 → opcode=Setpv, arg1=mem[1]=0 (Xp), arg2=mem[2]=4 (address).
    //   pointers[Xp] = mem[4] mod size = 17 mod 6.
    let mut c = cell_with(&mut arena, &[op(Opcode::Setpv), 0, 4, 0, 17, 0]); // size 6
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pointers[Direction::Xp.index()], 17 % 6);
    assert!(c.pointer_override[Direction::Xp.index()]);
}

// ----- emission -----

#[test]
fn port_accumulates_active_outflow() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Port), 1, 5, 0]);
    c.active_outflow[Direction::Xn.index()] = 3;
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.active_outflow[Direction::Xn.index()], 8);
}

// ----- sensors -----

#[test]
fn senergy_reads_neighbor_energy_into_memory() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Senergy), 2, 3, 0]);
    let mut neighbors = VOID;
    neighbors[Direction::Yp.index()] = 99;
    execute_instruction(&mut c, &mut arena, &neighbors);
    assert_eq!(c.memory(&arena)[3], 99);
}

#[test]
fn senergy_returns_zero_for_void_neighbor() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Senergy), 0, 3, 0]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[3], 0);
}

// ----- indirect addressing -----

#[test]
fn ldi_loads_indirect_via_runtime_address() {
    let mut arena = Arena::with_capacity(64);
    // mem[3] = 5. We want mem[2] = mem[mem[3]] = mem[5] = 7.
    let mut c = cell_with(&mut arena, &[op(Opcode::Ldi), 2, 3, 5, 0, 7, 0]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[2], 7);
}

#[test]
fn sti_stores_indirect_via_runtime_address() {
    let mut arena = Arena::with_capacity(64);
    // mem[3] = 5 (target). mem[4] = 99 (value).
    // sti a=3 b=4 → mem[mem[3]] = mem[4] → mem[5] = 99.
    let mut c = cell_with(&mut arena, &[op(Opcode::Sti), 3, 4, 5, 99, 0, 0]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[5], 99);
}

// ----- UI fields -----

#[test]
fn sid_writes_origin_tag_into_memory() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Sid), 1, 0]);
    c.origin_tag = 0xCAFE_BABE;
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[1], 0xCAFE_BABE);
}

#[test]
fn paint_writes_appearance() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Paint), 0xFF00_FF00, 0]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.appearance, 0xFF00_FF00);
}

// ----- introspection invariant -----

#[test]
fn senergy_can_only_read_via_neighbor_energies() {
    let mut arena = Arena::with_capacity(64);
    // The function signature itself enforces the invariant: there is
    // no way for `execute_instruction` to look at another cell. This
    // test simply documents that with a sensor read against a custom
    // neighbor table that doesn't match any real cell.
    let mut c = cell_with(&mut arena, &[op(Opcode::Senergy), 4, 1, 0]);
    let mut neighbors = VOID;
    neighbors[Direction::Zp.index()] = 12345;
    execute_instruction(&mut c, &mut arena, &neighbors);
    // 4 mod 6 = 4 = Zp.index().
    assert_eq!(c.memory(&arena)[1], 12345);
}

// ----- direction modulo (d mod DIRS) for opcodes with a `d` operand -----

#[test]
fn direction_operand_wraps_modulo_six() {
    let mut arena = Arena::with_capacity(64);
    // d=8 → 8 mod 6 = 2 = Yp.
    let mut c = cell_with(&mut arena, &[op(Opcode::Setp), 8, 3, 0, 0, 0]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert!(c.pointer_override[Direction::Yp.index()]);
    assert!(!c.pointer_override[Direction::Xp.index()]);
}

// ----- port wraps on accumulation -------------------------------------------

#[test]
fn port_uses_wrapping_add_on_active_outflow() {
    let mut arena = Arena::with_capacity(64);
    // `port` uses `wrapping_add` on `active_outflow` — the 32-bit wrap
    // is intentional, so a rogue program can't crash the VM by driving
    // a face's outflow past `u32::MAX`. Pre-load `active_outflow[Xp]`
    // near the top of `u32` so a small `port` increment wraps.
    let mut c = cell_with(&mut arena, &[op(Opcode::Port), 0, 100, 0]);
    c.active_outflow[Direction::Xp.index()] = u32::MAX - 50;
    execute_instruction(&mut c, &mut arena, &VOID);
    // wrapping: (u32::MAX - 50) + 100 = 49.
    assert_eq!(c.active_outflow[Direction::Xp.index()], 49);
}

#[test]
fn byte_folding_to_nop_advances_pc_by_one() {
    let mut arena = Arena::with_capacity(64);
    // 0x1F = 31; 31 % 31 = 0 = Nop. The first byte past the defined range
    // happens to fold back onto `nop`, advancing PC by 1 and touching nothing.
    let mut c = cell_with(&mut arena, &[0; 10]);
    c.set_memory_slot(&mut arena, 5, 0x1F);
    c.set_memory_slot(&mut arena, 6, 0);
    c.set_memory_slot(&mut arena, 7, 9);
    c.pc = 5;
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pc, 6);
    assert_eq!(c.memory(&arena)[9], 0);
}

// ----- bitwise -----

#[test]
fn and_masks_destination_with_source() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::And), 3, 4, 0b1100, 0b1010]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[3], 0b1000);
    assert_eq!(c.pc, 3);
}

#[test]
fn or_combines_destination_with_source() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Or), 3, 4, 0b1100, 0b1010]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[3], 0b1110);
}

#[test]
fn xor_toggles_bits() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Xor), 3, 4, 0b1100, 0b1010]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[3], 0b0110);
}

#[test]
fn not_complements_all_bits() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Not), 2, 0x0F]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[2], 0xFFFF_FFF0);
    assert_eq!(c.pc, 2);
}

// ----- shifts -----

#[test]
fn shl_shifts_left_by_source() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Shl), 3, 4, 1, 4]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[3], 16);
}

#[test]
fn shl_masks_shift_amount_modulo_32() {
    let mut arena = Arena::with_capacity(64);
    // 36 mod 32 = 4 → 1 << 4 = 16. Must not panic on >= 32.
    let mut c = cell_with(&mut arena, &[op(Opcode::Shl), 3, 4, 1, 36]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[3], 16);
}

#[test]
fn shr_shifts_right_logically() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Shr), 3, 4, 0x8000_0000, 3]);
    execute_instruction(&mut c, &mut arena, &VOID);
    // Logical (unsigned) — high bit does not sign-extend.
    assert_eq!(c.memory(&arena)[3], 0x1000_0000);
}

#[test]
fn shr_masks_shift_amount_modulo_32() {
    let mut arena = Arena::with_capacity(64);
    // 35 mod 32 = 3.
    let mut c = cell_with(&mut arena, &[op(Opcode::Shr), 3, 4, 0x80, 35]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[3], 0x10);
}

// ----- mul / div / mod -----

#[test]
fn mul_wraps_modulo_2_to_32() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Mul), 3, 4, 0x8000_0000, 2]);
    execute_instruction(&mut c, &mut arena, &VOID);
    // 0x8000_0000 * 2 = 0x1_0000_0000 wraps to 0.
    assert_eq!(c.memory(&arena)[3], 0);
}

#[test]
fn div_truncates_toward_zero() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Div), 3, 4, 23, 5]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[3], 4);
}

#[test]
fn div_by_zero_yields_zero() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Div), 3, 4, 23, 0]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[3], 0);
}

#[test]
fn mod_returns_remainder() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Mod), 3, 4, 23, 5]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[3], 3);
}

#[test]
fn mod_by_zero_yields_zero() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Mod), 3, 4, 23, 0]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.memory(&arena)[3], 0);
}

// ----- signed conditional jumps -----

#[test]
fn jp_branches_on_positive() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Jp), 3, 4, 5, 0, 0]); // size 6, mem[3]=5
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pc, 4);
}

#[test]
fn jp_falls_through_on_zero_and_negative() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Jp), 3, 4, 0, 0, 0]); // mem[3]=0
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pc, 3, "zero is not positive");

    let mut c = cell_with(&mut arena, &[op(Opcode::Jp), 3, 4, u32::MAX, 0, 0]); // -1
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pc, 3, "negative is not positive");
}

#[test]
fn jn_branches_on_negative() {
    let mut arena = Arena::with_capacity(64);
    // mem[3] = 0xFFFF_FFFF = -1 as i32.
    let mut c = cell_with(&mut arena, &[op(Opcode::Jn), 3, 4, u32::MAX, 0, 0]);
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pc, 4);
}

#[test]
fn jn_falls_through_on_zero_and_positive() {
    let mut arena = Arena::with_capacity(64);
    let mut c = cell_with(&mut arena, &[op(Opcode::Jn), 3, 4, 0, 0, 0]); // mem[3]=0
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pc, 3, "zero is not negative");

    let mut c = cell_with(&mut arena, &[op(Opcode::Jn), 3, 4, 7, 0, 0]); // positive
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pc, 3, "positive is not negative");
}

#[test]
fn decode_uses_low_byte_only() {
    let mut arena = Arena::with_capacity(64);
    // Slot 0x0000_0105 decodes to Inc (low byte 0x05) — upper bits
    // never change which opcode runs.
    let mut c = cell_with(&mut arena, &[0; 10]);
    c.set_memory_slot(&mut arena, 5, 0x0000_0105);
    c.set_memory_slot(&mut arena, 6, 9);
    c.pc = 5;
    execute_instruction(&mut c, &mut arena, &VOID);
    assert_eq!(c.pc, 7, "Inc has length 2");
    assert_eq!(c.memory(&arena)[9], 1);
}
