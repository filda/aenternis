//! Integration tests for the [`Opcode`] enum and decoder.
//!
//! Three properties verified:
//!
//! 1. **Decoding** — every defined byte (`< COUNT`) maps to its expected
//!    opcode unchanged; every byte `>= COUNT` folds onto a real opcode via
//!    `byte % COUNT`; the upper 24 bits of a slot are ignored during decode.
//! 2. **Length** — instruction widths match the table in `docs/vm.md`.
//! 3. **Surface** — `Opcode::ALL` lists all 31 variants in numeric
//!    order, `COUNT` and `MAX` are consistent.

use aenternis_core::Opcode;

// ----- decode -----

#[test]
fn decode_returns_correct_opcode_for_every_defined_byte() {
    let pairs = [
        (0x00u8, Opcode::Nop),
        (0x01, Opcode::Set),
        (0x02, Opcode::Copy),
        (0x03, Opcode::Add),
        (0x04, Opcode::Sub),
        (0x05, Opcode::Inc),
        (0x06, Opcode::Dec),
        (0x07, Opcode::Jmp),
        (0x08, Opcode::Jz),
        (0x09, Opcode::Setp),
        (0x0A, Opcode::Getp),
        (0x0B, Opcode::Port),
        (0x0C, Opcode::Senergy),
        (0x0D, Opcode::Jne),
        (0x0E, Opcode::Je),
        (0x0F, Opcode::Ldi),
        (0x10, Opcode::Sti),
        (0x11, Opcode::Setpv),
        (0x12, Opcode::Sid),
        (0x13, Opcode::Paint),
        (0x14, Opcode::And),
        (0x15, Opcode::Or),
        (0x16, Opcode::Xor),
        (0x17, Opcode::Not),
        (0x18, Opcode::Shl),
        (0x19, Opcode::Shr),
        (0x1A, Opcode::Mul),
        (0x1B, Opcode::Div),
        (0x1C, Opcode::Mod),
        (0x1D, Opcode::Jp),
        (0x1E, Opcode::Jn),
    ];
    for (byte, expected) in pairs {
        assert_eq!(
            Opcode::decode(u32::from(byte)),
            expected,
            "decode({byte:#x}) mismatch"
        );
    }
}

#[test]
fn decode_folds_high_bytes_into_opcode_space() {
    // `decode` is total: bytes `>= COUNT` fold onto a real opcode via
    // `byte % COUNT`, which is exactly `Opcode::ALL[byte % COUNT]`.
    for byte in 0u16..=0xFF {
        let folded = Opcode::ALL[(byte % u16::from(Opcode::COUNT)) as usize];
        assert_eq!(
            Opcode::decode(u32::from(byte)),
            folded,
            "decode({byte:#x}) should fold to ALL[{byte:#x} % COUNT]"
        );
    }
}

#[test]
fn decode_bytes_below_count_are_identity() {
    // The fold leaves `b < COUNT` unchanged (`b % COUNT == b`), which is
    // the backward-compatibility guarantee for assembled programs.
    for byte in 0..Opcode::COUNT {
        assert_eq!(Opcode::decode(u32::from(byte)) as u8, byte);
    }
}

#[test]
fn decode_ignores_upper_24_bits_of_slot() {
    // The decoder reads only the low byte. A slot with arbitrary
    // upper bits but a known opcode in the low byte must decode the
    // same as the bare opcode.
    let slot = 0xDEAD_BE00u32 | 0x05; // upper data + Inc
    assert_eq!(Opcode::decode(slot), Opcode::Inc);

    let slot = 0xFFFF_FF00u32 | 0x13; // upper data + Paint
    assert_eq!(Opcode::decode(slot), Opcode::Paint);

    // High low-byte folds: 0xAB % 31 = 0x12 = Sid. Upper bits irrelevant.
    let slot = 0xCAFE_BA00u32 | 0xAB;
    assert_eq!(
        Opcode::decode(slot),
        Opcode::ALL[(0xAB % Opcode::COUNT) as usize]
    );
}

// ----- length -----

#[test]
fn lengths_match_vm_spec() {
    // Source of truth: `docs/vm.md`.
    let cases = [
        (Opcode::Nop, 1),
        (Opcode::Set, 3),
        (Opcode::Copy, 3),
        (Opcode::Add, 3),
        (Opcode::Sub, 3),
        (Opcode::Inc, 2),
        (Opcode::Dec, 2),
        (Opcode::Jmp, 2),
        (Opcode::Jz, 3),
        (Opcode::Setp, 3),
        (Opcode::Getp, 3),
        (Opcode::Port, 3),
        (Opcode::Senergy, 3),
        (Opcode::Jne, 3),
        (Opcode::Je, 4),
        (Opcode::Ldi, 3),
        (Opcode::Sti, 3),
        (Opcode::Setpv, 3),
        (Opcode::Sid, 2),
        (Opcode::Paint, 2),
        (Opcode::And, 3),
        (Opcode::Or, 3),
        (Opcode::Xor, 3),
        (Opcode::Not, 2),
        (Opcode::Shl, 3),
        (Opcode::Shr, 3),
        (Opcode::Mul, 3),
        (Opcode::Div, 3),
        (Opcode::Mod, 3),
        (Opcode::Jp, 3),
        (Opcode::Jn, 3),
    ];
    for (op, expected_len) in cases {
        assert_eq!(op.length(), expected_len, "{op:?}.length()");
    }
}

#[test]
fn every_length_is_in_one_to_four() {
    // No instruction is 0 slots wide (would loop forever) or > 4 (no
    // current instruction has more than 3 operands).
    for &op in &Opcode::ALL {
        let n = op.length();
        assert!((1..=4).contains(&n), "{op:?} has out-of-range length {n}");
    }
}

// ----- surface -----

#[test]
fn count_constant_matches_all_array_length() {
    assert_eq!(Opcode::ALL.len(), Opcode::COUNT as usize);
}

#[test]
fn max_constant_matches_highest_discriminant() {
    let highest = Opcode::ALL.iter().map(|&op| op as u8).max().unwrap();
    assert_eq!(highest, Opcode::MAX);
}

#[test]
fn all_lists_opcodes_in_numeric_order() {
    let mut prev: i32 = -1;
    for &op in &Opcode::ALL {
        let value = i32::from(op as u8);
        assert!(value > prev, "{op:?} (={value}) breaks numeric order");
        prev = value;
    }
}

#[test]
fn all_contains_each_opcode_exactly_once() {
    use std::collections::HashSet;
    let set: HashSet<Opcode> = Opcode::ALL.iter().copied().collect();
    assert_eq!(set.len(), Opcode::ALL.len());
}

#[test]
fn opcode_is_copy_and_eq() {
    let a = Opcode::Add;
    let b = a; // Copy works
    assert_eq!(a, b);
    assert_eq!(a, Opcode::Add);
    assert_ne!(a, Opcode::Sub);
}

#[test]
fn discriminant_round_trip_via_decode() {
    // For every variant: encode as u8, decode back, must match.
    for &op in &Opcode::ALL {
        let byte = op as u8;
        assert_eq!(Opcode::decode(u32::from(byte)), op);
    }
}
