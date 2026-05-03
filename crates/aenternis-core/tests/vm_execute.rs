//! Integration tests for [`vm::execute_instruction`].
//!
//! Per-opcode coverage plus the cross-cutting behaviors:
//! modular addressing, PC wrap, unknown opcode, empty cell, and the
//! introspection invariant (sensors only see `neighbor_energies`,
//! never their own cell).

use aenternis_core::vm::execute_instruction;
use aenternis_core::{Cell, Direction, Opcode};

/// Build a cell whose memory starts with the given program slots.
/// `pc` defaults to 0; `origin_tag` defaults to 0.
fn cell_with(program: &[u32]) -> Cell {
    Cell::with_memory(program.to_vec())
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
    let mut c = Cell::new();
    execute_instruction(&mut c, &VOID);
    assert!(c.memory.is_empty());
    assert_eq!(c.pc, 0);
}

#[test]
fn unknown_opcode_advances_pc_by_one() {
    // 0xFF is well above `Opcode::MAX`. Decode → None → nop semantics.
    let mut c = cell_with(&[0xFF, 99, 99]);
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.pc, 1);
    // Memory untouched.
    assert_eq!(c.memory, vec![0xFF, 99, 99]);
}

#[test]
fn pc_wraps_at_memory_boundary() {
    // 4-slot memory, PC at 3, opcode = inc with arg at PC+1 (wraps to 0).
    // length=2, after execute PC = (3+2) % 4 = 1.
    let mut c = cell_with(&[0, op(Opcode::Inc), 99, 99]);
    c.pc = 3;
    // mem[3] = 99, but with PC=3 the opcode slot is 99 (low byte 0x63 — undefined).
    // That makes this test about unknown-opcode wrap, not inc wrap. Rebuild:
    let mut c = cell_with(&[op(Opcode::Inc), 1, op(Opcode::Inc), 0]);
    c.pc = 2;
    // PC=2, opcode = Inc, arg1 = mem[3] = 0 (target memory[0]).
    // mem[0] becomes wrapping_add(op(Opcode::Inc), 1) = 0x06.
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.pc, (2 + 2) % 4); // = 0
    assert_eq!(c.memory[0], op(Opcode::Inc).wrapping_add(1));
}

// ----- nop -----

#[test]
fn nop_advances_pc_by_one() {
    let mut c = cell_with(&[op(Opcode::Nop), 0, 0]);
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.pc, 1);
    assert_eq!(c.memory, vec![op(Opcode::Nop), 0, 0]);
}

// ----- set / copy -----

#[test]
fn set_writes_value_to_address() {
    let mut c = cell_with(&[op(Opcode::Set), 4, 0xDEAD_BEEF, 0, 0]); // size 5
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.memory[4], 0xDEAD_BEEF);
    assert_eq!(c.pc, 3);
}

#[test]
fn set_address_is_modular() {
    let mut c = cell_with(&[op(Opcode::Set), 5, 42]); // memory size 3
    execute_instruction(&mut c, &VOID);
    // 5 % 3 = 2.
    assert_eq!(c.memory[2], 42);
}

#[test]
fn copy_moves_value_from_b_to_a() {
    let mut c = cell_with(&[op(Opcode::Copy), 2, 3, 100, 200]);
    execute_instruction(&mut c, &VOID);
    // mem[2] = mem[3] = 100
    assert_eq!(c.memory[2], 100);
}

// ----- arithmetic -----

#[test]
fn add_wraps_modulo_2_to_32() {
    // Operands are addresses (3 and 4); the values being added live at
    // those addresses. mem[3] = u32::MAX, mem[4] = 5.
    let mut c = cell_with(&[op(Opcode::Add), 3, 4, u32::MAX, 5]);
    execute_instruction(&mut c, &VOID);
    // mem[3] = mem[3].wrapping_add(mem[4]) = u32::MAX + 5 = 4.
    assert_eq!(c.memory[3], 4);
}

#[test]
fn sub_wraps_modulo_2_to_32() {
    let mut c = cell_with(&[op(Opcode::Sub), 3, 4, 3, 10]);
    execute_instruction(&mut c, &VOID);
    // mem[3] = mem[3].wrapping_sub(mem[4]) = 3 - 10 wraps.
    assert_eq!(c.memory[3], 3u32.wrapping_sub(10));
}

#[test]
fn inc_advances_value_by_one() {
    let mut c = cell_with(&[op(Opcode::Inc), 2, 41]);
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.memory[2], 42);
    assert_eq!(c.pc, 2);
}

#[test]
fn inc_wraps_at_u32_max() {
    let mut c = cell_with(&[op(Opcode::Inc), 2, u32::MAX]);
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.memory[2], 0);
}

#[test]
fn dec_decreases_value_by_one() {
    let mut c = cell_with(&[op(Opcode::Dec), 2, 1]);
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.memory[2], 0);
}

#[test]
fn dec_wraps_at_zero() {
    let mut c = cell_with(&[op(Opcode::Dec), 2, 0]);
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.memory[2], u32::MAX);
}

// ----- control flow -----

#[test]
fn jmp_sets_pc_modularly() {
    let mut c = cell_with(&[op(Opcode::Jmp), 7, 0, 0, 0]); // size 5
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.pc, 7 % 5);
}

#[test]
fn jz_takes_branch_when_value_is_zero() {
    let mut c = cell_with(&[op(Opcode::Jz), 3, 4, 0, 0]); // mem[3]=0
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.pc, 4);
}

#[test]
fn jz_falls_through_when_value_is_nonzero() {
    let mut c = cell_with(&[op(Opcode::Jz), 3, 4, 99, 0]); // mem[3]=99
    execute_instruction(&mut c, &VOID);
    // Length 3, fall through to PC = 3.
    assert_eq!(c.pc, 3);
}

#[test]
fn jne_takes_branch_when_value_is_nonzero() {
    let mut c = cell_with(&[op(Opcode::Jne), 3, 4, 99, 0]);
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.pc, 4);
}

#[test]
fn jne_falls_through_when_value_is_zero() {
    let mut c = cell_with(&[op(Opcode::Jne), 3, 4, 0, 0]);
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.pc, 3);
}

#[test]
fn je_takes_branch_when_values_equal() {
    let mut c = cell_with(&[op(Opcode::Je), 4, 5, 6, 7, 7, 0]);
    execute_instruction(&mut c, &VOID);
    // mem[4]=7, mem[5]=7 → equal → PC = mem[6] mod size = 6 mod 7 = 6.
    assert_eq!(c.pc, 6);
}

#[test]
fn je_falls_through_when_values_differ() {
    let mut c = cell_with(&[op(Opcode::Je), 4, 5, 6, 7, 8, 0]);
    execute_instruction(&mut c, &VOID);
    // Length 4, fall through to PC = 4.
    assert_eq!(c.pc, 4);
}

// ----- pointer ops -----

#[test]
fn setp_writes_pointer_and_marks_override() {
    let mut c = cell_with(&[op(Opcode::Setp), 2, 4, 0, 0, 0, 0, 0]); // size 8
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.pointers[Direction::Yp.index()], 4);
    assert!(c.pointer_override[Direction::Yp.index()]);
    assert_eq!(c.pc, 3);
}

#[test]
fn setp_value_is_clamped_modulo_memory_size() {
    let mut c = cell_with(&[op(Opcode::Setp), 0, 99, 0, 0]); // size 5
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.pointers[Direction::Xp.index()], 99 % 5);
}

#[test]
fn getp_reads_pointer_into_memory() {
    let mut c = cell_with(&[op(Opcode::Getp), 1, 3, 0]);
    c.pointers[Direction::Xn.index()] = 42;
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.memory[3], 42);
}

#[test]
fn setpv_uses_runtime_value_from_memory() {
    // memory layout: [Setpv, 0, 4, 0, 17, 0]
    //   PC=0 → opcode=Setpv, arg1=mem[1]=0 (Xp), arg2=mem[2]=4 (address).
    //   pointers[Xp] = mem[4] mod size = 17 mod 6.
    let mut c = cell_with(&[op(Opcode::Setpv), 0, 4, 0, 17, 0]); // size 6
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.pointers[Direction::Xp.index()], 17 % 6);
    assert!(c.pointer_override[Direction::Xp.index()]);
}

// ----- emission -----

#[test]
fn port_accumulates_active_outflow() {
    let mut c = cell_with(&[op(Opcode::Port), 1, 5, 0]);
    c.active_outflow[Direction::Xn.index()] = 3;
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.active_outflow[Direction::Xn.index()], 8);
}

#[test]
fn port_saturates_on_u32_overflow() {
    let mut c = cell_with(&[op(Opcode::Port), 0, 100, 0]);
    c.active_outflow[Direction::Xp.index()] = u32::MAX - 50;
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.active_outflow[Direction::Xp.index()], u32::MAX);
}

// ----- sensors -----

#[test]
fn senergy_reads_neighbor_energy_into_memory() {
    let mut c = cell_with(&[op(Opcode::Senergy), 2, 3, 0]);
    let mut neighbors = VOID;
    neighbors[Direction::Yp.index()] = 99;
    execute_instruction(&mut c, &neighbors);
    assert_eq!(c.memory[3], 99);
}

#[test]
fn senergy_returns_zero_for_void_neighbor() {
    let mut c = cell_with(&[op(Opcode::Senergy), 0, 3, 0]);
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.memory[3], 0);
}

// ----- indirect addressing -----

#[test]
fn ldi_loads_indirect_via_runtime_address() {
    // mem[3] = 5. We want mem[2] = mem[mem[3]] = mem[5] = 7.
    let mut c = cell_with(&[op(Opcode::Ldi), 2, 3, 5, 0, 7, 0]);
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.memory[2], 7);
}

#[test]
fn sti_stores_indirect_via_runtime_address() {
    // mem[3] = 5 (target). mem[4] = 99 (value).
    // sti a=3 b=4 → mem[mem[3]] = mem[4] → mem[5] = 99.
    let mut c = cell_with(&[op(Opcode::Sti), 3, 4, 5, 99, 0, 0]);
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.memory[5], 99);
}

// ----- UI fields -----

#[test]
fn sid_writes_origin_tag_into_memory() {
    let mut c = cell_with(&[op(Opcode::Sid), 1, 0]);
    c.origin_tag = 0xCAFE_BABE;
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.memory[1], 0xCAFE_BABE);
}

#[test]
fn paint_writes_appearance() {
    let mut c = cell_with(&[op(Opcode::Paint), 0xFF00_FF00, 0]);
    execute_instruction(&mut c, &VOID);
    assert_eq!(c.appearance, 0xFF00_FF00);
}

// ----- introspection invariant -----

#[test]
fn senergy_can_only_read_via_neighbor_energies() {
    // The function signature itself enforces the invariant: there is
    // no way for `execute_instruction` to look at another cell. This
    // test simply documents that with a sensor read against a custom
    // neighbor table that doesn't match any real cell.
    let mut c = cell_with(&[op(Opcode::Senergy), 4, 1, 0]);
    let mut neighbors = VOID;
    neighbors[Direction::Zp.index()] = 12345;
    execute_instruction(&mut c, &neighbors);
    // 4 mod 6 = 4 = Zp.index().
    assert_eq!(c.memory[1], 12345);
}

// ----- direction modulo (d mod DIRS) for opcodes with a `d` operand -----

#[test]
fn direction_operand_wraps_modulo_six() {
    // d=8 → 8 mod 6 = 2 = Yp.
    let mut c = cell_with(&[op(Opcode::Setp), 8, 3, 0, 0, 0]);
    execute_instruction(&mut c, &VOID);
    assert!(c.pointer_override[Direction::Yp.index()]);
    assert!(!c.pointer_override[Direction::Xp.index()]);
}
