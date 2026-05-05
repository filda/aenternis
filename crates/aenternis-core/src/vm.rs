//! Virtual machine for Aenternis cells.
//!
//! Every cell is a latent micro-CPU. When energy is non-zero it executes
//! `floor(energy / K)` instructions per tick, drawing each one from the
//! slot at its program counter. Memory and program share the same
//! 32-bit slot array (Von Neumann); the opcode is the lowest byte of a
//! slot, the upper bits become embedded data depending on the
//! instruction.
//!
//! This module provides:
//!
//! - [`Opcode`] — enum over the 20 currently defined opcodes
//! - [`Opcode::decode`] — slot → optional opcode (unknown = `None`,
//!   which the executor treats as `nop`)
//! - [`Opcode::length`] — instruction width in slots, drives PC advance
//! - [`execute_instruction`] — single-step interpreter for one opcode
//!
//! The orchestrating `cpu_phase` (per-cell tick budget) lives in
//! [`crate::tick`] because it consults the world for neighbor energies
//! before stepping each cell.
//!
//! See `docs/vm.md` for the full instruction-set specification and the
//! introspection invariant ("a cell cannot read another cell's
//! interior, only its emissions").

use crate::{Cell, Direction};

/// One of the 20 defined Aenternis opcodes.
///
/// The discriminants match the slot encoding (low byte == opcode), so
/// `opcode as u8` is the canonical wire representation. New opcodes are
/// added by extending the enum *and* the `decode` / `length` matches —
/// `clippy::needless_match` keeps them in sync at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Opcode {
    /// `nop` — does nothing. PC advances by 1 slot.
    Nop = 0x00,
    /// `set a v` — `mem[a] = v`. 3 slots.
    Set = 0x01,
    /// `copy a b` — `mem[a] = mem[b]`. 3 slots.
    Copy = 0x02,
    /// `add a b` — `mem[a] = (mem[a] + mem[b]) mod 2^32`. 3 slots.
    Add = 0x03,
    /// `sub a b` — `mem[a] = (mem[a] - mem[b]) mod 2^32`. 3 slots.
    Sub = 0x04,
    /// `inc a` — `mem[a] = (mem[a] + 1) mod 2^32`. 2 slots.
    Inc = 0x05,
    /// `dec a` — `mem[a] = (mem[a] - 1) mod 2^32`. 2 slots.
    Dec = 0x06,
    /// `jmp a` — `PC = a`. 2 slots.
    Jmp = 0x07,
    /// `jz a t` — if `mem[a] == 0` then `PC = t`. 3 slots.
    Jz = 0x08,
    /// `setp d v` — `pointers[d] = v`, sets the override flag. 3 slots.
    Setp = 0x09,
    /// `getp d a` — `mem[a] = pointers[d]`. 3 slots.
    Getp = 0x0A,
    /// `port d i` — `active_outflow[d] += i`. 3 slots.
    Port = 0x0B,
    /// `senergy d a` — `mem[a] = neighbor[d].energy` (0 if void). 3 slots.
    Senergy = 0x0C,
    /// `jne a t` — if `mem[a] != 0` then `PC = t`. 3 slots.
    Jne = 0x0D,
    /// `je a b t` — if `mem[a] == mem[b]` then `PC = t`. 4 slots.
    Je = 0x0E,
    /// `ldi a b` — `mem[a] = mem[mem[b]]` (load indirect). 3 slots.
    Ldi = 0x0F,
    /// `sti a b` — `mem[mem[a]] = mem[b]` (store indirect). 3 slots.
    Sti = 0x10,
    /// `setpv d a` — `pointers[d] = mem[a]`, sets the override flag. 3 slots.
    Setpv = 0x11,
    /// `sid a` — `mem[a] = own origin_tag`. 2 slots.
    Sid = 0x12,
    /// `paint v` — `appearance = v`. 2 slots.
    Paint = 0x13,
    /// `sinflow d a` — `mem[a] = own inflow[d]` from the last tick. 3 slots.
    Sinflow = 0x14,
    /// `sself a` — `mem[a] = own energy / memSize`. 2 slots.
    Sself = 0x15,
    /// `srate d a` — `mem[a] = own combined rate (rates[d] + active_outflow[d])`. 3 slots.
    Srate = 0x16,
}

impl Opcode {
    /// Number of opcode variants (currently `23`).
    pub const COUNT: u8 = 23;

    /// Highest valid opcode value (currently `0x16`).
    pub const MAX: u8 = 0x16;

    /// Decode the lowest byte of a slot into an opcode.
    ///
    /// Returns `None` for unknown bytes (`> MAX`). The executor treats
    /// unknown as `nop` (PC advance by 1), but distinguishing the two
    /// at decode time helps tooling — a disassembler may print the raw
    /// byte in hex rather than misleadingly listing it as `nop`.
    #[must_use]
    pub const fn decode(slot: u32) -> Option<Self> {
        match slot & 0xFF {
            0x00 => Some(Self::Nop),
            0x01 => Some(Self::Set),
            0x02 => Some(Self::Copy),
            0x03 => Some(Self::Add),
            0x04 => Some(Self::Sub),
            0x05 => Some(Self::Inc),
            0x06 => Some(Self::Dec),
            0x07 => Some(Self::Jmp),
            0x08 => Some(Self::Jz),
            0x09 => Some(Self::Setp),
            0x0A => Some(Self::Getp),
            0x0B => Some(Self::Port),
            0x0C => Some(Self::Senergy),
            0x0D => Some(Self::Jne),
            0x0E => Some(Self::Je),
            0x0F => Some(Self::Ldi),
            0x10 => Some(Self::Sti),
            0x11 => Some(Self::Setpv),
            0x12 => Some(Self::Sid),
            0x13 => Some(Self::Paint),
            0x14 => Some(Self::Sinflow),
            0x15 => Some(Self::Sself),
            0x16 => Some(Self::Srate),
            _ => None,
        }
    }

    /// Instruction width in slots. The executor advances `PC` by this
    /// amount after executing the instruction (modulo memory size).
    #[must_use]
    pub const fn length(self) -> u32 {
        match self {
            Self::Nop => 1,
            Self::Inc | Self::Dec | Self::Jmp | Self::Sid | Self::Paint | Self::Sself => 2,
            Self::Set
            | Self::Copy
            | Self::Add
            | Self::Sub
            | Self::Jz
            | Self::Setp
            | Self::Getp
            | Self::Port
            | Self::Senergy
            | Self::Jne
            | Self::Ldi
            | Self::Sti
            | Self::Setpv
            | Self::Sinflow
            | Self::Srate => 3,
            Self::Je => 4,
        }
    }

    /// All defined opcodes in canonical (numeric) order.
    pub const ALL: [Self; Self::COUNT as usize] = [
        Self::Nop,
        Self::Set,
        Self::Copy,
        Self::Add,
        Self::Sub,
        Self::Inc,
        Self::Dec,
        Self::Jmp,
        Self::Jz,
        Self::Setp,
        Self::Getp,
        Self::Port,
        Self::Senergy,
        Self::Jne,
        Self::Je,
        Self::Ldi,
        Self::Sti,
        Self::Setpv,
        Self::Sid,
        Self::Paint,
        Self::Sinflow,
        Self::Sself,
        Self::Srate,
    ];
}

/// Decode and execute the instruction at `cell.pc`, advancing the
/// program counter by the instruction's length (or to a jump target
/// when the opcode dictates).
///
/// `neighbor_energies` is a six-direction read-only snapshot used by
/// `senergy`. The caller is expected to assemble it once per cell at
/// the start of the CPU phase — the introspection invariant says a
/// cell can only see its neighbors' emissions on the shared face, so a
/// snapshot is the right shape: a cell cannot observe live changes in
/// a neighbor mid-instruction.
///
/// `legacy_port_wrap` toggles `port`'s accumulation semantics:
///
/// - `false` (default) — `saturating_add`, the safer Rust-native path.
///   When noise memory triggers many `port` ops in one tick, every
///   targeted direction saturates at `u32::MAX` and the proportional
///   clamp later splits outflow evenly across them.
/// - `true` — `wrapping_add`, matching JS prototype 9-B's
///   `(activeOutflow + arg1) >>> 0`. Individual directions hold
///   residual `mod 2^32` values that vary direction-by-direction, so
///   the dominant direction takes most of the outflow after clamp.
///   This is the source of 9-B's visible asymmetric expansion that the
///   saturating path doesn't reproduce.
///
/// **Empty cells** (`memory.len() == 0`) are a no-op — there is no
/// program to run. Callers don't need to special-case them.
///
/// **Unknown opcodes** (low byte > [`Opcode::MAX`]) act as `nop` —
/// PC advances by 1 slot. This is a defensive default: random noise
/// in memory must never crash the VM.
///
/// All addresses are taken modulo `memory.len()` (modular addressing,
/// never out of bounds). All arithmetic is wrapping.
#[allow(clippy::too_many_lines)] // 20 opcodes per match; splitting hurts more than it helps
pub fn execute_instruction(
    cell: &mut Cell,
    neighbor_energies: &[u32; Direction::COUNT],
    legacy_port_wrap: bool,
    legacy_opcode_set: bool,
) {
    let mem_size = cell.memory.len();
    if mem_size == 0 {
        return;
    }
    let pc_u = cell.pc as usize;
    let opcode_slot = cell.memory[pc_u % mem_size];

    let Some(op) = Opcode::decode(opcode_slot) else {
        // Unknown opcode → nop, advance by 1.
        cell.pc = ((pc_u + 1) % mem_size) as u32;
        return;
    };

    // Legacy opcode set: JS prototype 9-B stops at 0x13 (`paint`).
    // Treating 0x14..=0x16 as unknown here keeps PC-walk identical to
    // JS when noise memory encodes one of those bytes.
    if legacy_opcode_set && (opcode_slot & 0xFF) > 0x13 {
        cell.pc = ((pc_u + 1) % mem_size) as u32;
        return;
    }

    let length = op.length() as usize;

    // Read up to three operand slots upfront. After this point we no
    // longer hold an immutable borrow of `cell.memory`, so the match
    // arms are free to mutate cells.
    let arg1 = if length >= 2 {
        cell.memory[(pc_u + 1) % mem_size]
    } else {
        0
    };
    let arg2 = if length >= 3 {
        cell.memory[(pc_u + 2) % mem_size]
    } else {
        0
    };
    let arg3 = if length >= 4 {
        cell.memory[(pc_u + 3) % mem_size]
    } else {
        0
    };

    // `Some(addr)` overrides the default PC-advance when set by a jump.
    let mut jump_to: Option<usize> = None;

    match op {
        Opcode::Nop => {}

        Opcode::Set => {
            let dst = (arg1 as usize) % mem_size;
            cell.memory[dst] = arg2;
        }
        Opcode::Copy => {
            let src = (arg2 as usize) % mem_size;
            let dst = (arg1 as usize) % mem_size;
            cell.memory[dst] = cell.memory[src];
        }
        Opcode::Add => {
            let src = (arg2 as usize) % mem_size;
            let dst = (arg1 as usize) % mem_size;
            cell.memory[dst] = cell.memory[dst].wrapping_add(cell.memory[src]);
        }
        Opcode::Sub => {
            let src = (arg2 as usize) % mem_size;
            let dst = (arg1 as usize) % mem_size;
            cell.memory[dst] = cell.memory[dst].wrapping_sub(cell.memory[src]);
        }
        Opcode::Inc => {
            let dst = (arg1 as usize) % mem_size;
            cell.memory[dst] = cell.memory[dst].wrapping_add(1);
        }
        Opcode::Dec => {
            let dst = (arg1 as usize) % mem_size;
            cell.memory[dst] = cell.memory[dst].wrapping_sub(1);
        }

        Opcode::Jmp => {
            jump_to = Some((arg1 as usize) % mem_size);
        }
        Opcode::Jz => {
            let probe = (arg1 as usize) % mem_size;
            if cell.memory[probe] == 0 {
                jump_to = Some((arg2 as usize) % mem_size);
            }
        }
        Opcode::Jne => {
            let probe = (arg1 as usize) % mem_size;
            if cell.memory[probe] != 0 {
                jump_to = Some((arg2 as usize) % mem_size);
            }
        }
        Opcode::Je => {
            let a = (arg1 as usize) % mem_size;
            let b = (arg2 as usize) % mem_size;
            if cell.memory[a] == cell.memory[b] {
                jump_to = Some((arg3 as usize) % mem_size);
            }
        }

        Opcode::Setp => {
            let dir = (arg1 as usize) % Direction::COUNT;
            cell.pointers[dir] = arg2 % (mem_size as u32);
            cell.pointer_override[dir] = true;
        }
        Opcode::Getp => {
            let dir = (arg1 as usize) % Direction::COUNT;
            let dst = (arg2 as usize) % mem_size;
            cell.memory[dst] = cell.pointers[dir];
        }
        Opcode::Setpv => {
            let dir = (arg1 as usize) % Direction::COUNT;
            let src = (arg2 as usize) % mem_size;
            cell.pointers[dir] = cell.memory[src] % (mem_size as u32);
            cell.pointer_override[dir] = true;
        }

        Opcode::Port => {
            let dir = (arg1 as usize) % Direction::COUNT;
            cell.active_outflow[dir] = if legacy_port_wrap {
                cell.active_outflow[dir].wrapping_add(arg2)
            } else {
                cell.active_outflow[dir].saturating_add(arg2)
            };
        }
        Opcode::Senergy => {
            let dir = (arg1 as usize) % Direction::COUNT;
            let dst = (arg2 as usize) % mem_size;
            cell.memory[dst] = neighbor_energies[dir];
        }

        Opcode::Ldi => {
            let b_addr = (arg2 as usize) % mem_size;
            let runtime = cell.memory[b_addr] as usize;
            let src = runtime % mem_size;
            let dst = (arg1 as usize) % mem_size;
            cell.memory[dst] = cell.memory[src];
        }
        Opcode::Sti => {
            let a_addr = (arg1 as usize) % mem_size;
            let b_addr = (arg2 as usize) % mem_size;
            let runtime = cell.memory[a_addr] as usize;
            let dst = runtime % mem_size;
            cell.memory[dst] = cell.memory[b_addr];
        }

        Opcode::Sid => {
            let dst = (arg1 as usize) % mem_size;
            cell.memory[dst] = cell.origin_tag;
        }
        Opcode::Paint => {
            cell.appearance = arg1;
        }

        Opcode::Sinflow => {
            let dir = (arg1 as usize) % Direction::COUNT;
            let dst = (arg2 as usize) % mem_size;
            cell.memory[dst] = cell.inflow[dir];
        }
        Opcode::Sself => {
            // Own energy = memory length (the cardinal cell invariant).
            // u32 cap because `wasm32` memory.len() never reaches 2^32.
            let dst = (arg1 as usize) % mem_size;
            cell.memory[dst] = u32::try_from(mem_size).unwrap_or(u32::MAX);
        }
        Opcode::Srate => {
            // Combined rate = natural rate + active outflow accumulated
            // earlier in this CPU phase via `port`.
            let dir = (arg1 as usize) % Direction::COUNT;
            let dst = (arg2 as usize) % mem_size;
            let combined = cell.rates[dir].saturating_add(cell.active_outflow[dir]);
            cell.memory[dst] = combined;
        }
    }

    cell.pc = jump_to.map_or_else(
        || ((pc_u + length) % mem_size) as u32,
        |target| target as u32,
    );
}
