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
//! - [`Opcode`] — enum over the 31 currently defined opcodes
//! - [`Opcode::decode`] — slot → opcode, total via the `byte % COUNT`
//!   fold (every byte decodes to a real opcode; Z80-density)
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

use crate::world::arena::Arena;
use crate::{Cell, Direction};

/// One of the 31 defined Aenternis opcodes.
///
/// The discriminants match the slot encoding (low byte == opcode), so
/// `opcode as u8` is the canonical wire representation. Discriminants are
/// **contiguous `0..COUNT-1` and append-only** — `decode` relies on this
/// to fold any byte onto a variant via `ALL[byte % COUNT]`, and the
/// append-only rule keeps existing programs stable across additions
/// (see `docs/vm.md`). New opcodes extend the enum, `decode`'s
/// backing `ALL` array, and the `length` match together.
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
    /// `and a b` — `mem[a] &= mem[b]`. 3 slots.
    And = 0x14,
    /// `or a b` — `mem[a] |= mem[b]`. 3 slots.
    Or = 0x15,
    /// `xor a b` — `mem[a] ^= mem[b]`. 3 slots.
    Xor = 0x16,
    /// `not a` — `mem[a] = !mem[a]` (bitwise complement). 2 slots.
    Not = 0x17,
    /// `shl a b` — `mem[a] <<= (mem[b] mod 32)`. 3 slots.
    Shl = 0x18,
    /// `shr a b` — `mem[a] >>= (mem[b] mod 32)` (logical). 3 slots.
    Shr = 0x19,
    /// `mul a b` — `mem[a] = (mem[a] * mem[b]) mod 2^32`. 3 slots.
    Mul = 0x1A,
    /// `div a b` — `mem[a] = mem[b]==0 ? 0 : mem[a] / mem[b]` (unsigned). 3 slots.
    Div = 0x1B,
    /// `mod a b` — `mem[a] = mem[b]==0 ? 0 : mem[a] % mem[b]` (unsigned). 3 slots.
    Mod = 0x1C,
    /// `jp a t` — if `(mem[a] as i32) > 0` then `PC = t`. 3 slots.
    Jp = 0x1D,
    /// `jn a t` — if `(mem[a] as i32) < 0` then `PC = t`. 3 slots.
    Jn = 0x1E,
}

impl Opcode {
    /// Number of opcode variants (currently `31`).
    pub const COUNT: u8 = 31;

    /// Highest valid opcode value (currently `0x1E`).
    pub const MAX: u8 = 0x1E;

    /// Decode the lowest byte of a slot into an opcode.
    ///
    /// **Total** — every one of the 256 byte values maps to a valid
    /// opcode via the fold `byte % COUNT`. Opcodes occupy a contiguous
    /// `0..COUNT-1` range, so the modulo always lands on a real variant
    /// (`Self::ALL[byte % COUNT]`). This is the Z80-density mechanism:
    /// random noise always decodes to *something* executable rather than
    /// mostly `nop`, which is what lets meaningful programs emerge from a
    /// big-bang of noise.
    ///
    /// Bytes `< COUNT` are unchanged by the fold (`b % COUNT == b`), so
    /// every assembled instruction keeps its meaning even as new opcodes
    /// are appended later — provided opcodes stay contiguous and
    /// append-only. See `docs/vm.md`.
    #[must_use]
    pub const fn decode(slot: u32) -> Self {
        Self::ALL[(slot as u8 % Self::COUNT) as usize]
    }

    /// Instruction width in slots. The executor advances `PC` by this
    /// amount after executing the instruction (modulo memory size).
    #[must_use]
    pub const fn length(self) -> u32 {
        match self {
            Self::Nop => 1,
            Self::Inc | Self::Dec | Self::Jmp | Self::Sid | Self::Paint | Self::Not => 2,
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
            | Self::And
            | Self::Or
            | Self::Xor
            | Self::Shl
            | Self::Shr
            | Self::Mul
            | Self::Div
            | Self::Mod
            | Self::Jp
            | Self::Jn => 3,
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
        Self::And,
        Self::Or,
        Self::Xor,
        Self::Not,
        Self::Shl,
        Self::Shr,
        Self::Mul,
        Self::Div,
        Self::Mod,
        Self::Jp,
        Self::Jn,
    ];

    /// Number of operand slots this opcode consumes after its own slot
    /// (`length - 1`). Each operand is exactly one slot, so an `n`-arg
    /// instruction is `n + 1` slots wide.
    #[must_use]
    pub const fn arg_count(self) -> u32 {
        self.length() - 1
    }

    /// Canonical lowercase mnemonic, matching the `OPCODES` map in
    /// `src/asm.ts` (which mirrors this — `asm.ts` is a UI helper, this
    /// is the source of truth). Used by the macro expander
    /// ([`crate::macros`]) to parse assembler-syntax macro bodies.
    #[must_use]
    pub const fn mnemonic(self) -> &'static str {
        match self {
            Self::Nop => "nop",
            Self::Set => "set",
            Self::Copy => "copy",
            Self::Add => "add",
            Self::Sub => "sub",
            Self::Inc => "inc",
            Self::Dec => "dec",
            Self::Jmp => "jmp",
            Self::Jz => "jz",
            Self::Setp => "setp",
            Self::Getp => "getp",
            Self::Port => "port",
            Self::Senergy => "senergy",
            Self::Jne => "jne",
            Self::Je => "je",
            Self::Ldi => "ldi",
            Self::Sti => "sti",
            Self::Setpv => "setpv",
            Self::Sid => "sid",
            Self::Paint => "paint",
            Self::And => "and",
            Self::Or => "or",
            Self::Xor => "xor",
            Self::Not => "not",
            Self::Shl => "shl",
            Self::Shr => "shr",
            Self::Mul => "mul",
            Self::Div => "div",
            Self::Mod => "mod",
            Self::Jp => "jp",
            Self::Jn => "jn",
        }
    }

    /// Resolve a lowercase mnemonic to its opcode. Inverse of
    /// [`Opcode::mnemonic`]; `None` for an unknown mnemonic.
    #[must_use]
    pub fn from_mnemonic(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.mnemonic() == name)
    }
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
/// **Empty cells** (`memory.len() == 0`) are a no-op — there is no
/// program to run. Callers don't need to special-case them.
///
/// **Every byte is executable.** [`Opcode::decode`] is total — a low
/// byte `>= COUNT` folds onto a real opcode (`byte % COUNT`) rather than
/// acting as `nop`. Random noise in memory therefore always runs *some*
/// instruction; the VM still never crashes, because every opcode is
/// memory-safe under modular addressing and wrapping arithmetic.
///
/// All addresses are taken modulo `memory.len()` (modular addressing,
/// never out of bounds). All arithmetic is wrapping. The `port` opcode
/// uses `wrapping_add` to accumulate into `active_outflow` — the wrap
/// is intentional, not undefined behavior; rogue programs that drive a
/// face's outflow past `u32::MAX` just wrap around.
#[allow(clippy::too_many_lines)] // 31 opcodes per match; splitting hurts more than it helps
pub fn execute_instruction(
    cell: &mut Cell,
    arena: &mut Arena,
    neighbor_energies: &[u32; Direction::COUNT],
) {
    let mem_size = cell.memory_len();
    if mem_size == 0 {
        return;
    }
    let pc_u = cell.pc as usize;
    let opcode_slot = cell.memory_slot(arena, pc_u % mem_size);

    // `decode` is total — every byte folds onto a real opcode, so there is
    // no "unknown → nop" fallback. A byte `>= COUNT` executes as its folded
    // opcode `byte % COUNT`.
    let op = Opcode::decode(opcode_slot);

    let length = op.length() as usize;

    // Read up to three operand slots upfront. Storing them in
    // locals (rather than re-reading mid-instruction) preserves
    // the introspection invariant: even if an opcode writes to a
    // slot mid-instruction, subsequent operand reads see the
    // *pre-write* state. Matches the JS prototype's behaviour
    // exactly.
    let arg1 = if length >= 2 {
        cell.memory_slot(arena, (pc_u + 1) % mem_size)
    } else {
        0
    };
    let arg2 = if length >= 3 {
        cell.memory_slot(arena, (pc_u + 2) % mem_size)
    } else {
        0
    };
    let arg3 = if length >= 4 {
        cell.memory_slot(arena, (pc_u + 3) % mem_size)
    } else {
        0
    };

    // `Some(addr)` overrides the default PC-advance when set by a jump.
    let mut jump_to: Option<usize> = None;

    match op {
        Opcode::Nop => {}

        Opcode::Set => {
            let dst = (arg1 as usize) % mem_size;
            cell.set_memory_slot(arena, dst, arg2);
        }
        Opcode::Copy => {
            let src = (arg2 as usize) % mem_size;
            let dst = (arg1 as usize) % mem_size;
            let v = cell.memory_slot(arena, src);
            cell.set_memory_slot(arena, dst, v);
        }
        Opcode::Add => {
            let src = (arg2 as usize) % mem_size;
            let dst = (arg1 as usize) % mem_size;
            let lhs = cell.memory_slot(arena, dst);
            let rhs = cell.memory_slot(arena, src);
            cell.set_memory_slot(arena, dst, lhs.wrapping_add(rhs));
        }
        Opcode::Sub => {
            let src = (arg2 as usize) % mem_size;
            let dst = (arg1 as usize) % mem_size;
            let lhs = cell.memory_slot(arena, dst);
            let rhs = cell.memory_slot(arena, src);
            cell.set_memory_slot(arena, dst, lhs.wrapping_sub(rhs));
        }
        Opcode::Inc => {
            let dst = (arg1 as usize) % mem_size;
            let v = cell.memory_slot(arena, dst);
            cell.set_memory_slot(arena, dst, v.wrapping_add(1));
        }
        Opcode::Dec => {
            let dst = (arg1 as usize) % mem_size;
            let v = cell.memory_slot(arena, dst);
            cell.set_memory_slot(arena, dst, v.wrapping_sub(1));
        }

        Opcode::Jmp => {
            jump_to = Some((arg1 as usize) % mem_size);
        }
        Opcode::Jz => {
            let probe = (arg1 as usize) % mem_size;
            if cell.memory_slot(arena, probe) == 0 {
                jump_to = Some((arg2 as usize) % mem_size);
            }
        }
        Opcode::Jne => {
            let probe = (arg1 as usize) % mem_size;
            if cell.memory_slot(arena, probe) != 0 {
                jump_to = Some((arg2 as usize) % mem_size);
            }
        }
        Opcode::Je => {
            let a = (arg1 as usize) % mem_size;
            let b = (arg2 as usize) % mem_size;
            if cell.memory_slot(arena, a) == cell.memory_slot(arena, b) {
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
            let v = cell.pointers[dir];
            cell.set_memory_slot(arena, dst, v);
        }
        Opcode::Setpv => {
            let dir = (arg1 as usize) % Direction::COUNT;
            let src = (arg2 as usize) % mem_size;
            cell.pointers[dir] = cell.memory_slot(arena, src) % (mem_size as u32);
            cell.pointer_override[dir] = true;
        }

        Opcode::Port => {
            let dir = (arg1 as usize) % Direction::COUNT;
            cell.active_outflow[dir] = cell.active_outflow[dir].wrapping_add(arg2);
        }
        Opcode::Senergy => {
            let dir = (arg1 as usize) % Direction::COUNT;
            let dst = (arg2 as usize) % mem_size;
            let v = neighbor_energies[dir];
            cell.set_memory_slot(arena, dst, v);
        }

        Opcode::Ldi => {
            let b_addr = (arg2 as usize) % mem_size;
            let runtime = cell.memory_slot(arena, b_addr) as usize;
            let src = runtime % mem_size;
            let dst = (arg1 as usize) % mem_size;
            let v = cell.memory_slot(arena, src);
            cell.set_memory_slot(arena, dst, v);
        }
        Opcode::Sti => {
            let a_addr = (arg1 as usize) % mem_size;
            let b_addr = (arg2 as usize) % mem_size;
            let runtime = cell.memory_slot(arena, a_addr) as usize;
            let dst = runtime % mem_size;
            let v = cell.memory_slot(arena, b_addr);
            cell.set_memory_slot(arena, dst, v);
        }

        Opcode::Sid => {
            let dst = (arg1 as usize) % mem_size;
            let tag = cell.origin_tag;
            cell.set_memory_slot(arena, dst, tag);
        }
        Opcode::Paint => {
            cell.appearance = arg1;
        }

        Opcode::And => {
            let src = (arg2 as usize) % mem_size;
            let dst = (arg1 as usize) % mem_size;
            let lhs = cell.memory_slot(arena, dst);
            let rhs = cell.memory_slot(arena, src);
            cell.set_memory_slot(arena, dst, lhs & rhs);
        }
        Opcode::Or => {
            let src = (arg2 as usize) % mem_size;
            let dst = (arg1 as usize) % mem_size;
            let lhs = cell.memory_slot(arena, dst);
            let rhs = cell.memory_slot(arena, src);
            cell.set_memory_slot(arena, dst, lhs | rhs);
        }
        Opcode::Xor => {
            let src = (arg2 as usize) % mem_size;
            let dst = (arg1 as usize) % mem_size;
            let lhs = cell.memory_slot(arena, dst);
            let rhs = cell.memory_slot(arena, src);
            cell.set_memory_slot(arena, dst, lhs ^ rhs);
        }
        Opcode::Not => {
            let dst = (arg1 as usize) % mem_size;
            let v = cell.memory_slot(arena, dst);
            cell.set_memory_slot(arena, dst, !v);
        }
        Opcode::Shl => {
            // `wrapping_shl` masks the shift amount by 32 (i.e. `mem[b] % 32`),
            // matching the `& 31` in the spec and never panicking on `>= 32`.
            let src = (arg2 as usize) % mem_size;
            let dst = (arg1 as usize) % mem_size;
            let lhs = cell.memory_slot(arena, dst);
            let rhs = cell.memory_slot(arena, src);
            cell.set_memory_slot(arena, dst, lhs.wrapping_shl(rhs));
        }
        Opcode::Shr => {
            // Logical (unsigned) shift right; `wrapping_shr` masks by 32.
            let src = (arg2 as usize) % mem_size;
            let dst = (arg1 as usize) % mem_size;
            let lhs = cell.memory_slot(arena, dst);
            let rhs = cell.memory_slot(arena, src);
            cell.set_memory_slot(arena, dst, lhs.wrapping_shr(rhs));
        }
        Opcode::Mul => {
            let src = (arg2 as usize) % mem_size;
            let dst = (arg1 as usize) % mem_size;
            let lhs = cell.memory_slot(arena, dst);
            let rhs = cell.memory_slot(arena, src);
            cell.set_memory_slot(arena, dst, lhs.wrapping_mul(rhs));
        }
        Opcode::Div => {
            // Division by zero yields 0 — the VM has no trap mechanism, so a
            // defined deterministic result is required. See `docs/vm.md`.
            let src = (arg2 as usize) % mem_size;
            let dst = (arg1 as usize) % mem_size;
            let lhs = cell.memory_slot(arena, dst);
            let rhs = cell.memory_slot(arena, src);
            cell.set_memory_slot(arena, dst, lhs.checked_div(rhs).unwrap_or(0));
        }
        Opcode::Mod => {
            // Modulo by zero yields 0 (same rationale as `div`).
            let src = (arg2 as usize) % mem_size;
            let dst = (arg1 as usize) % mem_size;
            let lhs = cell.memory_slot(arena, dst);
            let rhs = cell.memory_slot(arena, src);
            cell.set_memory_slot(arena, dst, lhs.checked_rem(rhs).unwrap_or(0));
        }
        Opcode::Jp => {
            // Signed `> 0` without an `as i32` cast (MSRV 1.85 predates
            // `u32::cast_signed`): positive two's-complement means the sign
            // bit is clear and the value is non-zero.
            let probe = (arg1 as usize) % mem_size;
            let v = cell.memory_slot(arena, probe);
            if v != 0 && v < 0x8000_0000 {
                jump_to = Some((arg2 as usize) % mem_size);
            }
        }
        Opcode::Jn => {
            // Signed `< 0`: the two's-complement sign bit is set.
            let probe = (arg1 as usize) % mem_size;
            if cell.memory_slot(arena, probe) >= 0x8000_0000 {
                jump_to = Some((arg2 as usize) % mem_size);
            }
        }
    }

    cell.pc = jump_to.map_or_else(
        || ((pc_u + length) % mem_size) as u32,
        |target| target as u32,
    );
}
