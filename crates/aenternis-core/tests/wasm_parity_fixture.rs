//! Native side of the wasm ↔ native physics-parity gate.
//!
//! `tests/fixtures/parity-world.json` (repo root) holds a golden
//! snapshot: a fully-parameterized world (gravity + pressure +
//! mutation + macro genesis + program overlay) advanced a fixed number
//! of ticks. This test asserts the **native** core reproduces it
//! byte-for-byte; `tests/web/wasm-native-parity.test.ts` asserts the
//! **compiled wasm bundle** reproduces the same file through the
//! `World` boundary (`newWithProgram` + setters). Both pinning to one
//! committed artifact makes any cross-backend divergence — codegen,
//! float semantics, RNG, genesis — a red `./check`, not a silent
//! physics fork.
//!
//! Regenerate after an *intentional* physics change:
//!
//! ```text
//! UPDATE_PARITY_FIXTURE=1 cargo test -p aenternis-core --test wasm_parity_fixture
//! ```
//!
//! then re-run `./check` so the wasm side confirms the new values.

use aenternis_core::{snap_gamma, snapshot, tick, Base, GenesisConfig, SparseWorld};
use serde_json::{json, Value};

/// Fixture path, anchored to the crate dir so `cargo test` works from
/// any cwd.
fn fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/parity-world.json")
}

/// Build and run the fixture world. Every value here must match the
/// `config` block of the fixture (asserted below, so the two can't
/// drift apart silently).
fn run_reference() -> (Value, SparseWorld) {
    let config = json!({
        "seed": 1234,
        "energy": 400,
        "coeff": 0.15,
        "k": 1,
        "moveThreshold": 1.0,
        "gravity": 1.0,
        "gravityAlpha": 0.05,
        "gravityRadius": 2,
        "pressure": 0.2,
        "pressureGamma": 2.0,
        "pressureEref": 50000.0,
        "mutationStrength": 1.0,
        "mutationHalfDensity": 40000.0,
        "genesisWindow": 256,
        "genesisFertility": 1.0,
        "program": [11, 22, 33],
        "ticks": 8,
    });

    let program: Vec<u32> = config["program"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| u32::try_from(v.as_u64().unwrap()).unwrap())
        .collect();
    let genesis = GenesisConfig {
        window: u32::try_from(config["genesisWindow"].as_u64().unwrap()).unwrap(),
        fertility: config["genesisFertility"].as_f64().unwrap(),
    };
    let mut world = SparseWorld::big_bang_with_config(
        config["seed"].as_u64().unwrap(),
        u32::try_from(config["energy"].as_u64().unwrap()).unwrap(),
        Base::Macros,
        &program,
        &genesis,
    );
    world.move_threshold = 1.0;
    world.gravity = config["gravity"].as_f64().unwrap();
    world.gravity_alpha = config["gravityAlpha"].as_f64().unwrap();
    world.gravity_radius = i32::try_from(config["gravityRadius"].as_i64().unwrap()).unwrap();
    world.pressure = config["pressure"].as_f64().unwrap();
    world.pressure_gamma = snap_gamma(config["pressureGamma"].as_f64().unwrap());
    world.pressure_eref = config["pressureEref"].as_f64().unwrap();
    world.mutation_strength = config["mutationStrength"].as_f64().unwrap();
    world.mutation_half_density = config["mutationHalfDensity"].as_f64().unwrap();

    for _ in 0..config["ticks"].as_u64().unwrap() {
        tick::step(
            &mut world,
            config["coeff"].as_f64().unwrap(),
            u32::try_from(config["k"].as_u64().unwrap()).unwrap(),
        );
    }
    (config, world)
}

#[test]
fn native_core_matches_the_committed_parity_fixture() {
    let (config, world) = run_reference();

    let mut snap = Vec::new();
    snapshot::snapshot_into(&world, &mut snap);
    let actual = json!({
        "//": "Golden wasm↔native parity fixture. Regenerate ONLY after an \
               intentional physics change: UPDATE_PARITY_FIXTURE=1 cargo test \
               -p aenternis-core --test wasm_parity_fixture. Pinned natively by \
               that test and against the compiled wasm bundle by \
               tests/web/wasm-native-parity.test.ts.",
        "config": config,
        "expected": {
            "cellCount": world.len(),
            "totalEnergy": world.total_energy(),
            "snapshot": snap,
        },
    });

    let path = fixture_path();
    if std::env::var("UPDATE_PARITY_FIXTURE").is_ok() {
        let pretty = serde_json::to_string_pretty(&actual).unwrap();
        std::fs::write(&path, pretty + "\n").unwrap();
        println!("parity fixture regenerated at {}", path.display());
        return;
    }

    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "fixture missing at {} ({e}); regenerate with UPDATE_PARITY_FIXTURE=1",
                path.display()
            )
        }))
        .unwrap();

    assert_eq!(
        fixture["config"], actual["config"],
        "fixture config drifted from the reference runner — regenerate deliberately"
    );
    assert_eq!(
        fixture["expected"], actual["expected"],
        "native core no longer reproduces the golden world — if the physics \
         change is intentional, regenerate with UPDATE_PARITY_FIXTURE=1 and \
         re-run ./check so the wasm side re-verifies"
    );
}
